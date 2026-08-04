//! EPUB font obfuscation, undone.
//!
//! Both algorithms XOR a prefix of the font with a key derived from the
//! book's unique identifier. The reader is told which fonts are obfuscated by
//! `META-INF/encryption.xml`, which this pipeline does not carry into its
//! output - so leaving the bytes scrambled produced a font no reading system
//! could use. De-obfuscating here yields a plain font and lets the
//! declaration go, which is also the only option that stays correct when
//! `normalize_identity` later replaces the unique identifier the key came
//! from.
//!
//! Both operations are XOR, so each function is its own inverse.

use sha1::{Digest, Sha1};

/// Which obfuscation scheme a font was scrambled with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Obfuscation {
    /// `http://www.idpf.org/2008/embedding`: SHA-1 of the identifier with all
    /// whitespace removed, XORed over the first 1040 bytes. Always yields a
    /// usable key: SHA-1 of any input (including the empty string) is 20
    /// bytes.
    Idpf,
    /// Adobe's scheme: the 16 raw bytes of a `urn:uuid:` identifier's UUID,
    /// XORed over the first 1024 bytes. Yields no key - and so leaves the
    /// font untouched - when the identifier is not a canonical `urn:uuid:`
    /// value.
    Adobe,
}

/// XOR `data`'s leading `len` bytes with `key`, repeating.
fn xor_prefix(data: &mut [u8], key: &[u8], len: usize) {
    if key.is_empty() {
        return;
    }
    for (i, byte) in data.iter_mut().take(len).enumerate() {
        *byte ^= key[i % key.len()];
    }
}

/// `unique_id` with every whitespace character OCF 3.3 section 4.4.3 names
/// removed. That section names exactly four - U+0020 SPACE, U+0009 TAB,
/// U+000D CR and U+000A LF - which is narrower than Rust's
/// `char::is_whitespace` (the Unicode `White_Space` property, which also
/// covers U+00A0 NBSP and half a dozen others): using the wider set would
/// hash a different string than the identifier's producer did whenever one of
/// those extra characters was present, silently deriving the wrong key.
///
/// Public to the crate so the reader can apply it once, when it lifts the
/// identifier out of the OPF, and both key derivations below see the same
/// normalized string. In particular [`uuid_hex_digits`] matches a literal
/// `urn:uuid:` prefix, so an indented `<dc:identifier>` would otherwise fail
/// the match on its leading newline and derive no Adobe key at all.
pub(crate) fn strip_ocf_whitespace(unique_id: &str) -> String {
    unique_id
        .chars()
        .filter(|c| !matches!(c, ' ' | '\t' | '\r' | '\n'))
        .collect()
}

/// The IDPF key: SHA-1 over the identifier, whitespace-stripped per
/// [`strip_ocf_whitespace`]. Applied again here rather than assumed, so the
/// key is correct however the caller obtained the identifier - the operation
/// is idempotent.
fn idpf_key(unique_id: &str) -> Vec<u8> {
    Sha1::digest(strip_ocf_whitespace(unique_id).as_bytes()).to_vec()
}

/// The 32 lowercase hex digits of `unique_id`'s UUID, if `unique_id` is a
/// `urn:uuid:` identifier (case-insensitive prefix and digits). `None` for
/// anything else - a bare UUID with no `urn:uuid:` prefix, or a non-UUID
/// identifier such as an ISBN - so a caller never guesses at a shape the
/// identifier does not actually have.
///
/// Hyphens are removed wherever they fall rather than required in canonical
/// 8-4-4-4-12 grouping. The stricter reading rejected identifiers other tools
/// accept, and the key those tools derive is the one the book was actually
/// obfuscated with - so being stricter here protects nothing, it just fails to
/// unscramble fonts every other reader handles.
///
/// Close to, but not the same as, Python's `uuid.UUID`: that also accepts a
/// `{...}`-wrapped form, which this does not. Note too that the caller has
/// already run [`strip_ocf_whitespace`], so an identifier with whitespace
/// where the hyphens should be reaches here as bare hex and is accepted.
fn uuid_hex_digits(unique_id: &str) -> Option<String> {
    let lower = unique_id.to_ascii_lowercase();
    let rest = lower.strip_prefix("urn:uuid:")?;
    let digits: String = rest.chars().filter(|c| *c != '-').collect();
    if digits.len() != 32 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some(digits)
}

