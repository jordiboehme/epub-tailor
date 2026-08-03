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
    /// whitespace removed, XORed over the first 1040 bytes.
    Idpf,
    /// Adobe's scheme: the identifier's UUID hex digits as 16 raw bytes,
    /// XORed over the first 1024 bytes.
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

/// The IDPF key: SHA-1 over the identifier with every whitespace character
/// removed, per the OCF specification.
fn idpf_key(unique_id: &str) -> Vec<u8> {
    let stripped: String = unique_id.chars().filter(|c| !c.is_whitespace()).collect();
    Sha1::digest(stripped.as_bytes()).to_vec()
}

/// Adobe's key: the 32 hex digits of the identifier's UUID, as 16 bytes.
/// Returns an empty key when the identifier holds no UUID, which makes
/// `xor_prefix` a no-op rather than corrupting the font further.
fn adobe_key(unique_id: &str) -> Vec<u8> {
    let hex: Vec<u8> = unique_id
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .map(|c| c as u8)
        .collect();
    if hex.len() < 32 {
        return Vec::new();
    }
    hex[..32]
        .chunks(2)
        .filter_map(|pair| {
            let s = std::str::from_utf8(pair).ok()?;
            u8::from_str_radix(s, 16).ok()
        })
        .collect()
}

/// Undo `algorithm` over `data` in place. Self-inverse.
pub(crate) fn deobfuscate(data: &mut [u8], algorithm: Obfuscation, unique_id: &str) {
    match algorithm {
        Obfuscation::Idpf => xor_prefix(data, &idpf_key(unique_id), 1040),
        Obfuscation::Adobe => xor_prefix(data, &adobe_key(unique_id), 1024),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn a_font_shorter_than_the_keyed_prefix_does_not_panic() {
        let mut data = vec![1u8, 2, 3];
        deobfuscate(&mut data, Obfuscation::Idpf, "id");
        assert_eq!(data.len(), 3);
    }
}
