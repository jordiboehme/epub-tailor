//! Enforce the device's CSS byte and rule caps.
//!
//! The firmware reads at most `css_max_bytes` from any single CSS file and
//! parses at most `css_max_rules` rules across the whole book. We split an
//! oversized file at rule boundaries (the device zip-scans every `.css`, so the
//! extra parts are still read) and warn - but never delete - when the book as a
//! whole exceeds the rule cap, since the excess is harmless and its handling is
//! order-dependent on device.

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

/// A book-wide warning when the total kept rule count exceeds the device cap.
pub(crate) fn rule_cap_warning(total_rules: usize, max_rules: usize) -> Option<Warning> {
    (total_rules > max_rules).then(|| Warning {
        message: format!(
            "the book has {total_rules} CSS rules over the device cap of {max_rules}; \
             the device will drop the rules past the cap"
        ),
        file: None,
    })
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
            } = filter_css(chunk, "part.css", &mut Vec::new());
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
    fn rule_cap_warns_only_over_the_limit() {
        assert!(rule_cap_warning(1500, 1500).is_none());
        let warning = rule_cap_warning(1501, 1500).expect("should warn over the cap");
        assert!(warning.message.contains("1501"));
        assert!(warning.file.is_none());
    }
}
