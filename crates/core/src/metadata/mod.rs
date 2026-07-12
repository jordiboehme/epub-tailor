//! The metadata document: one shape, three doors.
//!
//! [`MetadataDoc`] is what the user hands us when the book itself is missing
//! something. The same type is used by:
//!
//! - `--metadata <file|->` on `fit` and `md` (JSON or YAML),
//! - Markdown front-matter (which is just YAML at the top of the file),
//! - and the output of `epub-tailor metadata fetch`, so a looked-up record can
//!   be piped straight back in.
//!
//! One type for all three means a record fetched from Open Library is a legal
//! `--metadata` file and a legal Markdown front-matter block, with no
//! translation step and nothing to keep in sync.
//!
//! Every field is optional, and absent means *say nothing* - not *clear it*.
//! That is what makes the default [`MergeMode::Fill`] safe: you can hand over a
//! document with only a publisher in it and be certain nothing else moves.

pub mod openlibrary;

use serde::{Deserialize, Serialize};

use crate::epub::model::{Creator, Identifier, Metadata, Series};
use crate::report::{Transformation, Warning};

/// Accepts either a single value or a list of them, so `author: Jane Author`
/// and `authors: [Jane, Bill]` both work. The Markdown front-matter has always
/// been this lenient about `author:`; this keeps that promise and extends it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OneOrMany<T> {
    One(T),
    Many(Vec<T>),
}

impl<T> OneOrMany<T> {
    fn into_vec(self) -> Vec<T> {
        match self {
            OneOrMany::One(one) => vec![one],
            OneOrMany::Many(many) => many,
        }
    }
}

/// User-supplied metadata. Every field optional; absent means "leave it alone".
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MetadataDoc {
    pub title: Option<String>,
    /// `author:` and `authors:` are the same key; either takes a name or a list.
    #[serde(alias = "author", skip_serializing_if = "Option::is_none")]
    pub authors: Option<OneOrMany<Creator>>,
    #[serde(alias = "contributor", skip_serializing_if = "Option::is_none")]
    pub contributors: Option<OneOrMany<Creator>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// The book's unique identifier. Only ever *fills* an absent one - see
    /// [`apply`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<String>,
    /// Secondary identifiers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifiers: Option<Vec<Identifier>>,
    /// Shorthand: `isbn: 9780261102217` is the same as adding an identifier
    /// with scheme `ISBN`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub isbn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(alias = "subject", skip_serializing_if = "Option::is_none")]
    pub subjects: Option<OneOrMany<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rights: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series_index: Option<String>,
    /// Path to a cover image to embed. Relative to the document for Markdown
    /// front-matter; relative to the working directory for `--metadata`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cover: Option<String>,
}

/// How a [`MetadataDoc`] meets the metadata already in the book.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MergeMode {
    /// Only write a field the book does not already have. The default, because
    /// the problem being solved is *missing* metadata: a lookup should never
    /// quietly overwrite a publisher the book already got right.
    #[default]
    Fill,
    /// Every field the document mentions wins.
    Replace,
}

impl MetadataDoc {
    /// Whether this document says anything at all.
    pub fn is_empty(&self) -> bool {
        let MetadataDoc {
            title,
            authors,
            contributors,
            language,
            identifier,
            identifiers,
            isbn,
            description,
            publisher,
            subjects,
            date,
            rights,
            series,
            series_index,
            cover,
        } = self;
        title.is_none()
            && authors.is_none()
            && contributors.is_none()
            && language.is_none()
            && identifier.is_none()
            && identifiers.is_none()
            && isbn.is_none()
            && description.is_none()
            && publisher.is_none()
            && subjects.is_none()
            && date.is_none()
            && rights.is_none()
            && series.is_none()
            && series_index.is_none()
            && cover.is_none()
    }

    /// Parse a document from JSON or YAML. YAML is a superset of JSON, so one
    /// parser reads both and the caller never has to guess from the extension.
    pub fn parse(text: &str) -> Result<MetadataDoc, String> {
        serde_yaml_ng::from_str(text).map_err(|e| e.to_string())
    }
}

