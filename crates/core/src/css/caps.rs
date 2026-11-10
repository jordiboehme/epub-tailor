//! Enforce the device's CSS byte and rule caps.
//!
//! The firmware reads at most `css_max_bytes` from any single CSS file and
//! stores a bounded number of rules across the whole book. We split an
//! oversized file at rule boundaries (the device also zip-scans for `.css`
//! files the manifest misses, so the extra parts are still read) and warn -
//! but never delete - when the book as a whole exceeds a store cap, since
//! which rules are lost depends on the order the device loads the sheets in.

use std::collections::HashSet;

use crate::report::Warning;

/// Split minified, device-conformant `css` into chunks each at most `max_bytes`,
/// breaking only at rule boundaries (`}`). A single rule larger than `max_bytes`
/// is left whole in its own chunk (it cannot be split without corrupting it).
/// Returns a single chunk when no split is needed.
///
/// This relies on the input being the output of the subset filter: no comments,
/// strings, or nested braces, so every top-level `}` ends a rule.
pub(crate) fn split_css(css: &str, max_bytes: usize) -> Vec<String> {
    if max_bytes == 0 || css.len() <= max_bytes {
        return vec![css.to_string()];
    }
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();
    for rule in css.split_inclusive('}') {
        if !current.is_empty() && current.len() + rule.len() > max_bytes {
            chunks.push(std::mem::take(&mut current));
        }
        current.push_str(rule);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    if chunks.is_empty() {
        chunks.push(String::new());
    }
    chunks
}

/// The zip path for the `n`-th split part of `path` (`n >= 2`): `main.css` with
/// `n == 2` becomes `main-2.css`.
pub(crate) fn split_path(path: &str, part: usize) -> String {
    match path.rfind('.') {
        Some(dot) => format!("{}-{}{}", &path[..dot], part, &path[dot..]),
        None => format!("{path}-{part}"),
    }
}

/// Most distinct declaration blocks the device stores (`CssParser.cpp:46` at
/// firmware 2.0.0).
const MAX_UNIQUE_STYLES: usize = 256;

/// Most bytes of selector text the device stores (`CssParser.cpp:45`).
const SELECTOR_POOL_CAP: usize = 32 * 1024;

/// Book-wide warnings for the device's CSS rule store, given every stylesheet
/// it will load (minified, device-conformant CSS), counted like the device
/// stores them (`CssParser.cpp:44-46, 727-736, 1172, 1226-1240`). A sheet
/// byte-identical to an earlier one is parsed once (`Epub.cpp:257-318`). Each
/// comma member is one rule, except that a selector equal to the one stored
/// just before it merges into that entry (`insertOrMerge`) and adds neither a
/// rule nor selector bytes; a repeat with another selector in between is
/// stored again. The store holds up to `max_rules` rules, 256 distinct
/// declaration blocks and 32 KB of selector text; past any of them the device
/// stops storing new rules, so rules in later sheets are lost. The device
/// compares parsed styles rather than text, so counting distinct declaration
/// text is a close, slightly high, estimate.
pub(crate) fn store_cap_warnings(sheets: &[&str], max_rules: usize) -> Vec<Warning> {
    let mut rules = 0usize;
    let mut selector_bytes = 0usize;
    let mut bodies: HashSet<&str> = HashSet::new();
    let mut seen_sheets: HashSet<&str> = HashSet::new();
    let mut last_selector: Option<String> = None;
    for sheet in sheets {
        if !seen_sheets.insert(sheet) {
            continue;
        }
        for rule in sheet.split('}') {
            let Some((selectors, body)) = rule.split_once('{') else {
                continue;
            };
            for selector in selectors.split(',') {
                let key = selector.to_ascii_lowercase();
                if last_selector.as_deref() == Some(key.as_str()) {
                    continue;
                }
                rules += 1;
                selector_bytes += selector.len();
                last_selector = Some(key);
            }
            bodies.insert(body);
        }
    }
    let mut warnings = Vec::new();
    let mut warn = |message: String| {
        warnings.push(Warning {
            message,
            file: None,
        })
    };
    if rules > max_rules {
        warn(format!(
            "the book has {rules} CSS rules (one per selector) over the device cap of \
             {max_rules}; the device will drop the rules past the cap"
        ));
    }
    if bodies.len() > MAX_UNIQUE_STYLES {
        warn(format!(
            "the book has {} distinct CSS declaration blocks over the device cap of \
             {MAX_UNIQUE_STYLES}; the device will drop the rules past the cap",
            bodies.len()
        ));
    }
    if selector_bytes > SELECTOR_POOL_CAP {
        warn(format!(
            "the book's CSS selectors take {selector_bytes} bytes over the device cap of \
             {SELECTOR_POOL_CAP}; the device will drop the rules past the cap"
        ));
    }
    warnings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::subset::{FilteredCss, filter_css};

    #[test]
    fn no_split_when_within_cap() {
        let css = ".a{text-align:center}.b{text-align:left}";
        assert_eq!(split_css(css, 1024), vec![css.to_string()]);
    }

    #[test]
    fn splits_at_rule_boundaries() {
        // Three ~24-byte rules, cap 50 -> two per chunk then one.
        let css = ".aaaa{text-align:center}.bbbb{text-align:center}.cccc{text-align:center}";
        let chunks = split_css(css, 50);
        assert!(chunks.len() >= 2, "expected a split, got {chunks:?}");
        // Every chunk ends on a rule boundary and none exceeds the cap except a
        // lone oversized rule (none here).
        for chunk in &chunks {
            assert!(chunk.ends_with('}'), "chunk not on a boundary: {chunk}");
        }
        // Rejoining reproduces the original.
        assert_eq!(chunks.concat(), css);
    }

    #[test]
    fn split_parts_each_refilter_to_themselves() {
        let mut css = String::new();
        for i in 0..200 {
            css.push_str(&format!(".c{i}{{margin-left:1em}}"));
        }
        let chunks = split_css(&css, 512);
        assert!(chunks.len() > 1, "expected multiple parts");
        for chunk in &chunks {
            assert!(chunk.len() <= 512 || chunk.matches('}').count() == 1);
            let FilteredCss {
                css: refiltered, ..
            } = filter_css(chunk, "part.css", false, &mut Vec::new());
            assert_eq!(&refiltered, chunk, "a split part must refilter to itself");
        }
    }

    #[test]
    fn oversized_single_rule_stays_whole() {
        let big = format!(".a{{{}}}", "margin-left:1em;".repeat(100));
        let chunks = split_css(&big, 32);
        assert_eq!(chunks, vec![big]);
    }

    #[test]
    fn split_path_inserts_index_before_extension() {
        assert_eq!(
            split_path("OEBPS/styles/main.css", 2),
            "OEBPS/styles/main-2.css"
        );
        assert_eq!(split_path("main.css", 3), "main-3.css");
        assert_eq!(split_path("noext", 2), "noext-2");
    }

    #[test]
    fn rule_cap_counts_every_comma_member() {
        // 750 rules of two selectors each are 1,500 stored rules on the device.
        let mut sheet = String::new();
        for i in 0..750 {
            sheet.push_str(&format!(".a{i},.b{i}{{margin-left:1em}}"));
        }
        assert!(store_cap_warnings(&[sheet.as_str()], 1500).is_empty());
        sheet.push_str(".c{margin-left:1em}");
        let warnings = store_cap_warnings(&[sheet.as_str()], 1500);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].message.contains("1501"), "{warnings:?}");
        assert!(warnings[0].file.is_none());
    }

    #[test]
    fn style_cap_counts_distinct_declaration_blocks_across_sheets() {
        let mut first = String::new();
        let mut second = String::new();
        for i in 0..128 {
            first.push_str(&format!(".a{i}{{margin-left:{i}px}}"));
            // Same blocks again in another sheet: not new.
            second.push_str(&format!(".b{i}{{margin-left:{i}px}}"));
            second.push_str(&format!(".c{i}{{margin-right:{i}px}}"));
        }
        assert!(store_cap_warnings(&[&first, &second], 1500).is_empty());
        second.push_str(".d{text-indent:-1em}");
        let warnings = store_cap_warnings(&[&first, &second], 1500);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].message.contains("257"), "{warnings:?}");
    }

    #[test]
    fn byte_identical_sheets_are_parsed_once() {
        // The device skips a stylesheet identical to an earlier one
        // (`Epub.cpp:257-318`), so a per-chapter copy adds nothing.
        let mut sheet = String::new();
        for i in 0..800 {
            sheet.push_str(&format!(".a{i}{{margin-left:1em}}"));
        }
        let warnings = store_cap_warnings(&[sheet.as_str(), sheet.as_str()], 1000);
        assert!(warnings.is_empty(), "{warnings:?}");
        let other = ".z{margin-left:1em}";
        let warnings = store_cap_warnings(&[sheet.as_str(), other, sheet.as_str()], 800);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].message.contains("801"), "{warnings:?}");
    }

    #[test]
    fn a_repeated_adjacent_selector_merges_into_one_stored_rule() {
        // `insertOrMerge` folds a rule into the entry stored just before it
        // (`CssParser.cpp:727-736`): one rule, one selector, one style.
        let sheet = ".a{margin-left:1em}.a{text-indent:1em}.a,.a{margin-top:1em}";
        assert!(store_cap_warnings(&[sheet], 1).is_empty());
        // With another selector in between, the device stores the repeat again.
        let sheet = ".a{margin-left:1em}.b{margin-left:1em}.a{text-indent:1em}";
        let warnings = store_cap_warnings(&[sheet], 2);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].message.contains('3'), "{warnings:?}");
    }

    #[test]
    fn selector_text_cap_counts_bytes() {
        // 128 selectors of 256 bytes (with the dot) are exactly 32 KB.
        let mut sheet = String::new();
        for i in 0..128 {
            sheet.push_str(&format!(".{}{i:03}{{margin:0}}", "x".repeat(252)));
        }
        assert!(store_cap_warnings(&[sheet.as_str()], 1500).is_empty());
        sheet.push_str(".y{margin:0}");
        let warnings = store_cap_warnings(&[sheet.as_str()], 1500);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].message.contains("32770"), "{warnings:?}");
    }
}
