//! Parsing the leading YAML frontmatter block of a Markdown book.
//!
//! [`crate::markdown`] hands this module the exact raw text comrak's
//! `front_matter` extension captured for a `NodeValue::FrontMatter` node
//! (opening delimiter line, YAML body, closing delimiter line, verbatim) - this
//! module never has to decide whether a block is present, only turn a known
//! one into a [`MetadataDoc`]. Unknown keys are ignored silently (no
//! `deny_unknown_fields`).
//!
//! The block *is* a [`MetadataDoc`] - the same type `--metadata` takes and
//! `metadata fetch` emits. So the full metadata vocabulary works here for free,
//! and a record looked up from Open Library can be pasted straight into the top
//! of a `.md` file. Before 0.2 this parsed four keys and silently swallowed the
//! rest, so `publisher:` in a front-matter block did precisely nothing.

use crate::error::ConvertError;
use crate::metadata::MetadataDoc;

/// The delimiter comrak's `front_matter` extension is configured with; see
/// [`crate::markdown::render::comrak_options`].
const DELIMITER: &str = "---";

/// Parse a frontmatter block's raw text (delimiters included, exactly as
/// captured by comrak's parser) into a [`MetadataDoc`].
///
/// # Errors
/// Returns [`ConvertError::InvalidMarkdown`] if the YAML between the
/// delimiters does not parse, with a message naming the line inside the block.
pub fn parse_frontmatter(raw_block: &str) -> Result<MetadataDoc, ConvertError> {
    let yaml_text = strip_delimiters(raw_block, DELIMITER);
    if yaml_text.trim().is_empty() {
        return Ok(MetadataDoc::default());
    }

    serde_yaml_ng::from_str(yaml_text).map_err(|e| {
        let context = e
            .location()
            .map(|loc| format!(" (line {} of the frontmatter block)", loc.line()))
            .unwrap_or_default();
        ConvertError::InvalidMarkdown(format!("frontmatter YAML is malformed{context}: {e}"))
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
    use crate::epub::model::Metadata;
    use crate::metadata::{self, MergeMode};

    /// Resolve a front-matter block the way `build_book` does, so the tests
    /// exercise the real path rather than the document's internal shape.
    fn resolved(raw: &str) -> Metadata {
        let doc = parse_frontmatter(raw).expect("valid frontmatter");
        let mut meta = Metadata::default();
        metadata::apply(
            &doc,
            &mut meta,
            MergeMode::Fill,
            &mut Vec::new(),
            &mut Vec::new(),
        );
        meta
    }

    #[test]
    fn parses_title_and_single_string_author() {
        let meta = resolved("---\ntitle: My Book\nauthor: Jane Doe\n---\n");
        assert_eq!(meta.title, "My Book");
        assert_eq!(meta.authors[0].name, "Jane Doe");
    }

    #[test]
    fn parses_author_as_a_list() {
        let meta = resolved("---\nauthor:\n  - Jane Doe\n  - John Smith\n---\n");
        let names: Vec<&str> = meta.authors.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, vec!["Jane Doe", "John Smith"]);
    }

    #[test]
    fn parses_language_and_cover() {
        let doc = parse_frontmatter("---\nlanguage: de\ncover: images/cover.jpg\n---\n")
            .expect("valid frontmatter");
        assert_eq!(doc.language, Some("de".to_string()));
        assert_eq!(doc.cover, Some("images/cover.jpg".to_string()));
    }

    #[test]
    fn missing_fields_default_to_none_or_empty() {
        let doc = parse_frontmatter("---\ntitle: Only Title\n---\n").expect("valid frontmatter");
        assert_eq!(doc.title, Some("Only Title".to_string()));
        assert!(doc.authors.is_none());
        assert_eq!(doc.language, None);
        assert_eq!(doc.cover, None);
    }

    #[test]
    fn the_full_metadata_vocabulary_works_in_frontmatter() {
        // Until 0.2 the block understood four keys and silently threw away the
        // rest, so a `publisher:` here did nothing at all. It is the same
        // document type `--metadata` takes now, so all of this lands.
        let meta = resolved(
            "---\n\
             title: A Book\n\
             author: Jane Author\n\
             publisher: Acme Press\n\
             description: A blurb.\n\
             subjects: [Fantasy, Adventure]\n\
             date: '1937-09-21'\n\
             isbn: '9780261102217'\n\
             series: The Chronicles\n\
             series_index: '2'\n\
             ---\n",
        );
        assert_eq!(meta.publisher.as_deref(), Some("Acme Press"));
        assert_eq!(meta.description.as_deref(), Some("A blurb."));
        assert_eq!(meta.subjects, vec!["Fantasy", "Adventure"]);
        assert_eq!(meta.date.as_deref(), Some("1937-09-21"));
        assert_eq!(meta.identifiers[0].value, "9780261102217");
        assert_eq!(meta.identifiers[0].scheme.as_deref(), Some("ISBN"));
        let series = meta.series.expect("series");
        assert_eq!(series.name, "The Chronicles");
        assert_eq!(series.index.as_deref(), Some("2"));
    }

    #[test]
    fn a_genuinely_unknown_key_is_still_ignored_silently() {
        let doc = parse_frontmatter("---\ntitle: T\nfavourite_biscuit: hobnob\n---\n")
            .expect("unknown keys must not error");
        assert_eq!(doc.title, Some("T".to_string()));
    }

    #[test]
    fn blank_block_yields_an_empty_document() {
        let doc = parse_frontmatter("---\n\n---\n").expect("blank block is valid");
        assert!(doc.is_empty());
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