/// Merge `doc` into `metadata`, returning the cover path it asked for (if any),
/// which the caller must resolve and embed - this module never touches files.
///
/// Two rules are not negotiable, whatever the [`MergeMode`]:
///
/// 1. **The unique identifier is only ever filled, never replaced.** A reading
///    system keys its library and your reading position off it; swapping it
///    orphans every bookmark you have. A document that tries is refused with a
///    warning, and the value is kept as a *secondary* identifier instead, where
///    it is useful and harmless.
/// 2. **An ISBN is added, not substituted.** Same reason.
///
/// This mirrors the existing caution in the content filters, which refuse to
/// let a rule empty the title and warn instead of doing it.
pub fn apply(
    doc: &MetadataDoc,
    metadata: &mut Metadata,
    mode: MergeMode,
    transformations: &mut Vec<Transformation>,
    warnings: &mut Vec<Warning>,
) -> Option<String> {
    let replace = mode == MergeMode::Replace;
    let mut set = |field: &str| {
        transformations.push(Transformation {
            kind: "metadata-set".to_string(),
            detail: format!("set {field} from the supplied metadata"),
            file: None,
        });
    };

    // Scalars: in fill mode, only when the book is silent.
    macro_rules! fill_opt {
        ($($name:ident),* $(,)?) => {
            $(
                if let Some(value) = doc.$name.clone()
                    && (replace || metadata.$name.is_none())
                {
                    metadata.$name = Some(value);
                    set(stringify!($name));
                }
            )*
        };
    }
    fill_opt!(description, publisher, date, rights);

    if let Some(title) = doc.title.clone()
        && !title.trim().is_empty()
        && (replace || metadata.title.is_empty() || metadata.title == "Untitled")
    {
        metadata.title = title;
        set("title");
    }
    if let Some(language) = doc.language.clone()
        && !language.trim().is_empty()
        && (replace || metadata.language.is_empty())
    {
        metadata.language = language;
        set("language");
    }
    if let Some(authors) = doc.authors.clone()
        && (replace || metadata.authors.is_empty())
    {
        metadata.authors = authors.into_vec();
        set("authors");
    }
    if let Some(contributors) = doc.contributors.clone()
        && (replace || metadata.contributors.is_empty())
    {
        metadata.contributors = contributors.into_vec();
        set("contributors");
    }
    if let Some(subjects) = doc.subjects.clone()
        && (replace || metadata.subjects.is_empty())
    {
        metadata.subjects = subjects.into_vec();
        set("subjects");
    }

    if let Some(name) = doc.series.clone()
        && !name.trim().is_empty()
        && (replace || metadata.series.is_none())
    {
        metadata.series = Some(Series {
            name,
            index: doc.series_index.clone(),
        });
        set("series");
    } else if let Some(index) = doc.series_index.clone()
        && let Some(series) = metadata.series.as_mut()
        && (replace || series.index.is_none())
    {
        series.index = Some(index);
        set("series index");
    }

    // Rule 1: the unique identifier is filled, never replaced.
    if let Some(wanted) = doc.identifier.clone().filter(|s| !s.trim().is_empty()) {
        match metadata.identifier.as_deref() {
            None => {
                metadata.identifier = Some(wanted);
                set("identifier");
            }
            Some(existing) if existing == wanted => {}
            Some(_) => {
                warnings.push(Warning {
                    message: format!(
                        "refusing to change the book's unique identifier to \"{wanted}\": a reading \
                         system keys your library and reading position off it. Kept it as a \
                         secondary identifier instead."
                    ),
                    file: None,
                });
                push_identifier(
                    metadata,
                    Identifier {
                        value: wanted,
                        scheme: None,
                    },
                );
            }
        }
    }

    // Rule 2: ISBNs and other identifiers are added alongside, never on top of.
    for id in doc.identifiers.clone().unwrap_or_default() {
        if !id.value.trim().is_empty() && push_identifier(metadata, id) {
            set("identifier");
        }
    }
    if let Some(isbn) = doc.isbn.clone().filter(|s| !s.trim().is_empty()) {
        let added = push_identifier(
            metadata,
            Identifier {
                value: isbn,
                scheme: Some("ISBN".to_string()),
            },
        );
        if added {
            set("ISBN");
        }
    }

    doc.cover.clone().filter(|s| !s.trim().is_empty())
}

/// Add a secondary identifier unless the book already carries that value
/// (either as its unique id or as a secondary one). Returns whether it landed.
fn push_identifier(metadata: &mut Metadata, id: Identifier) -> bool {
    let value = id.value.trim();
    if metadata.identifier.as_deref() == Some(value)
        || metadata.identifiers.iter().any(|e| e.value == value)
    {
        return false;
    }
    metadata.identifiers.push(id);
    true
}