/// Adobe's key: the identifier's UUID as 16 raw bytes. Returns an empty key
/// when the identifier holds no `urn:uuid:` value, which makes `xor_prefix`
/// a no-op rather than corrupting the font further.
///
/// The identifier is not scavenged for "32 hex digits" wherever they fall:
/// `urn:uuid:12345678-…` itself contains a hex digit (`d`, in `uuid`) before
/// the UUID proper starts, so a naive filter-the-whole-string approach reads
/// one character too early and returns every nibble shifted by one -
/// systematically wrong for the single most common identifier shape EPUB 3
/// recommends (`dc:identifier` as `urn:uuid:...`), not an edge case.
fn adobe_key(unique_id: &str) -> Vec<u8> {
    let Some(hex) = uuid_hex_digits(unique_id) else {
        return Vec::new();
    };
    hex.as_bytes()
        .chunks(2)
        .filter_map(|pair| {
            let s = std::str::from_utf8(pair).ok()?;
            u8::from_str_radix(s, 16).ok()
        })
        .collect()
}

/// Undo `algorithm` over `data` in place. Self-inverse. Returns whether a
/// usable key was derived from `unique_id`: `false` means `data` was left
/// untouched (Adobe's scheme with a non-`urn:uuid:` identifier is the only
/// way this happens - IDPF's SHA-1 key is always usable), so the caller
/// should report that the font is still scrambled rather than claim success.
pub(crate) fn deobfuscate(data: &mut [u8], algorithm: Obfuscation, unique_id: &str) -> bool {
    let (key, len) = match algorithm {
        Obfuscation::Idpf => (idpf_key(unique_id), 1040),
        Obfuscation::Adobe => (adobe_key(unique_id), 1024),
    };
    let applied = !key.is_empty();
    xor_prefix(data, &key, len);
    applied
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SHA-1("abc"), the standard FIPS 180/RFC 3174 test vector - computed
    /// independently (Python's `hashlib`), not read out of this
    /// implementation.
    const SHA1_ABC: [u8; 20] = [
        0xa9, 0x99, 0x3e, 0x36, 0x47, 0x06, 0x81, 0x6a, 0xba, 0x3e, 0x25, 0x71, 0x78, 0x50, 0xc2,
        0x6c, 0x9c, 0xd0, 0xd8, 0x9d,
    ];

    /// The Adobe key for `urn:uuid:12345678-1234-1234-1234-123456789012`,
    /// computed by hand from the UUID's hex digits (`12 34 56 78 12 34 12 34
    /// 12 34 12 34 56 78 90 12`), not read out of this implementation.
    const ADOBE_KEY_12345678: [u8; 16] = [
        0x12, 0x34, 0x56, 0x78, 0x12, 0x34, 0x12, 0x34, 0x12, 0x34, 0x12, 0x34, 0x56, 0x78, 0x90,
        0x12,
    ];

    /// XORing an all-zero buffer with a key reproduces the key bytes
    /// verbatim (`0 ^ k == k`), so de-obfuscating a zero-filled buffer reads
    /// the ACTUAL derived key back out - pinning it exactly, rather than
    /// merely proving a round trip that would pass under any key at all
    /// (which is exactly the hole that let a wrong Adobe key ship: every
    /// prior test round-tripped under the bug just as well as under the
    /// fix).
    #[test]
    fn idpf_key_matches_the_sha1_abc_test_vector_over_its_full_1040_byte_prefix() {
        let mut data = vec![0u8; 1100];
        deobfuscate(&mut data, Obfuscation::Idpf, "abc");
        for (i, byte) in data[..1040].iter().enumerate() {
            assert_eq!(
                *byte,
                SHA1_ABC[i % SHA1_ABC.len()],
                "byte {i} must be the repeating SHA-1 key, not something else"
            );
        }
        assert_eq!(&data[1040..], &vec![0u8; 60][..], "past 1040, untouched");
    }

    #[test]
    fn adobe_key_matches_the_uuid_bytes_over_its_full_1024_byte_prefix() {
        let mut data = vec![0u8; 1100];
        deobfuscate(
            &mut data,
            Obfuscation::Adobe,
            "urn:uuid:12345678-1234-1234-1234-123456789012",
        );
        for (i, byte) in data[..1024].iter().enumerate() {
            assert_eq!(
                *byte,
                ADOBE_KEY_12345678[i % ADOBE_KEY_12345678.len()],
                "byte {i} must be the repeating UUID key, not something else"
            );
        }
        assert_eq!(&data[1024..], &vec![0u8; 76][..], "past 1024, untouched");
    }

    /// The IDPF algorithm XORs the first 1040 bytes with a repeating SHA-1 of
    /// the unique identifier, so applying it twice is the identity.
    #[test]
    fn idpf_deobfuscation_is_its_own_inverse() {
        let original: Vec<u8> = (0..2000u32).map(|i| (i % 251) as u8).collect();
        let mut data = original.clone();
        deobfuscate(
            &mut data,
            Obfuscation::Idpf,
            "urn:uuid:12345678-1234-1234-1234-123456789012",
        );
        assert_ne!(data, original, "obfuscation must change the bytes");
        deobfuscate(
            &mut data,
            Obfuscation::Idpf,
            "urn:uuid:12345678-1234-1234-1234-123456789012",
        );
        assert_eq!(data, original, "applying twice must restore the original");
    }

    /// Pins the exact prefix cutoff, not just "some byte in range differs":
    /// byte 1039 (the last keyed byte) is guaranteed to change because
    /// `SHA1_ABC[1039 % 20] == SHA1_ABC[19] == 0x9d` is non-zero (XOR with a
    /// non-zero byte always changes the value), and byte 1040 must stay
    /// untouched. A prefix accidentally shortened to (say) 1024 would flip
    /// the first assertion; accidentally lengthened would flip the second.
    #[test]
    fn idpf_prefix_boundary_is_exactly_between_byte_1039_and_1040() {
        let original: Vec<u8> = (0..1100u32).map(|i| (i % 251) as u8).collect();
        let mut data = original.clone();
        deobfuscate(&mut data, Obfuscation::Idpf, "abc");
        assert_ne!(
            data[1039], original[1039],
            "byte 1039 (last of the 1040-byte prefix) must be keyed"
        );
        assert_eq!(
            data[1040], original[1040],
            "byte 1040 (first past the prefix) must be untouched"
        );
    }

    #[test]
    fn idpf_leaves_bytes_past_1040_untouched() {
        let original: Vec<u8> = (0..2000u32).map(|i| (i % 251) as u8).collect();
        let mut data = original.clone();
        deobfuscate(&mut data, Obfuscation::Idpf, "id");
        assert_eq!(
            &data[1040..],
            &original[1040..],
            "only the first 1040 bytes are keyed"
        );
        assert_ne!(&data[..1040], &original[..1040]);
    }

    #[test]
    fn adobe_deobfuscation_is_its_own_inverse_over_1024_bytes() {
        let original: Vec<u8> = (0..2000u32).map(|i| (i % 251) as u8).collect();
        let mut data = original.clone();
        let id = "urn:uuid:12345678-1234-1234-1234-123456789012";
        deobfuscate(&mut data, Obfuscation::Adobe, id);
        assert_ne!(data, original);
        assert_eq!(
            &data[1024..],
            &original[1024..],
            "only the first 1024 bytes are keyed"
        );
        deobfuscate(&mut data, Obfuscation::Adobe, id);
        assert_eq!(data, original);
    }

    /// Same reasoning as the IDPF boundary test: `ADOBE_KEY_12345678[1023 %
    /// 16] == ADOBE_KEY_12345678[15] == 0x12` is non-zero, so byte 1023 is
    /// guaranteed to change and byte 1024 is guaranteed not to.
    #[test]
    fn adobe_prefix_boundary_is_exactly_between_byte_1023_and_1024() {
        let original: Vec<u8> = (0..1100u32).map(|i| (i % 251) as u8).collect();
        let mut data = original.clone();
        deobfuscate(
            &mut data,
            Obfuscation::Adobe,
            "urn:uuid:12345678-1234-1234-1234-123456789012",
        );
        assert_ne!(
            data[1023], original[1023],
            "byte 1023 (last of the 1024-byte prefix) must be keyed"
        );
        assert_eq!(
            data[1024], original[1024],
            "byte 1024 (first past the prefix) must be untouched"
        );
    }

    #[test]
    fn a_font_shorter_than_the_keyed_prefix_does_not_panic() {
        let mut data = vec![1u8, 2, 3];
        deobfuscate(&mut data, Obfuscation::Idpf, "id");
        assert_eq!(data.len(), 3);
    }

    /// The bug this pins: scavenging hex digits out of the WHOLE identifier
    /// (rather than the UUID after `urn:uuid:`) reads the `d` in "uuid"
    /// itself as the first hex digit, shifting every nibble by one and
    /// deriving a completely wrong key for the identifier shape EPUB 3
    /// recommends.
    #[test]
    fn adobe_key_is_not_thrown_off_by_the_hex_digit_in_uuid_itself() {
        let mut data = vec![0u8; 16];
        deobfuscate(
            &mut data,
            Obfuscation::Adobe,
            "urn:uuid:12345678-1234-1234-1234-123456789012",
        );
        assert_eq!(data, ADOBE_KEY_12345678);
    }

    #[test]
    fn adobe_key_is_empty_without_a_urn_uuid_identifier() {
        // No key derivable: `deobfuscate` must report failure and leave the
        // data untouched, not XOR with a bogus key scavenged from whatever
        // hex-looking characters happen to appear (this ISBN-shaped
        // identifier has plenty: every digit 0-9 is a hex digit too).
        let mut data = vec![1u8, 2, 3, 4, 5];
        let original = data.clone();
        let applied = deobfuscate(&mut data, Obfuscation::Adobe, "9783407868213");
        assert!(!applied, "an ISBN identifier has no UUID to key from");
        assert_eq!(data, original, "no key means no bytes should change");
    }

    #[test]
    fn adobe_key_is_empty_for_a_bare_uuid_with_no_urn_uuid_prefix() {
        let mut data = vec![1u8, 2, 3, 4, 5];
        let original = data.clone();
        let applied = deobfuscate(
            &mut data,
            Obfuscation::Adobe,
            "12345678-1234-1234-1234-123456789012",
        );
        assert!(!applied, "the prefix-less spelling must not be accepted");
        assert_eq!(data, original);
    }

    #[test]
    fn adobe_key_accepts_an_ungrouped_uuid_the_way_calibre_does() {
        // Python's `uuid.UUID` and Calibre both strip hyphens and require 32
        // hex digits, rather than insisting on 8-4-4-4-12. A book obfuscated
        // by a tool that wrote the ungrouped spelling has a real key; refusing
        // to derive it leaves the font scrambled for no gain.
        let mut grouped = vec![0u8; 16];
        deobfuscate(
            &mut grouped,
            Obfuscation::Adobe,
            "urn:uuid:12345678-1234-1234-1234-123456789012",
        );
        let mut ungrouped = vec![0u8; 16];
        let applied = deobfuscate(
            &mut ungrouped,
            Obfuscation::Adobe,
            "urn:uuid:12345678123412341234123456789012",
        );
        assert!(applied, "the ungrouped spelling must still yield a key");
        assert_eq!(
            ungrouped, grouped,
            "and it must be the same key the canonical spelling gives"
        );
    }

    #[test]
    fn adobe_key_still_rejects_the_wrong_number_of_hex_digits() {
        // Relaxing the grouping must not relax the length: 31 digits is not a
        // UUID, and guessing a key from it would scramble the font.
        let mut data = vec![1u8, 2, 3, 4, 5];
        let original = data.clone();
        let applied = deobfuscate(
            &mut data,
            Obfuscation::Adobe,
            "urn:uuid:1234567812341234123412345678901",
        );
        assert!(!applied, "31 hex digits is not a UUID");
        assert_eq!(data, original);
    }

    #[test]
    fn adobe_key_prefix_match_is_case_insensitive() {
        let mut lower = vec![0u8; 16];
        deobfuscate(
            &mut lower,
            Obfuscation::Adobe,
            "urn:uuid:12345678-1234-1234-1234-123456789012",
        );
        let mut upper = vec![0u8; 16];
        deobfuscate(
            &mut upper,
            Obfuscation::Adobe,
            "URN:UUID:12345678-1234-1234-1234-123456789012",
        );
        assert_eq!(lower, upper);
        assert_eq!(lower, ADOBE_KEY_12345678);
    }

    #[test]
    fn deobfuscate_reports_whether_a_key_was_applied() {
        let mut idpf_data = vec![0u8; 4];
        assert!(
            deobfuscate(&mut idpf_data, Obfuscation::Idpf, ""),
            "IDPF always has a usable SHA-1 key, even for an empty identifier"
        );

        let mut adobe_ok = vec![0u8; 4];
        assert!(deobfuscate(
            &mut adobe_ok,
            Obfuscation::Adobe,
            "urn:uuid:12345678-1234-1234-1234-123456789012",
        ));

        let mut adobe_missing = vec![0u8; 4];
        assert!(!deobfuscate(
            &mut adobe_missing,
            Obfuscation::Adobe,
            "not-a-uuid"
        ));
    }
}
