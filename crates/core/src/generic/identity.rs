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

/// How many check positions `v` holds once its separators are stripped, and
/// whether the last of them is an `X`/`x` check character: `Some((13, false))`
/// for `9783407868213`, `Some((10, true))` for `080442957X`, `Some((8, _))`
/// for an ISSN. `None` as soon as any position is neither a digit nor a
/// trailing `X`, which is what makes this a *shape* test rather than a
/// checksum: it answers "could this be an ISBN/ISSN at all", not "is this
/// one valid".
fn check_digit_positions(v: &str) -> Option<(usize, bool)> {
    let stripped = strip_separators(v);
    let (last, rest) = stripped.as_bytes().split_last()?;
    let trailing_x = matches!(last, b'X' | b'x');
    if rest.iter().all(u8::is_ascii_digit) && (trailing_x || last.is_ascii_digit()) {
        Some((stripped.len(), trailing_x))
    } else {
        None
    }
}

/// Whether `value` is plausibly shaped like the type an `identifier-type`
/// refinement declares it to be.
///
/// The scheme shortcut in [`is_shared`] exists to rescue a *real* identifier
/// whose checksum fails - a typo, an odd spelling - so it must not be able to
/// vouch for a value that is not even shaped like the type it claims. Without
/// this gate, one OPF line the watermarking shop writes itself (`<meta
/// refines="#pub-id" property="identifier-type">ISBN</meta>`) launders any
/// per-copy value at all. That was measured end to end: `SHTX001.635962014`
/// survived a `generic` conversion under an ISBN refinement, and two copies of
/// one edition differing only in that value stopped converging.
///
/// ISBN-13 additionally requires the GS1 Bookland prefix, the same condition
/// [`is_valid_isbn13`] imposes. Every real ISBN-13 lives in that range, so a
/// 13-digit value outside it is not a mis-typed ISBN but something else
/// wearing the label - a millisecond timestamp (`1735689600000`) above all.
/// Requiring it here keeps the rescue path and the validation path agreeing on
/// what "ISBN-13 shaped" means rather than letting them diverge.
fn matches_declared_scheme_shape(value: &str, scheme: &str) -> bool {
    let core = strip_scheme_prefix(value.trim());
    match scheme {
        "isbn" => match check_digit_positions(core) {
            // Nine digits plus a check character, which is legitimately `X`.
            Some((10, _)) => true,
            // Thirteen digits under 978/979. The 13 form's check digit is
            // mod 10, so it is always 0-9 and never `X`.
            Some((13, false)) => {
                let stripped = strip_separators(core);
                stripped.starts_with("978") || stripped.starts_with("979")
            }
            _ => false,
        },
        "issn" => matches!(check_digit_positions(core), Some((8, _))),
        "doi" => doi_parts(core).is_some(),
        _ => false,
    }
}

/// The prefix of the identifier the writer synthesizes for a book that has
/// none (`synth_identifier`, in `epub::write`). Exempt from per-copy screening
/// for a reason stronger than convenience: the value is an FNV-1a hash of
/// title and authors, so two copies of the same edition derive the *same*
/// string by construction - it is the one identifier shape that provably
/// carries no per-copy information.
///
/// Without the exemption the shape trips the digit-run screen below: the hash
/// is 16 hex characters, ~10 of which are digits on average, so a clear
/// majority of these clear `digit_count >= 8` and read as per-copy. Measured
/// over ten real titles, 8 of 10 did. `normalize` dropping one is invisible
/// (the writer re-derives the identical value), but a *diagnostic* built on
/// the same screen would tell the user that a book epub-tailor itself just
/// cleaned is watermarked.
const SYNTHESIZED_IDENTIFIER_PREFIX: &str = "urn:epub-tailor:";

/// What kind of identifier a value is, for both the drop decision and the
/// diagnostic that explains it.
///
/// [`is_shared`] is defined in terms of this, so the destructive pass and
/// `check`'s reporting cannot disagree about what counts as a watermark - the
/// failure mode being a `check` that stays silent about an identifier
/// `--profile generic` then drops, or shouts about one it keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PerCopyKind {
    /// Kept: an edition identifier, or one that provably converges.
    Shared,
    /// A bare UUID. The EPUB 3 default unique identifier, minted per *build*
    /// far more often than per copy - so ubiquitous that reporting it as a
    /// watermark would fire on nearly every book and drown every real signal.
    Uuid,
    /// A shape that exists only to name one buyer: an email address, or a DOI
    /// whose own registrant/suffix betray it.
    Distinctive,
    /// A long digit run that is not a checksum-valid ISBN/ISSN - a vendor
    /// transaction id, a millisecond timestamp, a build hash. Suggestive of a
    /// watermark, never proof of one.
    Opaque,
}

