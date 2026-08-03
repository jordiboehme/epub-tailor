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

/// Registrant codes no registration agency ever assigns to a real publisher:
/// `10.5555` is the DOI Foundation's own official *test* prefix, and
/// `10.0000`/`10.9999` are the all-zero/all-nine placeholders a value
/// invented on the spot reaches for. A real DOI never carries one, so these
/// mark the value as minted rather than registered.
///
/// This screens the *registrant*, not the suffix, deliberately: an earlier
/// round screened long digit runs in the suffix instead and dropped whole
/// families of real DOIs (every Zenodo, medRxiv, PNAS, Wiley, MDPI,
/// figshare, OECD and Taylor & Francis DOI tested), because real DOI
/// suffixes legitimately contain long digit runs - `10.5281/zenodo.10001234`,
/// `10.1073/pnas.2019897118`, `10.1787/9789264189515-en`. A digit-run
/// threshold provably cannot separate those from a watermark; the registrant
/// can, because a watermarker has to either use a placeholder prefix or
/// forge one belonging to somebody else.
const PLACEHOLDER_DOI_REGISTRANTS: &[&str] = &["0000", "5555", "9999"];

/// A DOI suffix that is *entirely* one digit run at least this long is a bare
/// numeric token - a millisecond timestamp (`1735689600000`) or a raw
/// transaction id - wearing a DOI's clothes. The "entirely" is what makes
/// this safe where a plain digit-run threshold was not: a structured suffix
/// like `zenodo.10001234`, `pnas.2019897118` or `9789264189515-en` carries
/// non-digit characters and is never caught, while a suffix with nothing but
/// digits and no bibliographic structure at all is. Twelve keeps the real
/// all-numeric suffixes that do exist (`10.2172/1234567890`, ten digits;
/// `10.1000/182`, three).
const MIN_BARE_NUMERIC_DOI_SUFFIX: usize = 12;

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
/// a DOI costume rather than a real, shared bibliographic identifier.
///
/// Every screen here is a *discriminator*: something that differs between a
/// real DOI and a minted one. A registrant too short to have been assigned,
/// or one of the reserved placeholder/test prefixes; a suffix carrying an
/// email address or an embedded UUID; a suffix that is nothing but a long
/// digit run. What is deliberately absent is any threshold on how many
/// digits a suffix contains - see [`PLACEHOLDER_DOI_REGISTRANTS`] for the
/// measured reason that direction does not work.
fn doi_looks_per_copy(registrant: &str, suffix: &str) -> bool {
    if registrant.len() < MIN_DOI_REGISTRANT_DIGITS {
        return true;
    }
    if PLACEHOLDER_DOI_REGISTRANTS.contains(&registrant) {
        return true;
    }
    if suffix.contains('@') {
        return true;
    }
    if suffix.to_ascii_lowercase().contains("urn:uuid:") || contains_uuid(suffix) {
        return true;
    }
    suffix.len() >= MIN_BARE_NUMERIC_DOI_SUFFIX && suffix.chars().all(|c| c.is_ascii_digit())
}

/// The per-copy screens a declared identifier scheme must never be able to
/// override: a value that positively *looks* minted per copy stays dropped
/// however the OPF labels it.
///
/// This exists because the watermarker controls the OPF. An `<meta
/// refines="#x" property="identifier-type">DOI</meta>` costs a shop one line,
/// and if a declared scheme were taken as a warrant of trustworthiness it
/// would make the whole screen trivially defeatable. A scheme declaration is
/// a hint about a value's *format*, not a promise about its provenance, so
/// the shapes below - a UUID, an email address, a DOI whose own parts betray
/// it - are decided on the value alone.
///
/// The `urn:uuid:` substring check is deliberately tried before
/// [`looks_like_uuid`]: a bare UUID with extra surrounding text (which
/// `looks_like_uuid`'s exact-shape test would miss) still contains
/// `urn:uuid:` when that is how it was minted, and short-circuiting there
/// avoids running the (slightly more expensive) shape scan at all.
fn looks_per_copy_regardless_of_scheme(value: &str) -> bool {
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
    matches!(doi_parts(core), Some((registrant, suffix)) if doi_looks_per_copy(registrant, suffix))
}

