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

/// Leading scheme prefixes that mark a value as an already-typed ISBN/ISSN/
/// DOI, matched case-insensitively and stripped before validating. Every
/// prefix is tried and the value is only ever stripped once, so listing the
/// longest form (`urn:isbn:`) before its own suffix (`isbn:`) is not
/// required for correctness; the pairs are simply kept next to each other
/// for readability.
const SCHEME_PREFIXES: &[&str] = &[
    "urn:isbn:",
    "isbn:",
    "urn:issn:",
    "issn:",
    "urn:doi:",
    "doi:",
];

/// Strip a leading identifier-scheme prefix - `urn:isbn:`, `isbn:`,
/// `urn:issn:`, `issn:`, `urn:doi:`, `doi:`, or the bare-word `ISBN `/`ISSN `
/// spelling (`ISBN 978-3-407-86821-3`) - case-insensitively, so
/// `urn:isbn:9783407868213` (the canonical EPUB 3 spelling) checksums
/// exactly like the bare digits. Returns `value` unchanged when nothing
/// matches.
fn strip_scheme_prefix(value: &str) -> &str {
    let lower = value.to_ascii_lowercase();
    for prefix in SCHEME_PREFIXES {
        if lower.starts_with(prefix) {
            return value[prefix.len()..].trim_start();
        }
    }
    for word in ["isbn ", "issn "] {
        if lower.starts_with(word) {
            return value[word.len()..].trim_start();
        }
    }
    value
}

/// A DOI's registrant code must be at least four numerals (ISO 26324's own
/// minimum; every real-world registrant, `10.1016` for Elsevier, `10.1000`
/// reserved by the DOI Foundation itself for documentation, and so on, meets
/// it). A shorter registrant like `10.1` or `10.9` was never issued to
/// anyone, so it is not a real DOI shape at all - just digits that happen to
/// contain a `10.` and a `/`.
const MIN_DOI_REGISTRANT_DIGITS: usize = 4;

/// A contiguous run of digits this long inside a DOI suffix reads as a raw
/// numeric token (a timestamp, a transaction id) rather than the scattered
/// short digit groups an ordinary bibliographic DOI suffix carries (a real
/// example, `j.jbi.2020.103545`, has runs of at most 6). Matches the digit
/// threshold used elsewhere in this module for the same "this many digits in
/// a row stops looking like prose and starts looking like an id" judgment.
const MIN_SUSPICIOUS_DIGIT_RUN: usize = 8;

/// Split `value` into a DOI's registrant and suffix if it has the DOI shape
/// `10.<registrant>/<suffix>`: registrant all-digit and non-empty, suffix
/// non-empty. Purely a shape check (the DOI spec allows almost any character
/// in the suffix), so a shape match alone is not proof the DOI is real - see
/// [`is_per_copy`], which additionally screens the parts this returns before
/// trusting them. `value` is expected to already have any `doi:`/`urn:doi:`
/// prefix stripped.
fn doi_parts(value: &str) -> Option<(&str, &str)> {
    let rest = value.strip_prefix("10.")?;
    let (registrant, suffix) = rest.split_once('/')?;
    if !registrant.is_empty()
        && registrant.chars().all(|c| c.is_ascii_digit())
        && !suffix.is_empty()
    {
        Some((registrant, suffix))
    } else {
        None
    }
}

