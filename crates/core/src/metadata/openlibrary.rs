//! Turning Open Library's JSON into [`MetadataDoc`]s.
//!
//! **This module makes no network calls.** It is a pure function from a
//! response body to candidates, which is what lets it be tested exhaustively
//! from recorded fixtures with no socket in sight. The HTTP request itself
//! lives in the CLI, behind the `online` feature, so this crate keeps zero
//! network dependencies.
//!
//! ## Why Open Library, and nobody else
//!
//! Open Library's data is **CC0**: the Internet Archive "does not assert any
//! new copyright or other proprietary rights over any of the material in the
//! Open Library database". That is the whole argument. `epub-tailor` writes
//! metadata *into a file the user keeps*, and Google Books' terms forbid
//! creating "permanent copies" of their content - which is exactly what writing
//! a description into an EPUB is. CC0 has no such problem, and no attribution
//! obligation either.
//!
//! Open Library is also keyless, publishes its rate limits, and explicitly
//! welcomes "human-facing discovery and lookup services" while forbidding bulk
//! download. One book at a time, at the user's request, is precisely the
//! sanctioned case.
//!
//! ## The shape of the data
//!
//! Open Library splits a book across three records, which is why a full lookup
//! is two requests:
//!
//! - the **search** doc (`/search.json`) has title, authors, publisher, first
//!   publish year, subjects and ISBNs - enough to *choose* between candidates;
//! - the **work** (`/works/OL…W.json`) is the only place the **description**
//!   and the full subject list live;
//! - the **edition** has the publisher and the ISBNs for one specific printing.
//!
//! Series is genuinely sparse in Open Library and usually comes back empty. We
//! do not pretend otherwise.

use serde::{Deserialize, Serialize};

use crate::epub::model::Creator;
use crate::metadata::{MetadataDoc, OneOrMany};

/// One result of a metadata search: a complete record, plus what a UI needs to
/// show it and refer back to it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    /// A stable handle for this record, e.g. `openlibrary:OL262758W`. Hand it
    /// back to `metadata fetch` to get the full record.
    pub r#ref: String,
    /// Where it came from, for a UI to label and for the user to judge.
    pub source: String,
    /// The metadata itself - complete, so a picker needs no second round-trip
    /// and can let the user accept fields one at a time.
    pub metadata: MetadataDoc,
    /// A cover image URL, if Open Library has one. Not downloaded: cover art is
    /// not CC0 even though the metadata is, so fetching it is an explicit,
    /// separate opt-in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cover_url: Option<String>,
    /// How well this matched the query, 0.0 to 1.0. A hint for ordering, not a
    /// verdict.
    pub score: f32,
}

/// Open Library's `/search.json` response.
#[derive(Debug, Deserialize)]
struct SearchResponse {
    #[serde(default)]
    docs: Vec<SearchDoc>,
}

#[derive(Debug, Deserialize)]
struct SearchDoc {
    /// e.g. `/works/OL262758W`
    #[serde(default)]
    key: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    author_name: Vec<String>,
    #[serde(default)]
    publisher: Vec<String>,
    #[serde(default)]
    first_publish_year: Option<i64>,
    #[serde(default)]
    subject: Vec<String>,
    #[serde(default)]
    isbn: Vec<String>,
    #[serde(default)]
    cover_i: Option<i64>,
    // Open Library's `language` is deliberately not modelled: see the comment in
    // `parse_search`. Taking it would mean writing a MARC code into a field that
    // wants BCP47, from whichever edition happened to match.
}

/// Open Library's work record (`/works/OL…W.json`) - the only place the
/// description lives.
#[derive(Debug, Deserialize)]
struct WorkResponse {
    #[serde(default)]
    description: Option<Description>,
    #[serde(default)]
    subjects: Vec<String>,
    #[serde(default)]
    covers: Vec<i64>,
}

/// Open Library returns a description as a bare string on some records and as
/// `{"type": "/type/text", "value": "..."}` on others. Both are real; handle
/// both rather than dropping half the descriptions on the floor.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Description {
    Text(String),
    Typed { value: String },
}

impl Description {
    fn into_string(self) -> String {
        match self {
            Description::Text(s) => s,
            Description::Typed { value } => value,
        }
    }
}

/// The cover URL for an Open Library cover id.
fn cover_url(id: i64) -> String {
    format!("https://covers.openlibrary.org/b/id/{id}-L.jpg")
}