/// Classify an identifier: the single judgment [`is_shared`] and `check`'s
/// `watermark-identifier` finding are both derived from.
///
/// An explicitly declared shared scheme wins over the *heuristic* part of the
/// judgment, so an ISBN whose value happens not to checksum (a typo, an
/// unusual spelling) is never mistaken for a transaction number. That shortcut
/// is gated twice, because the shop writes the OPF and so controls everything
/// declared in it:
///
/// - it never overrides [`looks_per_copy_regardless_of_scheme`], so a value
///   that positively looks minted per copy stays dropped however it is
///   labelled;
/// - it only applies where [`matches_declared_scheme_shape`] agrees the value
///   is shaped like the declared type, so a declaration cannot vouch for a
///   value that is not even the right shape.
///
/// Anything the shortcut declines falls through to the normal per-copy
/// screening, exactly as if no scheme had been declared. Both gates are the
/// strict direction - they can only drop more, never keep more.
pub(crate) fn classify(value: &str, scheme: Option<&str>) -> PerCopyKind {
    let v = value.trim();
    if v.to_ascii_lowercase()
        .starts_with(SYNTHESIZED_IDENTIFIER_PREFIX)
    {
        return PerCopyKind::Shared;
    }
    // Split `looks_per_copy_regardless_of_scheme`'s verdict into its reasons.
    // The order mirrors that function exactly - an email is decided before the
    // UUID shapes, which are decided before the DOI screen - so the two agree
    // on every value by construction rather than by inspection.
    if v.contains('@') {
        return PerCopyKind::Distinctive;
    }
    if v.to_ascii_lowercase().contains("urn:uuid:") || looks_like_uuid(v) {
        return PerCopyKind::Uuid;
    }
    let core = strip_scheme_prefix(v);
    if matches!(doi_parts(core), Some((r, s)) if doi_looks_per_copy(r, s)) {
        return PerCopyKind::Distinctive;
    }

    if let Some(s) = scheme {
        let declared = s.to_ascii_lowercase();
        if SHARED_SCHEMES.contains(&declared.as_str())
            && matches_declared_scheme_shape(v, &declared)
        {
            return PerCopyKind::Shared;
        }
    }
    if is_per_copy(v) {
        PerCopyKind::Opaque
    } else {
        PerCopyKind::Shared
    }
}

