//! Parsing XHTML/HTML bytes into a kuchikiki DOM.

use kuchikiki::NodeRef;
use kuchikiki::traits::*;

use crate::error::ConvertError;

/// Parse XHTML/HTML `bytes` into a kuchikiki DOM tree.
///
/// The input is expected to already be UTF-8 (M1's reader normalizes every
/// text resource). html5ever's parser recovers from malformed markup rather
/// than failing, so the only error case is genuinely undecodable input.
///
/// # Errors
/// Returns [`ConvertError::InvalidEpub`] if `bytes` is not valid UTF-8.
pub fn parse_xhtml(bytes: &[u8]) -> Result<NodeRef, ConvertError> {
    let text = std::str::from_utf8(bytes).map_err(|e| {
        ConvertError::InvalidEpub(format!("XHTML document is not valid UTF-8: {e}"))
    })?;
    Ok(kuchikiki::parse_html().one(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_wellformed_document() {
        let doc = parse_xhtml(b"<html><head><title>T</title></head><body><p>Hi</p></body></html>")
            .expect("should parse");
        assert!(
            doc.select("p").expect("valid selector").count() == 1,
            "expected exactly one <p>"
        );
    }

    #[test]
    fn recovers_from_malformed_markup() {
        // Unclosed tags, stray text: html5ever recovers instead of erroring.
        let doc = parse_xhtml(b"<html><body><p>one<p>two<b>bold").expect("should recover");
        assert_eq!(doc.select("p").expect("valid selector").count(), 2);
    }

    #[test]
    fn rejects_invalid_utf8() {
        let err = parse_xhtml(&[0xFF, 0xFE, 0x00]).expect_err("invalid UTF-8 should error");
        assert!(matches!(err, ConvertError::InvalidEpub(_)));
    }
}
