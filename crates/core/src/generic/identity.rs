//! Per-copy identity in the package document.
//!
//! `dc:source` and arbitrary vendor `<meta>` need no handling: the writer
//! regenerates the OPF from a closed set of fields, so they never survive a
//! conversion in the first place. The extra `dc:identifier` list does survive,
//! and is the one metadata channel a shop can use to mark a copy.

use crate::epub::Book;
use crate::report::{Transformation, Warning};

/// Identifier schemes that name an *edition*, not a copy: shared by every
/// buyer, so they converge and are kept.
const SHARED_SCHEMES: &[&str] = &["isbn", "issn", "doi"];

/// Whether `value` looks like it was minted per copy rather than per edition.
fn is_per_copy(value: &str) -> bool {
    let v = value.trim();
    if v.contains('@') {
        return true; // an email address
    }
    if v.to_ascii_lowercase().contains("urn:uuid:") || looks_like_uuid(v) {
        return true;
    }
    // A long digit run that is not an ISBN-10/13 or ISSN length.
    let digits: String = v.chars().filter(char::is_ascii_digit).collect();
    digits.len() >= 8 && !matches!(digits.len(), 8 | 10 | 13)
}

fn looks_like_uuid(v: &str) -> bool {
    let hyphens = v.matches('-').count();
    hyphens == 4 && v.len() >= 36 && v.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

/// Whether an identifier is kept: an explicitly shared scheme always wins over
/// the shape heuristic, so an ISBN is never mistaken for a transaction number.
fn is_shared(value: &str, scheme: Option<&str>) -> bool {
    if let Some(s) = scheme
        && SHARED_SCHEMES.contains(&s.to_ascii_lowercase().as_str())
    {
        return true;
    }
    !is_per_copy(value)
}

/// Drop per-copy identifiers and, when the book's own unique identifier is one,
/// replace it with a value derived from title and authors.
///
/// This deliberately overrides the invariant documented on
/// `Metadata::identifier`: a reading system keys its library and reading
/// position off that value, so replacing it orphans bookmarks. Under an
/// explicit `generic` profile that is the intended trade - a per-copy
/// identifier *is* the watermark - but it is why the pass is opt-in.
///
/// `has_obfuscated_fonts` suppresses the replacement: EPUB font obfuscation
/// derives its key from the unique identifier, so rewriting it silently
/// corrupts every embedded font.
pub(crate) fn normalize(
    book: &mut Book,
    has_obfuscated_fonts: bool,
    transformations: &mut Vec<Transformation>,
    warnings: &mut Vec<Warning>,
) {
    let before = book.metadata.identifiers.len();
    book.metadata
        .identifiers
        .retain(|id| is_shared(&id.value, id.scheme.as_deref()));
    let dropped = before - book.metadata.identifiers.len();
    if dropped > 0 {
        transformations.push(Transformation {
            kind: "generic-identity".to_string(),
            detail: format!("dropped {dropped} per-copy identifier(s)"),
            file: None,
        });
    }

    let unique_is_per_copy = book.metadata.identifier.as_deref().is_some_and(is_per_copy);
    if !unique_is_per_copy {
        return;
    }
    if has_obfuscated_fonts {
        warnings.push(Warning {
            message: "the unique identifier looks per-copy but font obfuscation \
                      derives its key from it; left unchanged so the embedded \
                      fonts keep working"
                .to_string(),
            file: None,
        });
        return;
    }
    // Cleared here; the writer's `synth_identifier` derives the deterministic
    // replacement from title and authors, which converges by construction.
    book.metadata.identifier = None;
    transformations.push(Transformation {
        kind: "generic-identity".to_string(),
        detail: "replaced a per-copy unique identifier with a derived one".to_string(),
        file: None,
    });
}