/// Turn an Open Library description into something fit for `<dc:description>`.
///
/// Their descriptions are Markdown-ish free text contributed by the public, and
/// they come with baggage:
///
/// - a trailing attribution block (`"...text.\r\n\r\n([source][1])"`);
/// - Markdown emphasis, which is not markup a reader will render here;
/// - **Markdown links, including outright spam.** A real record for *The
///   Hobbit* ends with `[**PDF**](https://chesserresources.com/...)`. Writing
///   that URL into someone's book is not on. Links are flattened to their text
///   and the target is dropped.
///
/// `dc:description` is plain text, so flattening is the correct thing to do
/// anyway; that it also defuses the spam is a happy accident.
fn clean_description(text: &str) -> String {
    let cut = text
        .split("\n----------")
        .next()
        .unwrap_or(text)
        .split("\r\n\r\n([source]")
        .next()
        .unwrap_or(text)
        .split("\n([source]")
        .next()
        .unwrap_or(text);

    let mut out = String::with_capacity(cut.len());
    let mut chars = cut.replace("\r\n", "\n").chars().collect::<Vec<_>>();
    chars.reverse(); // pop() from the back is cheap; we push back on below.
    let mut stack: Vec<char> = chars;
    while let Some(c) = stack.pop() {
        match c {
            // `[text](url)` and `[text][ref]` -> `text`
            '[' => {
                let mut label = String::new();
                let mut closed = false;
                while let Some(inner) = stack.pop() {
                    if inner == ']' {
                        closed = true;
                        break;
                    }
                    label.push(inner);
                }
                if !closed {
                    out.push('[');
                    out.push_str(&label);
                    continue;
                }
                // Swallow the target, if there is one.
                if let Some(open) = stack.last().copied()
                    && (open == '(' || open == '[')
                {
                    let close = if open == '(' { ')' } else { ']' };
                    stack.pop();
                    while let Some(inner) = stack.pop() {
                        if inner == close {
                            break;
                        }
                    }
                }
                out.push_str(label.trim_matches('*').trim_matches('_'));
            }
            // Emphasis markers are noise in plain text.
            '*' | '_' => {}
            other => out.push(other),
        }
    }

    // Collapse the runs of blank lines the stripping can leave behind.
    out.split('\n')
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

/// Score a candidate against the query it answered: how much of the asked-for
/// title and author actually came back. Case-insensitive containment, which is
/// crude but honest - it is a hint for ordering, and the user still chooses.
fn score(doc: &SearchDoc, want_title: Option<&str>, want_author: Option<&str>) -> f32 {
    let mut points = 0.0f32;
    let mut total = 0.0f32;

    if let Some(want) = want_title.map(str::to_lowercase).filter(|s| !s.is_empty()) {
        total += 1.0;
        if let Some(got) = doc.title.as_deref().map(str::to_lowercase) {
            if got == want {
                points += 1.0;
            } else if got.contains(&want) || want.contains(&got) {
                points += 0.6;
            }
        }
    }
    if let Some(want) = want_author.map(str::to_lowercase).filter(|s| !s.is_empty()) {
        total += 1.0;
        let matched = doc
            .author_name
            .iter()
            .any(|a| a.to_lowercase().contains(&want) || want.contains(&a.to_lowercase()));
        if matched {
            points += 1.0;
        }
    }

    if total == 0.0 { 0.5 } else { points / total }
}

/// Parse a `/search.json` body into candidates, best match first.
///
/// `want_title` / `want_author` are the query that produced it, used only for
/// scoring.
pub fn parse_search(
    body: &str,
    want_title: Option<&str>,
    want_author: Option<&str>,
) -> Result<Vec<Candidate>, String> {
    let response: SearchResponse = serde_json::from_str(body)
        .map_err(|e| format!("Open Library sent JSON we cannot read: {e}"))?;

    let mut candidates: Vec<Candidate> = response
        .docs
        .into_iter()
        .filter_map(|doc| {
            // A record with no work key cannot be fetched later, so it is no use
            // as a candidate.
            let key = doc.key.clone()?;
            let olid = key.rsplit('/').next()?.to_string();
            let score = score(&doc, want_title, want_author);

            let metadata = MetadataDoc {
                title: doc.title.clone(),
                authors: (!doc.author_name.is_empty())
                    .then(|| OneOrMany::Many(doc.author_name.iter().map(Creator::new).collect())),
                // Language is deliberately NOT taken from Open Library. It is a
                // per-*edition* field, so a search for an English book happily
                // returns the Italian printing, and it arrives as a 3-letter
                // MARC code (`ita`) where `dc:language` wants BCP47 (`it`).
                // The book already knows what language it is in; we would only
                // be able to make that worse.
                language: None,
                publisher: doc.publisher.first().cloned(),
                subjects: (!doc.subject.is_empty())
                    .then(|| OneOrMany::Many(doc.subject.iter().take(12).cloned().collect())),
                date: doc.first_publish_year.map(|y| y.to_string()),
                isbn: doc.isbn.first().cloned(),
                ..MetadataDoc::default()
            };

            Some(Candidate {
                r#ref: format!("openlibrary:{olid}"),
                source: "openlibrary".to_string(),
                metadata,
                cover_url: doc.cover_i.map(cover_url),
                score,
            })
        })
        .collect();

    candidates.sort_by(|a, b| b.score.total_cmp(&a.score));
    Ok(candidates)
}

/// Fold a work record into a candidate's metadata: the description and the full
/// subject list, which the search doc does not carry.
pub fn merge_work(
    body: &str,
    doc: &mut MetadataDoc,
    cover_url_out: &mut Option<String>,
) -> Result<(), String> {
    let work: WorkResponse = serde_json::from_str(body)
        .map_err(|e| format!("Open Library sent a work record we cannot read: {e}"))?;

    if let Some(description) = work.description {
        let text = clean_description(&description.into_string());
        if !text.is_empty() {
            doc.description = Some(text);
        }
    }
    if !work.subjects.is_empty() {
        doc.subjects = Some(OneOrMany::Many(
            work.subjects.into_iter().take(12).collect(),
        ));
    }
    if cover_url_out.is_none()
        && let Some(id) = work.covers.iter().find(|id| **id > 0)
    {
        *cover_url_out = Some(cover_url(*id));
    }
    Ok(())
}

/// The work OLID inside a `openlibrary:OL…W` reference.
pub fn olid_of(reference: &str) -> Option<&str> {
    reference
        .strip_prefix("openlibrary:")
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A trimmed but faithful `/search.json` body, in Open Library's real shape.
    const SEARCH: &str = r#"{
      "numFound": 2,
      "docs": [
        {
          "key": "/works/OL262758W",
          "title": "The Hobbit",
          "author_name": ["J. R. R. Tolkien"],
          "publisher": ["Allen & Unwin", "Houghton Mifflin"],
          "first_publish_year": 1937,
          "subject": ["Fantasy", "Adventure", "Middle-earth"],
          "isbn": ["9780261102217", "0261102214"],
          "cover_i": 14625765,
          "language": ["eng"]
        },
        {
          "key": "/works/OL999999W",
          "title": "Some Other Book",
          "author_name": ["Nobody At All"]
        }
      ]
    }"#;

    #[test]
    fn a_search_response_becomes_candidates() {
        let candidates = parse_search(SEARCH, Some("The Hobbit"), Some("Tolkien")).expect("parses");
        assert_eq!(candidates.len(), 2);

        let best = &candidates[0];
        assert_eq!(best.r#ref, "openlibrary:OL262758W");
        assert_eq!(best.source, "openlibrary");
        assert_eq!(best.metadata.title.as_deref(), Some("The Hobbit"));
        assert_eq!(best.metadata.publisher.as_deref(), Some("Allen & Unwin"));
        assert_eq!(best.metadata.date.as_deref(), Some("1937"));
        assert_eq!(best.metadata.isbn.as_deref(), Some("9780261102217"));
        assert_eq!(
            best.cover_url.as_deref(),
            Some("https://covers.openlibrary.org/b/id/14625765-L.jpg")
        );
    }

    #[test]
    fn the_better_match_sorts_first() {
        let candidates = parse_search(SEARCH, Some("The Hobbit"), Some("Tolkien")).expect("parses");
        assert_eq!(candidates[0].r#ref, "openlibrary:OL262758W");
        assert!(
            candidates[0].score > candidates[1].score,
            "the book we asked for should outrank the one we did not"
        );
    }

    #[test]
    fn a_description_parses_from_a_bare_string() {
        let mut doc = MetadataDoc::default();
        let mut cover = None;
        merge_work(
            r#"{"description": "In a hole in the ground there lived a hobbit."}"#,
            &mut doc,
            &mut cover,
        )
        .expect("parses");
        assert_eq!(
            doc.description.as_deref(),
            Some("In a hole in the ground there lived a hobbit.")
        );
    }

    #[test]
    fn a_description_parses_from_the_typed_object_too() {
        // Open Library uses both shapes in the wild. Dropping one halves the
        // descriptions we can fill.
        let mut doc = MetadataDoc::default();
        let mut cover = None;
        merge_work(
            r#"{"description": {"type": "/type/text", "value": "A typed blurb."},
                "subjects": ["Fantasy"], "covers": [42]}"#,
            &mut doc,
            &mut cover,
        )
        .expect("parses");
        assert_eq!(doc.description.as_deref(), Some("A typed blurb."));
        assert_eq!(
            cover.as_deref(),
            Some("https://covers.openlibrary.org/b/id/42-L.jpg")
        );
    }

    #[test]
    fn a_spam_link_in_a_description_does_not_end_up_in_the_book() {
        // This is not hypothetical. The live Open Library record for The Hobbit
        // ends with exactly this, and without flattening it we would write a
        // stranger's URL into the user's EPUB.
        let mut doc = MetadataDoc::default();
        let mut cover = None;
        merge_work(
            r#"{"description": "A reluctant hobbit. [**PDF**](https://chesserresources.com/doc/the-hobbit/)"}"#,
            &mut doc,
            &mut cover,
        )
        .expect("parses");
        let description = doc.description.expect("a description");
        assert!(
            !description.contains("chesserresources"),
            "the URL must not survive: {description}"
        );
        assert!(
            !description.contains("http"),
            "no link target at all: {description}"
        );
        assert!(description.starts_with("A reluctant hobbit."));
    }

    #[test]
    fn markdown_is_flattened_because_dc_description_is_plain_text() {
        let mut doc = MetadataDoc::default();
        let mut cover = None;
        merge_work(
            r#"{"description": "A *fine* book by [Tolkien](https://example.org), **truly**."}"#,
            &mut doc,
            &mut cover,
        )
        .expect("parses");
        assert_eq!(
            doc.description.as_deref(),
            Some("A fine book by Tolkien, truly.")
        );
    }

    #[test]
    fn language_is_never_taken_from_open_library() {
        // It is a per-edition field (an English search happily returns the
        // Italian printing) and it arrives as a MARC code, not BCP47.
        let candidates = parse_search(SEARCH, None, None).expect("parses");
        assert!(
            candidates[0].metadata.language.is_none(),
            "the book knows its own language better than a lookup does"
        );
    }

    #[test]
    fn the_source_footer_is_stripped_from_a_description() {
        let mut doc = MetadataDoc::default();
        let mut cover = None;
        merge_work(
            "{\"description\": \"The real blurb.\\r\\n\\r\\n([source][1])\\n\\n[1]: https://example.org\"}",
            &mut doc,
            &mut cover,
        )
        .expect("parses");
        assert_eq!(doc.description.as_deref(), Some("The real blurb."));
    }

    #[test]
    fn a_doc_with_no_work_key_is_not_offered() {
        // Without a key there is nothing to `fetch` later, so it would be a
        // candidate the user could not act on.
        let candidates =
            parse_search(r#"{"docs": [{"title": "Keyless"}]}"#, None, None).expect("parses");
        assert!(candidates.is_empty());
    }

    #[test]
    fn an_empty_result_set_is_not_an_error() {
        let candidates = parse_search(r#"{"numFound": 0, "docs": []}"#, Some("x"), None)
            .expect("an empty result is a valid answer");
        assert!(candidates.is_empty());
    }

    #[test]
    fn garbage_is_reported_not_panicked_on() {
        let err = parse_search("not json at all", None, None).expect_err("must not parse");
        assert!(err.contains("cannot read"), "got: {err}");
    }

    #[test]
    fn a_reference_yields_its_olid() {
        assert_eq!(olid_of("openlibrary:OL262758W"), Some("OL262758W"));
        assert_eq!(olid_of("googlebooks:xyz"), None);
        assert_eq!(olid_of("openlibrary:"), None);
    }
}
