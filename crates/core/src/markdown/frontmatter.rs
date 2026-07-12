//! Parsing the leading YAML frontmatter block of a Markdown book.
//!
//! [`crate::markdown`] hands this module the exact raw text comrak's
//! `front_matter` extension captured for a `NodeValue::FrontMatter` node
//! (opening delimiter line, YAML body, closing delimiter line, verbatim) - this
//! module never has to decide whether a block is present, only turn a known
//! one into a [`Frontmatter`]. Unknown keys are ignored silently (no
//! `deny_unknown_fields`).

use serde::Deserialize;

use crate::error::ConvertError;

/// Book metadata read from the Markdown frontmatter block. Every field is
/// optional: [`crate::markdown`] fills in the fallbacks (first H1 for the
/// title, `"en"` for the language) once the rest of the document has been
/// parsed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Frontmatter {
    pub title: Option<String>,
    pub authors: Vec<String>,
    pub language: Option<String>,
    pub cover: Option<String>,
}

/// The YAML shape accepted in the frontmatter block, before `author`'s
/// string-or-list flexibility is normalized away.
#[derive(Debug, Deserialize)]
struct RawFrontmatter {
    title: Option<String>,
    author: Option<AuthorField>,
    language: Option<String>,
    cover: Option<String>,
}

/// `author:` accepts either a single string or a list of strings.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum AuthorField {
    One(String),
    Many(Vec<String>),
}

impl AuthorField {
    fn into_vec(self) -> Vec<String> {
        match self {
            AuthorField::One(s) => vec![s],
            AuthorField::Many(v) => v,
        }
    }
}

/// The delimiter comrak's `front_matter` extension is configured with; see
/// [`crate::markdown::render::comrak_options`].
const DELIMITER: &str = "---";

/// Parse a frontmatter block's raw text (delimiters included, exactly as
/// captured by comrak's parser) into a [`Frontmatter`].
///
/// # Errors
/// Returns [`ConvertError::InvalidMarkdown`] if the YAML between the
/// delimiters does not parse, with a message naming the line inside the block.
pub fn parse_frontmatter(raw_block: &str) -> Result<Frontmatter, ConvertError> {
    let yaml_text = strip_delimiters(raw_block, DELIMITER);
    if yaml_text.trim().is_empty() {
        return Ok(Frontmatter::default());
    }

    let raw: RawFrontmatter = serde_yaml_ng::from_str(yaml_text).map_err(|e| {
        let context = e
            .location()
            .map(|loc| format!(" (line {} of the frontmatter block)", loc.line()))
            .unwrap_or_default();
        ConvertError::InvalidMarkdown(format!("frontmatter YAML is malformed{context}: {e}"))
    })?;

    Ok(Frontmatter {
        title: raw.title,
        authors: raw.author.map(AuthorField::into_vec).unwrap_or_default(),
        language: raw.language,
        cover: raw.cover,
    })
}

/// Strip the opening and closing delimiter lines from a raw frontmatter block,
/// returning just the YAML text between them. `raw_block` is assumed to be a
/// well-formed capture (starts with `delimiter` alone on its own line, and
/// contains a matching closing line later) - the only shape comrak ever hands
/// this module.
fn strip_delimiters<'a>(raw_block: &'a str, delimiter: &str) -> &'a str {
    let Some(after_open) = raw_block.strip_prefix(delimiter).and_then(|rest| {
        rest.strip_prefix('\n')
            .or_else(|| rest.strip_prefix("\r\n"))
    }) else {
        return raw_block;
    };
    let mut consumed = 0;
    for line in after_open.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed == delimiter {
            return &after_open[..consumed];
        }
        consumed += line.len();
    }
    // No closing line found (should not happen for a comrak-captured block,
    // e.g. a delimiter right at EOF with nothing after it): treat the rest as
    // the whole body rather than panicking.
    after_open
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_title_and_single_string_author() {
        let raw = "---\ntitle: My Book\nauthor: Jane Doe\n---\n";
        let fm = parse_frontmatter(raw).expect("valid frontmatter");
        assert_eq!(fm.title, Some("My Book".to_string()));
        assert_eq!(fm.authors, vec!["Jane Doe".to_string()]);
    }

    #[test]
    fn parses_author_as_a_list() {
        let raw = "---\nauthor:\n  - Jane Doe\n  - John Smith\n---\n";
        let fm = parse_frontmatter(raw).expect("valid frontmatter");
        assert_eq!(
            fm.authors,
            vec!["Jane Doe".to_string(), "John Smith".to_string()]
        );
    }

    #[test]
    fn parses_language_and_cover() {
        let raw = "---\nlanguage: de\ncover: images/cover.jpg\n---\n";
        let fm = parse_frontmatter(raw).expect("valid frontmatter");
        assert_eq!(fm.language, Some("de".to_string()));
        assert_eq!(fm.cover, Some("images/cover.jpg".to_string()));
    }

    #[test]
    fn missing_fields_default_to_none_or_empty() {
        let raw = "---\ntitle: Only Title\n---\n";
        let fm = parse_frontmatter(raw).expect("valid frontmatter");
        assert_eq!(fm.title, Some("Only Title".to_string()));
        assert!(fm.authors.is_empty());
        assert_eq!(fm.language, None);
        assert_eq!(fm.cover, None);
    }

    #[test]
    fn unknown_keys_are_ignored_silently() {
        let raw = "---\ntitle: T\npublisher: Acme\n---\n";
        let fm = parse_frontmatter(raw).expect("unknown keys must not error");
        assert_eq!(fm.title, Some("T".to_string()));
    }

    #[test]
    fn blank_block_yields_default_frontmatter() {
        let raw = "---\n\n---\n";
        let fm = parse_frontmatter(raw).expect("blank block is valid");
        assert_eq!(fm, Frontmatter::default());
    }

    #[test]
    fn malformed_yaml_errors_with_line_context() {
        let raw = "---\ntitle: [unterminated\n---\n";
        let err = parse_frontmatter(raw).expect_err("malformed YAML must error");
        match err {
            ConvertError::InvalidMarkdown(msg) => {
                assert!(msg.contains("line"), "expected line context, got: {msg}");
            }
            other => panic!("expected InvalidMarkdown, got {other:?}"),
        }
    }

    #[test]
    fn wrong_shape_for_a_known_key_errors() {
        // `title` must be a string; a mapping is a type error, not silently
        // ignored (unlike a genuinely unknown key).
        let raw = "---\ntitle:\n  nested: yes\n---\n";
        let err = parse_frontmatter(raw).expect_err("wrong-shaped title must error");
        assert!(matches!(err, ConvertError::InvalidMarkdown(_)));
    }
}