/// Whether `value` looks like it was minted per copy rather than per edition.
fn is_per_copy(value: &str) -> bool {
    if looks_per_copy_regardless_of_scheme(value) {
        return true;
    }
    let core = strip_scheme_prefix(value.trim());
    // A DOI-shaped value was already decided above, and never falls through
    // to the digit-count path below: that path's short-value leniency
    // (`digit_count < 8` reads as shared) exists for plain identifiers, not
    // for a value that specifically dressed itself up as
    // `10.<registrant>/<suffix>`. Having cleared `doi_looks_per_copy`, it is
    // kept - a real DOI's suffix is not checksummable and its digit count
    // says nothing.
    if doi_parts(core).is_some() {
        return false;
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

/// Whether an identifier is kept.
///
/// An explicitly declared shared scheme wins over the *heuristic* part of the
/// judgment, so an ISBN whose value happens not to checksum (a typo, an
/// unusual spelling) is never mistaken for a transaction number. It does not
/// win over [`looks_per_copy_regardless_of_scheme`]: a shop writes the OPF,
/// so a scheme declaration it controls cannot be allowed to launder a value
/// that positively looks minted per copy. Screening first and shortcutting
/// second is the strict direction - it can only drop more, never keep more.
fn is_shared(value: &str, scheme: Option<&str>) -> bool {
    if looks_per_copy_regardless_of_scheme(value) {
        return false;
    }
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
    fn a_doi_under_the_all_zero_placeholder_registrant_is_dropped() {
        // Measured regression: a shop can mint doi:10.<registrant>/<transaction
        // id> just as easily as a bare vendor id. `10.0000` was never
        // assigned to anyone, which is what gives this one away.
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
    fn the_official_doi_test_prefix_is_dropped_even_with_an_innocuous_suffix() {
        // `10.5555` is the DOI Foundation's own registered *test* prefix, so
        // the registrant alone decides this one - no UUID, no email, no long
        // digit run in the suffix to fall back on.
        assert!(is_per_copy("doi:10.5555/ordinary.suffix.2019"));
    }

    #[test]
    fn the_all_nine_placeholder_registrant_is_dropped() {
        assert!(is_per_copy("10.9999/anything"));
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
    fn a_bare_numeric_suffix_the_length_of_a_millisecond_timestamp_is_dropped() {
        // Real registrant, but the suffix is nothing but a 13-digit run: no
        // volume, no article stem, no year - a raw token, not a citation.
        assert!(is_per_copy("10.1016/1735689600000"));
    }

    // -- real DOIs: the fidelity regression a raw digit-run threshold caused.
    // -- Every value below is a real, published DOI that the digit-run screen
    // -- dropped; a real DOI suffix legitimately contains long digit runs, so
    // -- no threshold on run length can separate these from a watermark. ----

    /// Every real DOI here was measured as wrongly dropped under the old
    /// `MIN_SUSPICIOUS_DIGIT_RUN = 8` screen, one whole publisher family per
    /// entry: Zenodo, figshare, medRxiv, PNAS, Wiley, MDPI, Taylor & Francis,
    /// OECD, a legacy SICI-style Wiley DOI, an LWW dotted-date DOI, an OSTI
    /// all-numeric DOI - plus the two shapes already pinned as kept and the
    /// DOI Foundation's own documentation DOI.
    const REAL_DOIS: &[&str] = &[
        "10.5281/zenodo.10001234",
        "10.6084/m9.figshare.12345678",
        "10.1101/2020.03.15.20036145",
        "10.1073/pnas.2019897118",
        "10.1002/anie.201915678",
        "10.3390/s20051234",
        "10.1080/00220388.2019.1626832",
        "10.1787/9789264189515-en",
        "10.1002/(SICI)1097-0258(19980815)17:15<1661::AID-SIM968>3.0.CO;2-2",
        "10.1097/01.mlr.0000114908.90348.f9",
        "10.2172/1234567890",
        "10.1016/j.jbi.2020.103545",
        "doi:10.1016/j.jbi.2020.103545",
        "10.1371/journal.pone.0173664",
        "10.1000/182",
    ];

    #[test]
    fn every_real_doi_is_kept() {
        for doi in REAL_DOIS {
            assert!(!is_per_copy(doi), "a real DOI must be kept: {doi}");
        }
    }

    #[test]
    fn every_real_doi_is_kept_through_is_shared_with_no_scheme_declared() {
        // The screening path R3 added runs ahead of the scheme shortcut, so
        // it has to be checked against the real DOIs too: a value with no
        // scheme at all must reach `is_per_copy`'s verdict unchanged.
        for doi in REAL_DOIS {
            assert!(is_shared(doi, None), "a real DOI must be kept: {doi}");
        }
    }

    /// The values a shop can mint, each of which must stay dropped whatever
    /// the loosening above does.
    const PER_COPY_DOIS: &[&str] = &[
        "doi:10.0000/SHTX001.635962014",
        "10.0000/6f2a1e40-8c31-4b7e-9a55-1d0c2f9b7e31",
        "doi:10.5555/copy-6f2a1e40-8c31-4b7e-9a55-1d0c2f9b7e31",
        "urn:doi:10.9/1735689600000",
    ];

    #[test]
    fn every_doi_costumed_watermark_is_still_dropped() {
        for value in PER_COPY_DOIS {
            assert!(
                is_per_copy(value),
                "a per-copy value must be dropped: {value}"
            );
        }
    }

    // -- R3: a declared identifier-type must not launder a per-copy value ---

    #[test]
    fn a_declared_doi_scheme_does_not_rescue_a_doi_costumed_watermark() {
        // The whole point: the watermarker writes the OPF, so adding
        // `<meta refines="#x" property="identifier-type">DOI</meta>` costs it
        // one line. The same value must be dropped with and without it.
        for value in PER_COPY_DOIS {
            assert!(
                !is_shared(value, None),
                "must be dropped without a scheme: {value}"
            );
            assert!(
                !is_shared(value, Some("DOI")),
                "a declared scheme must not rescue it: {value}"
            );
        }
    }

    #[test]
    fn a_declared_scheme_does_not_rescue_a_bare_uuid_or_an_email() {
        for value in [
            "6f2a1e40-8c31-4b7e-9a55-1d0c2f9b7e31",
            "urn:uuid:6f2a1e40-8c31-4b7e-9a55-1d0c2f9b7e31",
            "buyer42@example.com",
        ] {
            for scheme in ["ISBN", "ISSN", "DOI"] {
                assert!(
                    !is_shared(value, Some(scheme)),
                    "a declared {scheme} must not rescue {value}"
                );
            }
        }
    }

    #[test]
    fn a_declared_isbn_scheme_still_protects_a_real_isbn() {
        // The strictness R3 adds must not cost the scheme shortcut its whole
        // reason for existing: a genuine ISBN behind a genuine refinement
        // stays kept.
        assert!(is_shared("9783407868213", Some("ISBN")));
        assert!(is_shared("urn:isbn:9783407868213", Some("ISBN")));
        assert!(is_shared("2049-3630", Some("ISSN")));
        assert!(is_shared("10.1016/j.jbi.2020.103545", Some("DOI")));
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

    // -- R3, end to end through `normalize()`: the refinement a shop can add
    // -- to the OPF in one line must not rescue the watermark ---------------

    fn book_with_secondary(value: &str, scheme: Option<&str>) -> Book {
        let mut book = book_with_identifier("9783407868213", Some("ISBN"));
        book.metadata.identifiers = vec![crate::epub::model::Identifier {
            value: value.to_string(),
            scheme: scheme.map(str::to_string),
        }];
        book
    }

    #[test]
    fn a_doi_type_refinement_does_not_save_the_watermark_on_either_identifier_path() {
        // The proven end-to-end defeat: the shop appends
        // `<meta refines="#vendor" property="identifier-type">DOI</meta>` and
        // the value that `is_per_copy` drops sails through. Both spellings -
        // no refinement and a DOI refinement - must end with nothing left.
        for scheme in [None, Some("DOI")] {
            let mut book = book_with_secondary("doi:10.0000/SHTX001.635962014", scheme);
            let mut transformations = Vec::new();
            normalize(&mut book, &mut transformations);
            assert!(
                book.metadata.identifiers.is_empty(),
                "a DOI-costumed watermark must be dropped with scheme {scheme:?}"
            );

            let mut book = book_with_identifier("doi:10.0000/SHTX001.635962014", scheme);
            let mut transformations = Vec::new();
            normalize(&mut book, &mut transformations);
            assert_eq!(
                book.metadata.identifier, None,
                "the same value as the *unique* identifier must also be dropped \
                 with scheme {scheme:?}"
            );
        }
    }

    #[test]
    fn an_isbn_type_refinement_still_keeps_a_real_isbn_through_normalize() {
        let mut book = book_with_secondary("978-3-407-86821-3", Some("ISBN"));
        let mut transformations = Vec::new();
        normalize(&mut book, &mut transformations);
        assert_eq!(
            book.metadata.identifiers.len(),
            1,
            "a genuine ISBN behind a genuine refinement must be kept: {transformations:?}"
        );
    }
}
