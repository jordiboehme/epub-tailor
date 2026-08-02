//! Per-copy identity in the package document.
//!
//! `dc:source` and arbitrary vendor `<meta>` need no handling: the writer
//! regenerates the OPF from a closed set of fields, so they never survive a
//! conversion in the first place. The extra `dc:identifier` list does survive,
//! and is the one metadata channel a shop can use to mark a copy.

use crate::epub::Book;
use crate::report::Transformation;

/// Identifier schemes that name an *edition*, not a copy: shared by every
/// buyer, so they converge and are kept.
const SHARED_SCHEMES: &[&str] = &["isbn", "issn", "doi"];

/// Whether `value` looks like it was minted per copy rather than per edition.
///
/// The `urn:uuid:` substring check is deliberately tried before
/// [`looks_like_uuid`]: a bare UUID with extra surrounding text (which
/// `looks_like_uuid`'s exact-shape test would miss) still contains
/// `urn:uuid:` when that is how it was minted, and short-circuiting there
/// avoids running the (slightly more expensive) shape scan at all.
fn is_per_copy(value: &str) -> bool {
    let v = value.trim();
    if v.contains('@') {
        return true; // an email address
    }
    if v.to_ascii_lowercase().contains("urn:uuid:") || looks_like_uuid(v) {
        return true;
    }
    // A long digit run only reads as per-copy if it is not a real, checksum-
    // valid ISBN/ISSN: a 13-digit millisecond timestamp has the same *length*
    // as an ISBN-13 but fails its check digit, and an ISBN-10 ending in `X`
    // has only 9 digits - shape alone (as opposed to a real checksum) gets
    // both of those wrong.
    let digit_count = v.chars().filter(char::is_ascii_digit).count();
    if digit_count < 8 {
        return false;
    }
    let stripped = strip_separators(v);
    !(is_valid_isbn13(&stripped) || is_valid_isbn10(&stripped) || is_valid_issn(&stripped))
}

fn looks_like_uuid(v: &str) -> bool {
    let hyphens = v.matches('-').count();
    hyphens == 4 && v.len() >= 36 && v.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

/// Drop hyphens and spaces, the only separators a real ISBN/ISSN is ever
/// printed with, before checksum validation.
fn strip_separators(v: &str) -> String {
    v.chars().filter(|c| *c != '-' && *c != ' ').collect()
}

/// A checksum character's numeric value: a digit is itself, `X`/`x` is 10 -
/// the ISBN-10/ISSN convention for a check value of 10. `None` for anything
/// else.
fn check_char_value(c: u8) -> Option<u32> {
    match c {
        b'0'..=b'9' => Some((c - b'0') as u32),
        b'X' | b'x' => Some(10),
        _ => None,
    }
}

/// ISBN-13: 13 digits, alternating weights 1,3,1,3,…; valid when the
/// weighted sum is a multiple of 10.
fn is_valid_isbn13(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() != 13 || !bytes.iter().all(u8::is_ascii_digit) {
        return false;
    }
    let sum: u32 = bytes
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let d = (b - b'0') as u32;
            if i % 2 == 0 { d } else { d * 3 }
        })
        .sum();
    sum.is_multiple_of(10)
}

/// ISBN-10: 9 digits plus a check character (a digit, or `X`/`x` for 10),
/// weights 10,9,…,1 left to right; valid when the weighted sum is a multiple
/// of 11.
fn is_valid_isbn10(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() != 10 || !bytes[..9].iter().all(u8::is_ascii_digit) {
        return false;
    }
    let Some(check) = check_char_value(bytes[9]) else {
        return false;
    };
    let sum: u32 = bytes[..9]
        .iter()
        .enumerate()
        .map(|(i, b)| (b - b'0') as u32 * (10 - i as u32))
        .sum::<u32>()
        + check;
    sum.is_multiple_of(11)
}

