//! Text-node Unicode hygiene.
//!
//! The device reads UTF-8 verbatim, does no Unicode normalization, and hard
//! cuts words at 200 bytes. We NFC-normalize text so combining marks render,
//! strip characters that are invalid in XML 1.0 (plus a stray BOM), and warn
//! about over-long words rather than mutating them (injecting soft breaks
//! risks tofu glyphs on the device font).

use kuchikiki::NodeRef;
use unicode_normalization::{UnicodeNormalization, is_nfc};

use crate::report::{Transformation, Warning};

/// Words strictly longer than this many bytes are hard-cut by the device.
const MAX_WORD_BYTES: usize = 200;

/// NFC-normalize and sanitize every text node in `doc`, and warn about words
/// the device will hard-cut.
pub(crate) fn unicode_hygiene(
    doc: &NodeRef,
    report: &mut Vec<Transformation>,
    warnings: &mut Vec<Warning>,
    chapter_path: &str,
) {
    let mut changed = false;
    let mut long_words = 0usize;
    let mut longest = 0usize;

    for node in doc.inclusive_descendants() {
        let Some(cell) = node.as_text() else { continue };
        let original = cell.borrow().clone();
        let sanitized = sanitize(&original);
        let normalized = if is_nfc(&sanitized) {
            sanitized
        } else {
            sanitized.nfc().collect::<String>()
        };
        if normalized != original {
            *cell.borrow_mut() = normalized.clone();
            changed = true;
        }
        for word in normalized.split_whitespace() {
            if word.len() > MAX_WORD_BYTES {
                long_words += 1;
                longest = longest.max(word.len());
            }
        }
    }

    if changed {
        report.push(Transformation {
            kind: "text-nfc".to_string(),
            detail: "normalized text to NFC and stripped invalid characters".to_string(),
            file: Some(chapter_path.to_string()),
        });
    }
    if long_words > 0 {
        warnings.push(Warning {
            message: format!(
                "{long_words} word(s) exceed {MAX_WORD_BYTES} bytes (longest {longest} bytes); \
                 the device hard-cuts them"
            ),
            file: Some(chapter_path.to_string()),
        });
    }
}

/// Drop characters invalid in XML 1.0 (C0 controls other than tab/LF/CR) and
/// any U+FEFF, leaving all other characters untouched.
fn sanitize(s: &str) -> String {
    s.chars()
        .filter(|c| *c != '\u{FEFF}' && ((*c as u32) >= 0x20 || matches!(c, '\t' | '\n' | '\r')))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::testutil::{doc_from_body, serialize};

    fn run(body: &str) -> (String, Vec<Transformation>, Vec<Warning>) {
        let doc = doc_from_body(body);
        let mut report = Vec::new();
        let mut warnings = Vec::new();
        unicode_hygiene(&doc, &mut report, &mut warnings, "ch.xhtml");
        (serialize(&doc), report, warnings)
    }

    #[test]
    fn nfc_normalizes_decomposed_text_and_records_transformation() {
        // "e" + U+0301 COMBINING ACUTE ACCENT -> precomposed "é".
        let (out, report, _) = run("<p>cafe\u{0301}</p>");
        assert!(out.contains("<p>café</p>"), "got: {out}");
        assert_eq!(report.len(), 1);
        assert_eq!(report[0].kind, "text-nfc");
    }

    #[test]
    fn already_nfc_text_is_left_alone_and_records_nothing() {
        let (out, report, warnings) = run("<p>café</p>");
        assert!(out.contains("<p>café</p>"), "got: {out}");
        assert!(report.is_empty(), "no transformation expected");
        assert!(warnings.is_empty());
    }

    #[test]
    fn strips_bom_and_control_chars_from_text() {
        let (out, report, _) = run("<p>a\u{FEFF}b\u{0007}c</p>");
        assert!(out.contains("<p>abc</p>"), "got: {out}");
        assert_eq!(report.len(), 1, "stripping counts as a change");
    }

    #[test]
    fn long_word_warns_without_mutating() {
        let long = "x".repeat(250);
        let (out, report, warnings) = run(&format!("<p>{long}</p>"));
        assert!(out.contains(&long), "the long word must survive verbatim");
        assert!(report.is_empty(), "no NFC change, so no transformation");
        assert_eq!(warnings.len(), 1);
        assert!(
            warnings[0].message.contains("250 bytes"),
            "got: {}",
            warnings[0].message
        );
    }

    #[test]
    fn multibyte_word_length_is_measured_in_bytes() {
        // 120 'é' = 240 bytes but only 120 chars: must trip the byte threshold.
        let word = "é".repeat(120);
        let (_, _, warnings) = run(&format!("<p>{word}</p>"));
        assert_eq!(warnings.len(), 1, "240 bytes should warn");
    }
}