/// The longest run of consecutive ASCII digits anywhere in `s`.
fn longest_digit_run(s: &str) -> usize {
    let mut longest = 0;
    let mut current = 0;
    for c in s.chars() {
        if c.is_ascii_digit() {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    longest
}

/// Whether `s` contains a UUID anywhere inside it, not just as its entire
/// contents: [`looks_like_uuid`]'s exact-shape test misses a value like
/// `copy-<uuid>`, which still embeds a real UUID behind extra text.
/// Scans every 36-byte window (`.get` skips any that land off a char
/// boundary rather than panicking) for the hyphens-at-8/13/18/23 shape
/// [`looks_like_uuid`] already knows how to recognize.
fn contains_uuid(s: &str) -> bool {
    let bytes = s.len();
    if bytes < 36 {
        return false;
    }
    (0..=bytes - 36).any(|i| s.get(i..i + 36).is_some_and(looks_like_uuid))
}

/// Whether a DOI's registrant/suffix pair looks like a per-copy marker wearing
/// a DOI costume rather than a real, shared bibliographic identifier. A DOI
/// shape is trusted only when it clears every one of these; anything else -
/// an implausible registrant, an embedded UUID, an email address, or a raw
/// long digit run in the suffix - is treated as per-copy outright rather than
/// falling through to heuristics built for entirely different value shapes
/// (which, for a DOI's inherently non-numeric-checksummable suffix, would
/// misjudge it either way - see the module's callers for why this cannot
/// just defer to the generic digit-count path).
fn doi_looks_per_copy(registrant: &str, suffix: &str) -> bool {
    if registrant.len() < MIN_DOI_REGISTRANT_DIGITS {
        return true;
    }
    if suffix.contains('@') {
        return true;
    }
    if suffix.to_ascii_lowercase().contains("urn:uuid:") || contains_uuid(suffix) {
        return true;
    }
    longest_digit_run(suffix) >= MIN_SUSPICIOUS_DIGIT_RUN
}

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
    // A scheme prefix (`urn:isbn:`, `doi:`, the bare `ISBN ` word, ...) is
    // stripped before every check below, so a correctly-prefixed identifier
    // checksums (or shape-matches, for a DOI) exactly like its bare form.
    let core = strip_scheme_prefix(v);
    // A DOI-shaped value is decided here outright, never falling through to
    // the digit-count path below: that path's short-value leniency (`digit_
    // count < 8` reads as shared) exists for plain identifiers, not for a
    // value that specifically dressed itself up as `10.<registrant>/<suffix>`
    // - once something goes to the trouble of wearing that shape, it is
    // either a real DOI or a per-copy marker in costume, and `doi_looks_per_
    // copy` is what tells the two apart.
    if let Some((registrant, suffix)) = doi_parts(core) {
        return doi_looks_per_copy(registrant, suffix);
    }
    // A long digit run only reads as per-copy if it is not a real, checksum-
    // valid ISBN/ISSN: a 13-digit millisecond timestamp has the same *length*
    // as an ISBN-13 but fails its check digit, and an ISBN-10 ending in `X`
    // has only 9 digits - shape alone (as opposed to a real checksum) gets
    // both of those wrong.
    let digit_count = core.chars().filter(char::is_ascii_digit).count();
    if digit_count < 8 {
        return false;
    }
    let stripped = strip_separators(core);
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

/// ISBN-13: 13 digits under the `978`/`979` GS1 Bookland prefix, alternating
/// weights 1,3,1,3,…; valid when the weighted sum is a multiple of 10. The
/// prefix check matters: without it, roughly 1 in 10 arbitrary 13-digit
/// numbers (a vendor transaction id, say) passes the checksum alone by pure
/// chance.
fn is_valid_isbn13(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() != 13 || !bytes.iter().all(u8::is_ascii_digit) {
        return false;
    }
    if !(s.starts_with("978") || s.starts_with("979")) {
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

    // Uses `is_shared`, not `is_per_copy` directly, so the unique identifier
    // gets the same scheme shortcut a secondary identifier already gets: an
    // explicit `<meta refines="#pub-id" property="identifier-type">ISBN</meta>`
    // on the *unique* identifier must protect it exactly as it would a
    // secondary one, not just when the value also happens to checksum. No
    // identifier at all is, as before, nothing to replace.
    let unique_is_per_copy = book
        .metadata
        .identifier
        .as_deref()
        .is_some_and(|v| !is_shared(v, book.metadata.identifier_scheme.as_deref()));
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
    use crate::epub::Metadata;

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
    fn isbn13_requires_the_gs1_bookland_prefix() {
        // 13 digits with a genuinely valid mod-10 checksum, but under the 977
        // (ISSN-derived) prefix rather than 978/979 - without the prefix
        // check, roughly 1 in 10 arbitrary 13-digit numbers would false-
        // accept this way.
        assert!(!is_valid_isbn13("9771234567898"));
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

    // -- scheme-prefixed and DOI spellings, every one measured as wrongly
    // -- REPLACED before the prefix/DOI fix (see the module's callers) -----

    #[test]
    fn a_bare_isbn13_is_kept() {
        assert!(!is_per_copy("9783407868213"));
    }

    #[test]
    fn a_urn_isbn_prefixed_isbn13_is_kept() {
        assert!(!is_per_copy("urn:isbn:9783407868213"));
    }

    #[test]
    fn an_upper_case_urn_isbn_prefix_is_kept() {
        assert!(!is_per_copy("URN:ISBN:9783407868213"));
    }

    #[test]
    fn a_bare_isbn_scheme_prefix_is_kept() {
        assert!(!is_per_copy("isbn:9783407868213"));
    }

    #[test]
    fn an_isbn_word_prefix_with_a_hyphenated_value_is_kept() {
        assert!(!is_per_copy("ISBN 978-3-407-86821-3"));
    }

    #[test]
    fn a_urn_isbn_prefixed_isbn10_ending_in_x_is_kept() {
        assert!(!is_per_copy("urn:isbn:080442957X"));
    }

    #[test]
    fn a_urn_issn_prefixed_issn_is_kept() {
        assert!(!is_per_copy("urn:issn:2049-3630"));
    }

    #[test]
    fn a_bare_doi_is_kept() {
        assert!(!is_per_copy("10.1016/j.jbi.2020.103545"));
    }

    #[test]
    fn a_doi_scheme_prefixed_doi_is_kept() {
        assert!(!is_per_copy("doi:10.1016/j.jbi.2020.103545"));
    }

    #[test]
    fn a_urn_doi_prefixed_doi_is_kept() {
        assert!(!is_per_copy("urn:doi:10.1016/j.jbi.2020.103545"));
    }

    #[test]
    fn a_value_shaped_like_the_start_of_a_doi_but_missing_the_slash_is_not_mistaken_for_one() {
        // Starts like a DOI (`10.`) but never has the `/` that separates
        // registrant from suffix, so `doi_parts` must not match it; it
        // falls through to the checksum path instead and (correctly, since
        // it is not a real ISBN/ISSN either) reads as per-copy.
        assert!(is_per_copy("10.123456789"));
    }

    // -- a DOI shape is not automatically trusted: a per-copy marker wearing
    // -- a DOI costume must still be dropped ------------------------------

    #[test]
    fn a_doi_whose_suffix_is_a_long_digit_run_is_dropped() {
        // Measured regression: a shop can mint doi:10.<real-looking-
        // registrant>/<transaction id> just as easily as a bare vendor id.
        assert!(is_per_copy("doi:10.0000/SHTX001.635962014"));
    }

    #[test]
    fn a_doi_whose_suffix_is_a_bare_uuid_is_dropped() {
        assert!(is_per_copy("10.0000/6f2a1e40-8c31-4b7e-9a55-1d0c2f9b7e31"));
    }

    #[test]
    fn a_doi_whose_suffix_embeds_a_uuid_behind_other_text_is_dropped() {
        // The UUID is not the whole suffix here (`copy-` precedes it), which
        // is exactly the shape `looks_like_uuid`'s exact-match alone would
        // miss - `contains_uuid`'s substring scan is what has to catch it.
        assert!(is_per_copy(
            "doi:10.5555/copy-6f2a1e40-8c31-4b7e-9a55-1d0c2f9b7e31"
        ));
    }

    #[test]
    fn a_doi_whose_suffix_is_a_millisecond_timestamp_is_dropped() {
        assert!(is_per_copy("urn:doi:10.9/1735689600000"));
    }

    #[test]
    fn a_doi_with_an_implausibly_short_registrant_is_dropped() {
        // No real DOI registrant is a single digit (ISO 26324 requires at
        // least four numerals); this is just digits that happen to contain
        // "10." and "/", not a real DOI shape worth trusting.
        assert!(is_per_copy("10.1/x"));
    }

    #[test]
    fn a_doi_whose_suffix_embeds_an_email_address_is_dropped() {
        // Caught by is_per_copy's pre-existing top-level `@` guard before the
        // DOI-shape logic ever runs (the value contains '@', full stop) -
        // doi_looks_per_copy's own `@` check is unreachable through this
        // call site for exactly that reason, and is kept only as defense in
        // depth per the finding. This test pins the observable outcome
        // (dropped), not which internal guard produces it.
        assert!(is_per_copy("doi:10.1016/buyer@example.com"));
    }

    #[test]
    fn a_real_doi_with_short_scattered_digit_groups_in_its_suffix_stays_kept() {
        // The legitimate DOI already pinned elsewhere as KEPT, re-asserted
        // here to make the contrast with the long-digit-run cases above
        // explicit: "2020" and "103545" are short digit groups scattered
        // through ordinary bibliographic text, never a single run of 8+.
        assert!(!is_per_copy("10.1016/j.jbi.2020.103545"));
    }

    // -- the still-dropped per-copy cases, re-pinned after the prefix/DOI
    // -- fix touched the same function ---------------------------------

    #[test]
    fn a_millisecond_timestamp_is_still_dropped_after_the_prefix_fix() {
        assert!(is_per_copy("1735689600000"));
    }

    #[test]
    fn a_vendor_transaction_id_is_still_dropped_after_the_prefix_fix() {
        assert!(is_per_copy("SHTX001.635962014"));
    }

    #[test]
    fn a_bare_uuid_is_still_dropped_after_the_prefix_fix() {
        assert!(is_per_copy("6f2a1e40-8c31-4b7e-9a55-1d0c2f9b7e31"));
    }

    #[test]
    fn an_email_address_is_still_dropped_after_the_prefix_fix() {
        assert!(is_per_copy("buyer42@example.com"));
    }

    // -- the unique-identifier path must agree with the secondary path -----

    fn book_with_identifier(identifier: &str, scheme: Option<&str>) -> Book {
        Book {
            metadata: Metadata {
                identifier: Some(identifier.to_string()),
                identifier_scheme: scheme.map(str::to_string),
                ..Metadata::default()
            },
            resources: indexmap::IndexMap::new(),
            spine: Vec::new(),
            toc: Vec::new(),
            cover: None,
            opf_path: "content.opf".to_string(),
            nav_path: None,
            ncx_path: None,
        }
    }

    #[test]
    fn an_explicit_identifier_type_refinement_protects_the_unique_identifier_too() {
        // 13 digits that fail the ISBN-13 checksum outright (same value the
        // secondary-identifier path already pins in
        // `an_invalid_checksum_13_digit_number_is_dropped`), exercised
        // through `normalize()` on the *unique* identifier instead - the
        // exact disagreement the two-paths bug produced: the shape heuristic
        // alone reads this as per-copy, but an explicit `identifier-type`
        // refinement must protect it here exactly as it protects a
        // secondary identifier carrying the same value and scheme.
        let mut book = book_with_identifier("1234567890123", Some("ISBN"));
        let mut transformations = Vec::new();
        normalize(&mut book, &mut transformations);
        assert_eq!(book.metadata.identifier.as_deref(), Some("1234567890123"));
        assert!(
            transformations.is_empty(),
            "a scheme-protected unique identifier must not be replaced, got: {transformations:?}"
        );
    }

    #[test]
    fn a_per_copy_unique_identifier_with_no_scheme_is_still_replaced() {
        let mut book = book_with_identifier("SHTX001.635962014", None);
        let mut transformations = Vec::new();
        normalize(&mut book, &mut transformations);
        assert_eq!(book.metadata.identifier, None);
        assert_eq!(transformations.len(), 1);
    }
}