/// ISSN: 7 digits plus a check character (a digit, or `X`/`x` for 10),
/// weights 8,7,…,2 on the first seven; valid when the check character equals
/// `(11 - weighted_sum % 11) % 11`.
fn is_valid_issn(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() != 8 || !bytes[..7].iter().all(u8::is_ascii_digit) {
        return false;
    }
    let Some(check) = check_char_value(bytes[7]) else {
        return false;
    };
    let sum: u32 = bytes[..7]
        .iter()
        .enumerate()
        .map(|(i, b)| (b - b'0') as u32 * (8 - i as u32))
        .sum();
    check == (11 - sum % 11) % 11
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
pub(crate) fn normalize(book: &mut Book, transformations: &mut Vec<Transformation>) {
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
    // Cleared here; the writer's `synth_identifier` derives the deterministic
    // replacement from title and authors, which converges by construction.
    book.metadata.identifier = None;
    transformations.push(Transformation {
        kind: "generic-identity".to_string(),
        detail: "replaced a per-copy unique identifier with a derived one".to_string(),
        file: None,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hyphenated_isbn13_is_kept() {
        assert!(!is_per_copy("978-3-407-86821-3"));
    }

    #[test]
    fn an_isbn10_ending_in_x_is_kept() {
        assert!(!is_per_copy("080442957X"));
    }

    #[test]
    fn a_millisecond_timestamp_the_length_of_an_isbn13_is_dropped() {
        assert!(is_per_copy("1735689600000"));
    }

    #[test]
    fn a_vendor_transaction_id_is_dropped() {
        assert!(is_per_copy("SHTX001.635962014"));
    }

    #[test]
    fn a_bare_uuid_is_dropped() {
        assert!(is_per_copy("6f2a1e40-8c31-4b7e-9a55-1d0c2f9b7e31"));
    }

    #[test]
    fn a_urn_uuid_prefixed_value_is_dropped() {
        assert!(is_per_copy("urn:uuid:6f2a1e40-8c31-4b7e-9a55-1d0c2f9b7e31"));
    }

    #[test]
    fn an_email_address_is_dropped() {
        assert!(is_per_copy("buyer42@example.com"));
    }

    #[test]
    fn an_invalid_checksum_13_digit_number_is_dropped() {
        // Same shape as an ISBN-13 (13 digits) but the weighted sum is not a
        // multiple of 10.
        assert!(is_per_copy("1234567890123"));
    }

    #[test]
    fn a_short_numeric_id_under_the_digit_threshold_is_kept() {
        assert!(!is_per_copy("1234567"));
    }

    #[test]
    fn isbn13_checksum_accepts_a_real_isbn_and_rejects_a_timestamp() {
        assert!(is_valid_isbn13("9783407868213"));
        assert!(!is_valid_isbn13("1735689600000"));
    }

    #[test]
    fn isbn10_checksum_accepts_an_x_check_character() {
        assert!(is_valid_isbn10("080442957X"));
        assert!(is_valid_isbn10("080442957x"));
        assert!(!is_valid_isbn10("0804429571"));
    }

    #[test]
    fn issn_checksum_round_trips_a_known_issn() {
        // 2049-3630 is Wikidata's own ISSN, chosen only because its check
        // digit is easy to verify by hand: 2*8+0*7+4*6+9*5+3*4+6*3+3*2 = 16+
        // 0+24+45+12+18+6 = 121, 121 % 11 == 0, so the expected check is 0.
        assert!(is_valid_issn("20493630"));
        assert!(!is_valid_issn("20493631"));
    }

    #[test]
    fn a_scheme_shortcut_keeps_an_identifier_that_would_otherwise_look_per_copy() {
        // Fails every checksum (not 13/10/8 digits at all), but the scheme
        // says ISBN, and the scheme shortcut is unconditional.
        assert!(is_shared("not-a-real-isbn-shape", Some("ISBN")));
    }

    #[test]
    fn is_shared_drops_a_vendor_id_with_no_scheme() {
        assert!(!is_shared("SHTX001.635962014", None));
    }
}
