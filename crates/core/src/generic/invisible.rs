//! Invisible-character removal, script-aware.
//!
//! Zero-width characters are the standard modern text fingerprint: they survive
//! copy-paste and reformatting, so a per-copy payload encoded in them follows
//! the text everywhere. But `U+200C`/`U+200D` are *meaningful* in Arabic,
//! Hebrew and the Indic scripts, and `U+200D` joins emoji sequences, so a
//! blanket strip corrupts real content. Those two are removed only where the
//! surrounding characters cannot need them.

use crate::epub::Book;
use crate::report::Transformation;

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

/// Remove fingerprint characters from `text`, returning the result and how many
/// were removed.
pub fn scrub(text: &str) -> (String, usize) {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut removed = 0usize;
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
        if drop {
            removed += 1;
        } else {
            out.push(c);
        }
    }
    (out, removed)
}

/// Scrub every XHTML resource in the book.
pub(crate) fn scrub_book(book: &mut Book, transformations: &mut Vec<Transformation>) {
    let paths: Vec<String> = book
        .resources
        .iter()
        .filter(|(_, r)| r.media_type == "application/xhtml+xml")
        .map(|(p, _)| p.clone())
        .collect();
    for path in paths {
        let resource = &mut book.resources[&path];
        let Ok(text) = std::str::from_utf8(&resource.data) else {
            continue;
        };
        // A leading BOM is encoding, not a fingerprint: preserve it verbatim.
        let (bom, body) = match text.strip_prefix('\u{FEFF}') {
            Some(rest) => ("\u{FEFF}", rest),
            None => ("", text),
        };
        let (cleaned, removed) = scrub(body);
        if removed == 0 {
            continue;
        }
        resource.data = format!("{bom}{cleaned}").into_bytes();
        transformations.push(Transformation {
            kind: "generic-invisible".to_string(),
            detail: format!("removed {removed} invisible character(s)"),
            file: Some(path.clone()),
        });
    }
}
