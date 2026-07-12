//! XML text/attribute escaping shared by the XHTML serializer and the EPUB
//! output-document builders.

/// Append `s` to `out`, XML-escaping `&`, `<` and `>` (plus `"` when
/// `escape_quote` is set, for attribute values), and dropping control
/// characters that are invalid in XML 1.0 (everything below `U+0020` except
/// tab, line feed and carriage return).
pub(crate) fn escape_into(s: &str, escape_quote: bool, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if escape_quote => out.push_str("&quot;"),
            '\t' | '\n' | '\r' => out.push(c),
            c if (c as u32) < 0x20 => {}
            c => out.push(c),
        }
    }
}

/// XML-escape `s` for use as text content (`&`, `<`, `>`).
pub(crate) fn escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    escape_into(s, false, &mut out);
    out
}

/// XML-escape `s` for use inside a double-quoted attribute value
/// (`&`, `<`, `>`, `"`).
pub(crate) fn escape_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    escape_into(s, true, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_text_metacharacters() {
        assert_eq!(escape_text("a & b < c > d"), "a &amp; b &lt; c &gt; d");
    }

    #[test]
    fn text_keeps_double_quote_literal() {
        assert_eq!(escape_text(r#"say "hi""#), r#"say "hi""#);
    }

    #[test]
    fn attr_escapes_double_quote() {
        assert_eq!(escape_attr(r#"a"b&c"#), "a&quot;b&amp;c");
    }

    #[test]
    fn strips_invalid_control_chars_but_keeps_tab_lf_cr() {
        assert_eq!(escape_text("a\u{0}b\u{1}c\td\ne\rf"), "abc\td\ne\rf");
    }
}