/// The metadata fields a book is missing, by name - what `metadata show`
/// reports and what a lookup would be able to fill.
pub fn missing_fields(metadata: &Metadata) -> Vec<&'static str> {
    let mut missing = Vec::new();
    if metadata.title.is_empty() || metadata.title == "Untitled" {
        missing.push("title");
    }
    if metadata.authors.is_empty() {
        missing.push("authors");
    }
    if metadata.description.is_none() {
        missing.push("description");
    }
    if metadata.publisher.is_none() {
        missing.push("publisher");
    }
    if metadata.subjects.is_empty() {
        missing.push("subjects");
    }
    if metadata.date.is_none() {
        missing.push("date");
    }
    if metadata.series.is_none() {
        missing.push("series");
    }
    if metadata.identifiers.is_empty() {
        missing.push("isbn");
    }
    missing
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book_with_publisher() -> Metadata {
        Metadata {
            title: "A Book".to_string(),
            authors: vec![Creator::new("Jane Author")],
            language: "en".to_string(),
            identifier: Some("urn:uuid:original".to_string()),
            publisher: Some("The Real Publisher".to_string()),
            ..Metadata::default()
        }
    }

    fn apply_doc(doc: &MetadataDoc, meta: &mut Metadata, mode: MergeMode) -> Vec<Warning> {
        let mut t = Vec::new();
        let mut w = Vec::new();
        apply(doc, meta, mode, &mut t, &mut w);
        w
    }

    #[test]
    fn fill_mode_does_not_clobber_what_the_book_already_says() {
        let mut meta = book_with_publisher();
        let doc = MetadataDoc {
            publisher: Some("A Worse Guess".to_string()),
            description: Some("A blurb.".to_string()),
            ..Default::default()
        };
        apply_doc(&doc, &mut meta, MergeMode::Fill);
        assert_eq!(meta.publisher.as_deref(), Some("The Real Publisher"));
        assert_eq!(meta.description.as_deref(), Some("A blurb."));
    }

    #[test]
    fn replace_mode_overwrites() {
        let mut meta = book_with_publisher();
        let doc = MetadataDoc {
            publisher: Some("A Deliberate Choice".to_string()),
            ..Default::default()
        };
        apply_doc(&doc, &mut meta, MergeMode::Replace);
        assert_eq!(meta.publisher.as_deref(), Some("A Deliberate Choice"));
    }

    #[test]
    fn the_unique_identifier_is_never_replaced_even_in_replace_mode() {
        // Swapping it orphans every bookmark the reader has.
        let mut meta = book_with_publisher();
        let doc = MetadataDoc {
            identifier: Some("urn:uuid:something-else".to_string()),
            ..Default::default()
        };
        let warnings = apply_doc(&doc, &mut meta, MergeMode::Replace);
        assert_eq!(meta.identifier.as_deref(), Some("urn:uuid:original"));
        assert_eq!(warnings.len(), 1, "the refusal must be reported");
        assert!(warnings[0].message.contains("refusing to change"));
        // ...but it is not thrown away: it survives as a secondary identifier.
        assert_eq!(meta.identifiers[0].value, "urn:uuid:something-else");
    }

    #[test]
    fn an_isbn_is_added_alongside_the_unique_identifier() {
        let mut meta = book_with_publisher();
        let doc = MetadataDoc {
            isbn: Some("9780261102217".to_string()),
            ..Default::default()
        };
        apply_doc(&doc, &mut meta, MergeMode::Fill);
        assert_eq!(meta.identifier.as_deref(), Some("urn:uuid:original"));
        assert_eq!(meta.identifiers.len(), 1);
        assert_eq!(meta.identifiers[0].value, "9780261102217");
        assert_eq!(meta.identifiers[0].scheme.as_deref(), Some("ISBN"));
    }

    #[test]
    fn the_same_isbn_twice_is_not_added_twice() {
        let mut meta = book_with_publisher();
        let doc = MetadataDoc {
            isbn: Some("9780261102217".to_string()),
            ..Default::default()
        };
        apply_doc(&doc, &mut meta, MergeMode::Fill);
        apply_doc(&doc, &mut meta, MergeMode::Fill);
        assert_eq!(meta.identifiers.len(), 1);
    }

    #[test]
    fn an_author_parses_from_a_bare_string_a_list_or_an_object() {
        let doc = MetadataDoc::parse("author: Jane Author").expect("bare string");
        let mut meta = Metadata::default();
        apply_doc(&doc, &mut meta, MergeMode::Fill);
        assert_eq!(meta.authors, vec![Creator::new("Jane Author")]);

        let doc = MetadataDoc::parse("authors: [Jane Author, Bill Writer]").expect("list");
        let mut meta = Metadata::default();
        apply_doc(&doc, &mut meta, MergeMode::Fill);
        assert_eq!(meta.authors.len(), 2);

        let doc = MetadataDoc::parse(
            "authors:\n  - name: Bill Writer\n    file_as: 'Writer, Bill'\n    role: edt\n",
        )
        .expect("object");
        let mut meta = Metadata::default();
        apply_doc(&doc, &mut meta, MergeMode::Fill);
        assert_eq!(meta.authors[0].file_as.as_deref(), Some("Writer, Bill"));
        assert_eq!(meta.authors[0].role.as_deref(), Some("edt"));
    }

    #[test]
    fn json_and_yaml_are_both_accepted() {
        let json = MetadataDoc::parse(r#"{"publisher": "Allen & Unwin", "subjects": ["Fantasy"]}"#)
            .expect("json parses");
        assert_eq!(json.publisher.as_deref(), Some("Allen & Unwin"));
        let yaml = MetadataDoc::parse("publisher: Allen & Unwin\nsubject: Fantasy\n")
            .expect("yaml parses");
        assert_eq!(yaml.publisher.as_deref(), Some("Allen & Unwin"));
    }

    #[test]
    fn missing_fields_names_what_a_lookup_could_fill() {
        let missing = missing_fields(&book_with_publisher());
        assert!(missing.contains(&"description"));
        assert!(missing.contains(&"subjects"));
        assert!(!missing.contains(&"publisher"), "it has one");
        assert!(!missing.contains(&"title"), "it has one");
    }

    #[test]
    fn an_empty_document_changes_nothing() {
        let mut meta = book_with_publisher();
        let before = meta.clone();
        let doc = MetadataDoc::default();
        assert!(doc.is_empty());
        apply_doc(&doc, &mut meta, MergeMode::Replace);
        assert_eq!(meta, before);
    }
}