/// Whether an identifier is kept. See [`classify`] for the judgment itself.
fn is_shared(value: &str, scheme: Option<&str>) -> bool {
    matches!(classify(value, scheme), PerCopyKind::Shared)
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
    fn a_value_with_no_digits_at_all_is_kept_whatever_its_declared_scheme() {
        // Named for what it actually exercises. This value does NOT reach the
        // scheme shortcut any more: it is not ISBN-shaped, so
        // `matches_declared_scheme_shape` declines it and it falls through to
        // `is_per_copy`, which keeps it because it carries no digits at all
        // (under the 8-digit threshold). The declared scheme is therefore not
        // what saves it - the outcome is identical with no scheme, which is
        // asserted here so the test cannot be misread as proving the shortcut
        // fired. For the shortcut actually firing, see
        // `a_declared_isbn_scheme_rescues_an_isbn13_shaped_value_that_fails_its_checksum`.
        assert!(is_shared("not-a-real-isbn-shape", Some("ISBN")));
        assert!(is_shared("not-a-real-isbn-shape", None));
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

    // -- R5: the scheme shortcut is gated on the value being SHAPED like the
    // -- type it declares. Screening the three shapes of
    // -- `looks_per_copy_regardless_of_scheme` was not enough: every OTHER
    // -- per-copy shape was still laundered by one OPF line, measured end to
    // -- end (a vendor id survived a `generic` conversion under an ISBN
    // -- refinement and two copies of one edition stopped converging). ------

    /// Every spelling of the refinement a shop can add for free. Both cases
    /// are listed because the value is lower-cased before matching and the
    /// EPUB spec does not fix a case for it.
    const DECLARED_SCHEMES: &[Option<&str>] = &[
        None,
        Some("ISBN"),
        Some("ISSN"),
        Some("DOI"),
        Some("isbn"),
        Some("doi"),
    ];

    #[test]
    fn no_declared_scheme_rescues_any_per_copy_shape() {
        for value in [
            "SHTX001.635962014",
            "1735689600000",
            "6f2a1e40-8c31-4b7e-9a55-1d0c2f9b7e31",
            "urn:uuid:6f2a1e40-8c31-4b7e-9a55-1d0c2f9b7e31",
            "buyer42@example.com",
        ] {
            for scheme in DECLARED_SCHEMES {
                assert!(
                    !is_shared(value, *scheme),
                    "a declared {scheme:?} must not rescue {value}"
                );
            }
        }
    }

    #[test]
    fn a_declared_isbn_scheme_rescues_an_isbn13_shaped_value_that_fails_its_checksum() {
        // The reason the shortcut exists, and the one case that must keep
        // working: `9783407868214` is a real edition's ISBN with its check
        // digit mistyped (`...213` is the valid one). It is Bookland-prefixed
        // and 13 digits, so it is genuinely ISBN-13 shaped - a plausible typo
        // rather than a value in costume - and the refinement rescues it.
        assert!(is_shared("9783407868214", Some("ISBN")));
        assert!(is_shared("978-3-407-86821-4", Some("ISBN")));
        // Without the refinement the checksum decides, and it fails.
        assert!(!is_shared("9783407868214", None));
    }

    #[test]
    fn a_thirteen_digit_value_outside_the_bookland_range_is_not_isbn_shaped() {
        // The distinction the rescue case turns on. Both values are 13
        // digits and both fail the ISBN-13 checksum; only the Bookland
        // prefix separates a mis-typed ISBN from a timestamp or a
        // transaction number, so the shape gate requires it.
        assert!(matches_declared_scheme_shape("9783407868214", "isbn"));
        assert!(!matches_declared_scheme_shape("1735689600000", "isbn"));
        assert!(!matches_declared_scheme_shape("1234567890123", "isbn"));
        // 12 digits behind letters and a dot: not a digit string at all.
        assert!(!matches_declared_scheme_shape("SHTX001.635962014", "isbn"));
    }

    #[test]
    fn scheme_shapes_are_checked_against_the_declared_type_only() {
        // An ISSN-shaped value is not ISBN-shaped and vice versa, so a shop
        // cannot pick whichever label happens to fit.
        assert!(matches_declared_scheme_shape("2049-3630", "issn"));
        assert!(!matches_declared_scheme_shape("2049-3630", "isbn"));
        assert!(matches_declared_scheme_shape("080442957X", "isbn"));
        assert!(!matches_declared_scheme_shape("080442957X", "issn"));
        assert!(matches_declared_scheme_shape(
            "10.1016/j.jbi.2020.103545",
            "doi"
        ));
        assert!(!matches_declared_scheme_shape(
            "10.1016/j.jbi.2020.103545",
            "isbn"
        ));
        // A DOI-shaped value under a DOI refinement still has to clear
        // `looks_per_copy_regardless_of_scheme` first, which is why shape
        // agreement alone never decides the outcome.
        assert!(matches_declared_scheme_shape("10.0000/TXN-A", "doi"));
        assert!(!is_shared("10.0000/TXN-A", Some("DOI")));
    }

    #[test]
    fn a_scheme_prefixed_value_is_shape_checked_on_its_bare_form() {
        // The prefix is stripped before the shape test, so the canonical
        // EPUB 3 spelling shapes exactly like the bare digits.
        assert!(matches_declared_scheme_shape(
            "urn:isbn:9783407868214",
            "isbn"
        ));
        assert!(matches_declared_scheme_shape(
            "isbn:978-3-407-86821-4",
            "isbn"
        ));
        assert!(matches_declared_scheme_shape("urn:issn:2049-3630", "issn"));
        assert!(matches_declared_scheme_shape(
            "doi:10.1016/j.jbi.2020.103545",
            "doi"
        ));
    }

    #[test]
    fn every_real_identifier_is_kept_with_and_without_its_refinement() {
        // A refinement must never *cost* a real identifier its place either:
        // the gate only decides whether the shortcut applies, and a value it
        // declines still reaches the ordinary screening on its own merits.
        for (value, scheme) in [
            ("9783407868213", "ISBN"),
            ("978-3-407-86821-3", "ISBN"),
            ("080442957X", "ISBN"),
            ("2049-3630", "ISSN"),
            ("10.1016/j.jbi.2020.103545", "DOI"),
            ("doi:10.1016/j.jbi.2020.103545", "DOI"),
        ] {
            assert!(
                is_shared(value, None),
                "must be kept with no scheme: {value}"
            );
            assert!(
                is_shared(value, Some(scheme)),
                "must be kept under a declared {scheme}: {value}"
            );
        }
    }

    #[test]
    fn every_real_doi_is_kept_under_a_declared_doi_refinement() {
        // The R2 loosening must survive R5's tightening: the shape gate
        // accepts every real DOI, so none of them loses the shortcut.
        for doi in REAL_DOIS {
            assert!(
                is_shared(doi, Some("DOI")),
                "a real DOI must be kept under a DOI refinement: {doi}"
            );
        }
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
        // A Bookland-prefixed 13-digit value that fails the ISBN-13 checksum
        // (the real ISBN ends `...213`), exercised through `normalize()` on
        // the *unique* identifier - the exact disagreement the two-paths bug
        // produced: the checksum alone reads this as per-copy, but an explicit
        // `identifier-type` refinement must protect it here exactly as it
        // protects a secondary identifier carrying the same value and scheme.
        //
        // Re-pinned from `1234567890123`, which this test used to carry. That
        // value was never a plausible mis-typed ISBN - no real ISBN-13 exists
        // outside the 978/979 Bookland range - so it pinned the wrong thing:
        // it was indistinguishable from the millisecond timestamp
        // `1735689600000` that the scheme shortcut must NOT rescue, and no
        // shape test can separate the two. See
        // `matches_declared_scheme_shape`.
        let mut book = book_with_identifier("9783407868214", Some("ISBN"));
        let mut transformations = Vec::new();
        normalize(&mut book, &mut transformations);
        assert_eq!(book.metadata.identifier.as_deref(), Some("9783407868214"));
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

    // -- classify: the judgment `check` reports and `normalize` acts on ----

    /// Every value the rest of this module pins a verdict for, so the drift
    /// check below covers the same ground the drop decision is tested on
    /// rather than a fresh, friendlier sample.
    fn classification_corpus() -> Vec<(&'static str, Option<&'static str>)> {
        let mut cases: Vec<(&str, Option<&str>)> = vec![
            ("978-3-407-86821-3", None),
            ("080442957X", None),
            ("9783407868213", Some("ISBN")),
            ("urn:isbn:9783407868213", Some("ISBN")),
            ("2049-3630", Some("ISSN")),
            ("9783407868214", Some("ISBN")),
            ("9783407868214", None),
            ("not-a-real-isbn-shape", Some("ISBN")),
            ("not-a-real-isbn-shape", None),
            ("SHTX001.635962014", None),
            ("urn:uuid:0d2b2f1e-3c4a-4b5c-8d9e-0f1a2b3c4d5e", None),
            ("buyer@example.com", None),
            ("10.0000/TXN-A", Some("DOI")),
            ("urn:epub-tailor:47838272f6d0c223", None),
        ];
        for doi in REAL_DOIS {
            cases.push((doi, Some("DOI")));
            cases.push((doi, None));
        }
        cases
    }

    #[test]
    fn classify_and_is_shared_never_disagree() {
        // The pin that lets `check` reuse this judgment: a value classified
        // `Shared` is exactly a value `normalize` keeps. If these two ever
        // drift, `check` starts either hiding a watermark `generic` removes or
        // inventing one it does not.
        for (value, scheme) in classification_corpus() {
            assert_eq!(
                matches!(classify(value, scheme), PerCopyKind::Shared),
                is_shared(value, scheme),
                "classify and is_shared disagree about {value:?} (scheme {scheme:?})"
            );
        }
    }

    #[test]
    fn the_synthesized_identifier_is_shared_however_many_digits_its_hash_has() {
        // `urn:epub-tailor:<16 hex>` is derived from title and authors, so it
        // converges across copies and must never read as a watermark. The
        // digit count of the hash is the whole hazard: these two differ only
        // in that (13 digits vs 6), and without the prefix exemption the first
        // would be reported as per-copy and the second would not.
        for hash in ["47838272f6d0c223", "ada9f43a24de0cba"] {
            let value = format!("urn:epub-tailor:{hash}");
            assert_eq!(
                classify(&value, None),
                PerCopyKind::Shared,
                "the writer's own synthesized identifier must be kept: {value}"
            );
        }
    }

    #[test]
    fn classify_separates_a_bare_uuid_from_a_genuinely_distinctive_value() {
        // A bare UUID is the EPUB 3 default and says nothing on its own; an
        // email address or a placeholder-registrant DOI names a buyer. Both
        // are dropped, but only the second is worth telling the user about,
        // which is the whole reason this returns a kind and not a bool.
        assert_eq!(
            classify("urn:uuid:0d2b2f1e-3c4a-4b5c-8d9e-0f1a2b3c4d5e", None),
            PerCopyKind::Uuid
        );
        assert_eq!(
            classify("buyer@example.com", None),
            PerCopyKind::Distinctive
        );
        assert_eq!(
            classify("10.0000/TXN-A", Some("DOI")),
            PerCopyKind::Distinctive
        );
        assert_eq!(classify("SHTX001.635962014", None), PerCopyKind::Opaque);
        assert_eq!(classify("978-3-407-86821-3", None), PerCopyKind::Shared);
    }
}
