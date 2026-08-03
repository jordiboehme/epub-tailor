//! Invisible-character removal, script-aware.
//!
//! Zero-width characters are the standard modern text fingerprint: they survive
//! copy-paste and reformatting, so a per-copy payload encoded in them follows
//! the text everywhere. But `U+200C`/`U+200D` are *meaningful* in Arabic,
//! Hebrew and the Indic scripts, and `U+200D` joins emoji sequences, so a
//! blanket strip corrupts real content. Those two are removed only where the
//! surrounding characters cannot need them.
//!
//! The joiner-protection decision must see a document's *real* adjacent
//! characters, not just the ones sharing a DOM text node: markup routinely
//! sits between two characters that are visually and semantically adjacent -
//! `<span epub:type="pagebreak"/>` markers are common in print-derived EPUB 3
//! books, and inline `<span>`/`<b>`/emoji-component wrapping can split a
//! single word or a joined emoji sequence across several text nodes. A
//! per-node-only neighbour check sees `None` on both sides at every such
//! boundary and deletes the very characters this pass exists to protect. So
//! `scrub_dom` walks every text node in document order, runs the neighbour
//! analysis over the *concatenation* of their text, and writes the surviving
//! characters back node by node.

use crate::epub::{Book, Creator};
use crate::report::Transformation;
use kuchikiki::NodeRef;

/// Removed wherever they appear in text: none of these has a legitimate use
/// inside book prose. `U+FEFF` is handled by the caller when it leads a file,
/// where it is a byte-order mark rather than a joiner.
const ALWAYS: &[char] = &['\u{200B}', '\u{2060}', '\u{180E}', '\u{FEFF}'];

/// Whether `c` belongs to a script that uses ZWNJ/ZWJ to control shaping.
fn needs_joiners(c: char) -> bool {
    matches!(c as u32,
        0x0590..=0x05FF   // Hebrew
        | 0x0600..=0x06FF // Arabic
        | 0x0700..=0x074F // Syriac
        | 0x0750..=0x077F // Arabic Supplement
        | 0x0900..=0x097F // Devanagari
        | 0x0980..=0x09FF // Bengali
        | 0x0A00..=0x0A7F // Gurmukhi
        | 0x0A80..=0x0AFF // Gujarati
        | 0x0B00..=0x0B7F // Oriya
        | 0x0B80..=0x0BFF // Tamil
        | 0x0C00..=0x0C7F // Telugu
        | 0x0C80..=0x0CFF // Kannada
        | 0x0D00..=0x0D7F // Malayalam
        | 0x0D80..=0x0DFF // Sinhala
        | 0x0E00..=0x0E7F // Thai
        | 0x0F00..=0x0FFF // Tibetan
        | 0x1780..=0x17FF // Khmer
        | 0xFB50..=0xFDFF // Arabic Presentation Forms-A
        | 0xFE70..=0xFEFF // Arabic Presentation Forms-B
    )
}

/// Whether `c` is emoji-like, so a ZWJ next to it is a sequence joiner.
fn is_pictographic(c: char) -> bool {
    matches!(c as u32,
        // Covers the skin-tone modifiers (U+1F3FB..=U+1F3FF) already.
        0x1F000..=0x1FAFF
        | 0x2600..=0x27BF
        | 0xFE0F           // variation selector-16
    )
}

/// For each character in `chars`, whether it is kept (`true`) or a
/// fingerprint to drop (`false`). Shared by [`scrub`] (a bare string, used
/// directly for model strings which have no markup to split them) and
/// [`scrub_dom`] (a whole document's text spanning many DOM text nodes), so
/// both make the identical protection decision from the identical rule.
fn keep_mask(chars: &[char]) -> Vec<bool> {
    let mut keep = vec![true; chars.len()];
    for (i, &c) in chars.iter().enumerate() {
        let drop = if ALWAYS.contains(&c) {
            true
        } else if c == '\u{200C}' || c == '\u{200D}' {
            let prev = chars[..i]
                .iter()
                .rev()
                .find(|c| !c.is_whitespace())
                .copied();
            let next = chars[i + 1..].iter().find(|c| !c.is_whitespace()).copied();
            let protected =
                |n: Option<char>| n.is_some_and(|n| needs_joiners(n) || is_pictographic(n));
            !(protected(prev) || protected(next))
        } else {
            false
        };
        keep[i] = !drop;
    }
    keep
}

/// Remove fingerprint characters from `text`, returning the result and how many
/// were removed.
pub fn scrub(text: &str) -> (String, usize) {
    let chars: Vec<char> = text.chars().collect();
    let keep = keep_mask(&chars);
    let mut out = String::with_capacity(text.len());
    let mut removed = 0usize;
    for (&c, &k) in chars.iter().zip(&keep) {
        if k {
            out.push(c);
        } else {
            removed += 1;
        }
    }
    (out, removed)
}

/// Scrub invisible fingerprint characters from every text node in `doc`, with
/// the joiner-protection decision made against the concatenation of the whole
/// document's text in document order - not each text node in isolation - so a
/// joiner whose true neighbours live in a sibling node is still protected.
/// Returns the number of characters removed.
pub(crate) fn scrub_dom(doc: &NodeRef) -> usize {
    // Each text node, paired with its `[start, end)` character range in the
    // concatenated document text.
    let mut nodes: Vec<(NodeRef, usize, usize)> = Vec::new();
    let mut chars: Vec<char> = Vec::new();
    for node in doc.inclusive_descendants() {
        let Some(cell) = node.as_text() else {
            continue;
        };
        let start = chars.len();
        chars.extend(cell.borrow().chars());
        let end = chars.len();
        nodes.push((node, start, end));
    }
    if chars.is_empty() {
        return 0;
    }
    let keep = keep_mask(&chars);
    let mut removed = 0usize;
    for (node, start, end) in nodes {
        let dropped = keep[start..end].iter().filter(|k| !**k).count();
        if dropped == 0 {
            continue;
        }
        let rebuilt: String = chars[start..end]
            .iter()
            .zip(&keep[start..end])
            .filter_map(|(&c, &k)| k.then_some(c))
            .collect();
        let cell = node.as_text().expect("collected as a text node above");
        *cell.borrow_mut() = rebuilt;
        removed += dropped;
    }
    removed
}

/// Scrub one chapter's parsed DOM in place and record a transformation if
/// anything was removed. Called from both places `convert()` already parses
/// XHTML (the spine loop and the non-spine loop), so this never costs an
/// extra parse - and parsing has already decoded any entity-encoded
/// fingerprint (`&#8203;`, `&zwnj;`, ...), which a raw-bytes pass would miss.
pub(crate) fn scrub_chapter(doc: &NodeRef, transformations: &mut Vec<Transformation>, path: &str) {
    let removed = scrub_dom(doc);
    if removed > 0 {
        transformations.push(Transformation {
            kind: "generic-invisible".to_string(),
            detail: format!("removed {removed} invisible character(s)"),
            file: Some(path.to_string()),
        });
    }
}

/// Scrub every string the writer regenerates the OPF/nav/NCX from:
/// `book.metadata` and `book.toc`. The writer builds those three documents
/// straight from this model (see `epub::write`), discarding whatever bytes
/// `book.resources` holds for the nav document - so a fingerprint in the
/// title, an author name, the description or a TOC entry title would
/// otherwise survive every chapter-level scrub untouched. Mirrors
/// `normalize_model_strings`' structure and its documented exclusions (the
/// unique identifier and every href/path are never touched).
pub(crate) fn scrub_model_strings(book: &mut Book, transformations: &mut Vec<Transformation>) {
    fn clean(s: &mut String) -> usize {
        let (cleaned, removed) = scrub(s);
        if removed > 0 {
            *s = cleaned;
        }
        removed
    }
    fn clean_opt(s: &mut Option<String>) -> usize {
        s.as_mut().map(clean).unwrap_or(0)
    }
    fn clean_creators(creators: &mut [Creator]) -> usize {
        let mut removed = 0usize;
        for creator in creators {
            removed += clean(&mut creator.name);
            removed += clean_opt(&mut creator.file_as);
            removed += clean_opt(&mut creator.role);
        }
        removed
    }

    let m = &mut book.metadata;
    let mut removed = 0usize;
    removed += clean(&mut m.title);
    removed += clean_creators(&mut m.authors);
    removed += clean_creators(&mut m.contributors);
    removed += clean(&mut m.language);
    for id in &mut m.identifiers {
        removed += clean(&mut id.value);
        removed += clean_opt(&mut id.scheme);
    }
    removed += clean_opt(&mut m.description);
    removed += clean_opt(&mut m.publisher);
    for subject in &mut m.subjects {
        removed += clean(subject);
    }
    removed += clean_opt(&mut m.date);
    removed += clean_opt(&mut m.rights);
    if let Some(series) = &mut m.series {
        removed += clean(&mut series.name);
        removed += clean_opt(&mut series.index);
    }
    if removed > 0 {
        transformations.push(Transformation {
            kind: "generic-invisible".to_string(),
            detail: format!("removed {removed} invisible character(s) from metadata"),
            file: None,
        });
    }

    let mut toc_removed = 0usize;
    for entry in &mut book.toc {
        toc_removed += clean(&mut entry.title);
    }
    if toc_removed > 0 {
        transformations.push(Transformation {
            kind: "generic-invisible".to_string(),
            detail: format!(
                "removed {toc_removed} invisible character(s) from the table of contents"
            ),
            file: None,
        });
    }
}
