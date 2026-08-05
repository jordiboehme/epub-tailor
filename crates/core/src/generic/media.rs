//! Lossless media metadata removal: container surgery only, never a re-encode,
//! so the repair path keeps image quality exactly.

use super::normalize_media_type;
use crate::epub::Book;
use crate::report::Transformation;

/// JPEG markers carrying metadata rather than image data. `APP0` (JFIF) and
/// `APP2` (ICC) are kept: dropping the colour profile changes rendering.
const JPEG_DROP: &[u8] = &[0xE1, 0xED, 0xFE]; // APP1 (EXIF/XMP), APP13 (IPTC), COM

/// PNG chunks that carry no image data.
const PNG_DROP: &[&[u8; 4]] = &[b"tEXt", b"iTXt", b"zTXt", b"tIME", b"eXIf"];

/// Rewrite a JPEG without its metadata segments. Returns `None` when the input
/// is not a parseable JPEG - including a truncated or otherwise malformed one
/// that never reaches a terminal marker - so the caller leaves it untouched
/// rather than writing back a shortened, no-longer-decodable file.
pub(crate) fn strip_jpeg(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
        return None;
    }
    let mut out = vec![0xFF, 0xD8];
    let mut i = 2usize;
    while i < data.len() {
        if data[i] != 0xFF {
            return None;
        }
        // A marker may be preceded by any number of legal 0xFF fill bytes;
        // the marker code is the first non-0xFF byte after them.
        let marker_start = i;
        while data.get(i) == Some(&0xFF) {
            i += 1;
        }
        let marker = *data.get(i)?;
        i += 1;
        // Start of scan: the entropy-coded data runs to the end, copy verbatim.
        if marker == 0xDA {
            out.extend_from_slice(&data[marker_start..]);
            return Some(out);
        }
        if marker == 0xD9 {
            out.extend_from_slice(&data[marker_start..i]);
            return Some(out);
        }
        // Standalone markers carry no length field: TEM, RSTn. (RSTn only
        // ever appears inside entropy-coded scan data, copied verbatim above,
        // but handled here too so the omission is not accidental.)
        if marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            out.extend_from_slice(&data[marker_start..i]);
            continue;
        }
        let len = u16::from_be_bytes([*data.get(i)?, *data.get(i + 1)?]) as usize;
        let end = i + len;
        if end > data.len() {
            return None;
        }
        if !JPEG_DROP.contains(&marker) {
            out.extend_from_slice(&data[marker_start..end]);
        }
        i = end;
    }
    // Ran out of bytes without ever seeing SOS or EOI: truncated or
    // malformed, not a file we can safely rewrite.
    None
}

/// CRC-32/ISO-HDLC lookup table (the algorithm PNG chunk CRCs use), built at
/// compile time from the standard reflected polynomial `0xEDB88320`.
const CRC32_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut n = 0usize;
    while n < 256 {
        let mut c = n as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        table[n] = c;
        n += 1;
    }
    table
};

/// CRC-32/ISO-HDLC over `bytes`, matching the checksum PNG stores at the end
/// of every chunk (computed there over the chunk's type and data, never its
/// length or the CRC field itself).
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in bytes {
        let idx = ((crc ^ b as u32) & 0xFF) as usize;
        crc = CRC32_TABLE[idx] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

/// Rewrite a PNG without its ancillary text chunks. Returns `None` when the
/// input is not a parseable PNG - including a truncated one that never
/// reaches `IEND` - so the caller leaves it untouched rather than writing
/// back a shortened, no-longer-decodable file.
pub(crate) fn strip_png(data: &[u8]) -> Option<Vec<u8>> {
    const SIG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if !data.starts_with(SIG) {
        return None;
    }
    let mut out = SIG.to_vec();
    let mut i = SIG.len();
    let mut saw_iend = false;
    while i + 8 <= data.len() {
        let len = u32::from_be_bytes(data.get(i..i + 4)?.try_into().ok()?) as usize;
        let kind: &[u8; 4] = data.get(i + 4..i + 8)?.try_into().ok()?;
        let end = i.checked_add(12).and_then(|v| v.checked_add(len))?; // length + type + data + CRC
        if end > data.len() {
            return None;
        }
        // Validate the chunk's trailing CRC-32, computed over its type and
        // data (never the length field or the CRC field itself), before
        // trusting the chunk enough to keep or re-emit it.
        let stored_crc = u32::from_be_bytes(data[end - 4..end].try_into().ok()?);
        if crc32(&data[i + 4..end - 4]) != stored_crc {
            return None;
        }
        if !PNG_DROP.contains(&kind) {
            out.extend_from_slice(&data[i..end]);
        }
        i = end;
        if kind == b"IEND" {
            saw_iend = true;
            break;
        }
    }
    // A well-formed PNG always ends with an IEND chunk; anything else is
    // truncated or malformed.
    if saw_iend { Some(out) } else { None }
}

/// Which stripper applies to a resource, decided primarily by its magic
/// bytes rather than its declared manifest media type: a mis-declared image
/// (`image/jpg`, `IMAGE/JPEG`, `image/jpeg; charset=binary`, or a JPEG
/// sitting behind `application/octet-stream`) is exactly the shape a privacy
/// pass needs to see through, since a manifest error is precisely the sort of
/// small inconsistency a shop's per-copy build pipeline produces. The
/// declared type is still consulted as a fallback for the (rare) case where
/// the data does not start with a magic-byte match this scan recognizes but
/// the manifest is nonetheless one of the two canonical spellings.
fn image_kind(media_type: &str, data: &[u8]) -> Option<ImageKind> {
    if data.starts_with(&[0xFF, 0xD8]) {
        return Some(ImageKind::Jpeg);
    }
    if data.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Some(ImageKind::Png);
    }
    match normalize_media_type(media_type).as_str() {
        "image/jpeg" => Some(ImageKind::Jpeg),
        "image/png" => Some(ImageKind::Png),
        _ => None,
    }
}

enum ImageKind {
    Jpeg,
    Png,
}

/// Strip metadata from every raster image in the book.
pub(crate) fn strip(book: &mut Book, transformations: &mut Vec<Transformation>) {
    let paths: Vec<String> = book.resources.keys().cloned().collect();
    for path in paths {
        let resource = &mut book.resources[&path];
        let stripped = match image_kind(&resource.media_type, &resource.data) {
            Some(ImageKind::Jpeg) => strip_jpeg(&resource.data),
            Some(ImageKind::Png) => strip_png(&resource.data),
            None => None,
        };
        let Some(new_data) = stripped else { continue };
        if new_data.len() == resource.data.len() {
            continue;
        }
        let saved = resource.data.len() - new_data.len();
        resource.data = new_data;
        transformations.push(Transformation {
            kind: "generic-media".to_string(),
            detail: format!("stripped {saved} bytes of image metadata"),
            file: Some(path.clone()),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::epub::{Metadata, Resource};
    use indexmap::IndexMap;

    /// A truncated JPEG: SOI, then one droppable APP1 (EXIF) segment, cut
    /// immediately after it - no SOS, no EOI. Before the truncation fix this
    /// silently produced a shortened-but-still-JPEG-looking `Some(...)`.
    fn truncated_jpeg_with_a_droppable_segment() -> Vec<u8> {
        let mut out = vec![0xFF, 0xD8];
        let payload = b"Exif\0\0truncated-mid-file";
        out.extend_from_slice(&[0xFF, 0xE1]);
        out.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
        out.extend_from_slice(payload);
        out
    }

    /// Append one well-formed, correctly-CRC'd PNG chunk (length + type +
    /// data + CRC-32 over type-and-data) to `out`.
    fn push_png_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut type_and_data = Vec::with_capacity(4 + data.len());
        type_and_data.extend_from_slice(kind);
        type_and_data.extend_from_slice(data);
        out.extend_from_slice(&type_and_data);
        out.extend_from_slice(&crc32(&type_and_data).to_be_bytes());
    }

    /// A minimal, fully-valid PNG: signature, one `tEXt` chunk, then `IEND`,
    /// every chunk carrying a genuine CRC - so that flipping a single CRC
    /// byte in a test is an isolated, meaningful mutation rather than
    /// coincidentally already-wrong.
    pub(super) fn minimal_png_with_text_chunk() -> Vec<u8> {
        let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        push_png_chunk(&mut out, b"tEXt", b"comment-data");
        push_png_chunk(&mut out, b"IEND", &[]);
        out
    }

    /// A truncated PNG: signature, then one droppable `tEXt` chunk (with a
    /// genuine CRC), cut immediately after it - no `IEND`. Same failure
    /// shape as the JPEG case.
    fn truncated_png_with_a_droppable_chunk() -> Vec<u8> {
        let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        push_png_chunk(&mut out, b"tEXt", b"comment-data");
        out
    }

    #[test]
    fn a_jpeg_truncated_after_a_droppable_segment_is_rejected() {
        assert!(
            strip_jpeg(&truncated_jpeg_with_a_droppable_segment()).is_none(),
            "a truncated JPEG must not be rewritten as if it stripped cleanly"
        );
    }

    #[test]
    fn a_png_truncated_after_a_droppable_chunk_is_rejected() {
        assert!(
            strip_png(&truncated_png_with_a_droppable_chunk()).is_none(),
            "a truncated PNG must not be rewritten as if it stripped cleanly"
        );
    }

    #[test]
    fn crc32_matches_known_good_vectors() {
        // Independently verified via Python's zlib.crc32, a mature reference
        // implementation of the same CRC-32/ISO-HDLC algorithm PNG uses.
        assert_eq!(crc32(b""), 0x0000_0000);
        assert_eq!(
            crc32(b"123456789"),
            0xCBF4_3926,
            "the standard CRC-32 check value"
        );
        assert_eq!(
            crc32(b"IEND"),
            0xAE42_6082,
            "CRC of a zero-length IEND chunk's type-and-data"
        );
    }

    /// A real 1x1 greyscale PNG carrying a `tEXt` comment, byte-for-byte, with
    /// every chunk CRC computed by Python's `zlib.crc32` rather than by this
    /// module. The round-trip tests all build their fixtures with our own
    /// `crc32`, so a mutated CRC table stays self-consistent and they pass
    /// anyway; this one does not, and fails.
    const REAL_PNG_WITH_TEXT: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x00, 0x00, 0x00, 0x00, 0x3A,
        0x7E, 0x9B, 0x55, 0x00, 0x00, 0x00, 0x12, 0x74, 0x45, 0x58, 0x74, 0x43, 0x6F, 0x6D, 0x6D,
        0x65, 0x6E, 0x74, 0x00, 0x42, 0x55, 0x59, 0x45, 0x52, 0x2D, 0x34, 0x37, 0x31, 0x31, 0x40,
        0x76, 0x66, 0x58, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x60,
        0x00, 0x00, 0x00, 0x02, 0x00, 0x01, 0x48, 0xAF, 0xA4, 0x71, 0x00, 0x00, 0x00, 0x00, 0x49,
        0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    #[test]
    fn a_png_built_outside_this_module_round_trips_and_loses_its_text_chunk() {
        let out = strip_png(REAL_PNG_WITH_TEXT).expect("a real PNG with valid CRCs must parse");
        assert!(
            !out.windows(5).any(|w| w == b"BUYER"),
            "the tEXt payload must be gone"
        );
        assert!(
            out.windows(4).any(|w| w == b"IHDR") && out.windows(4).any(|w| w == b"IDAT"),
            "the image data must survive"
        );
        assert!(out.ends_with(&[0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82]));
    }

    #[test]
    fn a_png_with_a_corrupt_chunk_crc_is_left_alone() {
        let mut png = super::tests::minimal_png_with_text_chunk();
        let len = png.len();
        // The last 4 bytes of the file are the IEND chunk's CRC field
        // (len-12..len-8 is IEND's length, len-8..len-4 is the type "IEND",
        // len-4..len is the CRC): flip a bit inside that CRC field itself,
        // not the type that precedes it.
        png[len - 1] ^= 0xFF; // corrupt the IEND CRC
        assert!(strip_png(&png).is_none(), "a bad CRC must not be rewritten");
    }

    #[test]
    fn a_png_with_an_over_long_declared_chunk_length_is_rejected() {
        let mut png = super::tests::minimal_png_with_text_chunk();
        // Declare a chunk length of u32::MAX, far longer than the file
        // actually is. This does not discriminate the checked-arithmetic
        // change on a 64-bit target - `i + 12 + len` cannot overflow `usize`
        // here since `len` is at most `u32::MAX` and `i` is tiny, so the
        // pre-existing `end > data.len()` bounds check already rejects it
        // even without checked arithmetic. It still pins the required
        // behaviour (an over-long declared length must be rejected, not
        // wrapped or read out of bounds) and is the only overflow-adjacent
        // shape constructible from a 4-byte length field on this platform.
        png[8..12].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(strip_png(&png).is_none());
    }

    #[test]
    fn strip_leaves_truncated_resources_byte_identical() {
        let jpeg = truncated_jpeg_with_a_droppable_segment();
        let png = truncated_png_with_a_droppable_chunk();
        let mut resources = IndexMap::new();
        resources.insert(
            "pic.jpg".to_string(),
            Resource {
                data: jpeg.clone(),
                media_type: "image/jpeg".to_string(),
                ..Default::default()
            },
        );
        resources.insert(
            "pic.png".to_string(),
            Resource {
                data: png.clone(),
                media_type: "image/png".to_string(),
                ..Default::default()
            },
        );
        let mut book = Book {
            metadata: Metadata::default(),
            resources,
            spine: Vec::new(),
            toc: Vec::new(),
            cover: None,
            opf_path: "content.opf".to_string(),
            nav_path: None,
            ncx_path: None,
        };
        let mut transformations = Vec::new();
        strip(&mut book, &mut transformations);
        assert_eq!(
            book.resources["pic.jpg"].data, jpeg,
            "a truncated JPEG that fails to parse must survive untouched"
        );
        assert_eq!(
            book.resources["pic.png"].data, png,
            "a truncated PNG that fails to parse must survive untouched"
        );
        assert!(
            transformations.is_empty(),
            "no transformation should be recorded when nothing was stripped"
        );
    }

    #[test]
    fn jpeg_fill_bytes_before_a_marker_do_not_misparse_the_next_marker() {
        // SOI, then an APP1 (EXIF) segment whose marker is preceded by two
        // legal 0xFF fill bytes, then a minimal SOS/EOI tail.
        let mut jpeg = vec![0xFF, 0xD8];
        let payload = b"Exif\0\0BUYER-635962014";
        jpeg.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xE1]);
        jpeg.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
        jpeg.extend_from_slice(payload);
        jpeg.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x02, 0xFF, 0xD9]);

        let stripped = strip_jpeg(&jpeg).expect("still a parseable JPEG");
        assert!(
            !stripped.windows(9).any(|w| w == b"BUYER-635"),
            "the EXIF payload behind the fill bytes must still be dropped"
        );
        assert_eq!(&stripped[..2], &[0xFF, 0xD8]);
        assert!(stripped.ends_with(&[0xFF, 0xD9]));
    }

    #[test]
    fn jpeg_restart_markers_in_scan_data_survive_unchanged() {
        // SOI, a kept APP0 (JFIF) segment, SOS, then entropy-coded data
        // containing an RST0 marker byte pair - copied verbatim, never
        // misread as carrying a length field.
        let mut jpeg = vec![0xFF, 0xD8];
        jpeg.extend_from_slice(&[0xFF, 0xE0, 0x00, 0x04, 0x4A, 0x46]); // APP0, kept
        jpeg.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x02]); // SOS header
        jpeg.extend_from_slice(&[0x11, 0x22, 0xFF, 0xD0, 0x33, 0x44]); // scan data + RST0
        jpeg.extend_from_slice(&[0xFF, 0xD9]); // EOI

        let stripped = strip_jpeg(&jpeg).expect("still a parseable JPEG");
        assert_eq!(
            stripped, jpeg,
            "kept APP0 plus verbatim scan data is unchanged"
        );
    }

    #[test]
    fn a_standalone_marker_before_the_scan_is_copied_without_a_length_read() {
        // TEM and RSTn carry no length field. Reading two bytes after them as
        // a length misparses the rest of the file. This places one BEFORE the
        // SOS, which is the only position the dedicated branch handles - the
        // existing RST test puts one inside the scan data, where the verbatim
        // copy-to-end path covers it and the branch is never reached.
        let mut jpeg = vec![0xFF, 0xD8]; // SOI
        jpeg.extend_from_slice(&[0xFF, 0x01]); // TEM, no length
        let payload = b"Exif\0\0BUYER-1";
        jpeg.extend_from_slice(&[0xFF, 0xE1]);
        jpeg.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
        jpeg.extend_from_slice(payload);
        jpeg.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x02, 0xFF, 0xD9]); // SOS then EOI

        let out = strip_jpeg(&jpeg).expect("still parseable with a standalone marker");
        assert!(
            !out.windows(5).any(|w| w == b"BUYER"),
            "EXIF must still be stripped"
        );
        assert!(
            out.windows(2).any(|w| w == [0xFF, 0x01]),
            "the standalone marker itself must be preserved"
        );
        assert!(out.ends_with(&[0xFF, 0xD9]));
    }

    /// A complete, parseable JPEG (SOI through EOI) carrying a droppable
    /// APP1 (EXIF) segment with `payload` - unlike
    /// `truncated_jpeg_with_a_droppable_segment`, this one is a shape
    /// `strip_jpeg` accepts, so `strip()` actually rewrites it.
    fn jpeg_with_exif_payload(payload: &[u8]) -> Vec<u8> {
        let mut out = vec![0xFF, 0xD8];
        out.extend_from_slice(&[0xFF, 0xE1]);
        out.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
        out.extend_from_slice(payload);
        out.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x02, 0xFF, 0xD9]);
        out
    }

    /// `strip()` a single JPEG resource declared as `media_type` and assert
    /// its EXIF payload is gone - the shape every "silently ships the
    /// watermark" case in the finding takes: a real JPEG behind a manifest
    /// media type that is not the exact canonical `image/jpeg` string.
    fn assert_strips_mis_declared_jpeg(media_type: &str) {
        let payload = b"Exif\0\0BUYER-635962014-SECRET";
        let jpeg = jpeg_with_exif_payload(payload);
        let mut resources = IndexMap::new();
        resources.insert(
            "pic.jpg".to_string(),
            Resource {
                data: jpeg,
                media_type: media_type.to_string(),
                ..Default::default()
            },
        );
        let mut book = Book {
            metadata: Metadata::default(),
            resources,
            spine: Vec::new(),
            toc: Vec::new(),
            cover: None,
            opf_path: "content.opf".to_string(),
            nav_path: None,
            ncx_path: None,
        };
        let mut transformations = Vec::new();
        strip(&mut book, &mut transformations);
        let out = &book.resources["pic.jpg"].data;
        assert!(
            !out.windows(9).any(|w| w == b"BUYER-635"),
            "{media_type:?}: the EXIF payload must be stripped, not shipped intact"
        );
        assert_eq!(
            transformations.len(),
            1,
            "{media_type:?}: a transformation must be reported, not a silent no-op"
        );
    }

    #[test]
    fn strips_the_common_non_canonical_image_jpg_spelling() {
        assert_strips_mis_declared_jpeg("image/jpg");
    }

    #[test]
    fn strips_an_upper_case_media_type() {
        assert_strips_mis_declared_jpeg("IMAGE/JPEG");
    }

    #[test]
    fn strips_a_media_type_carrying_a_parameter() {
        assert_strips_mis_declared_jpeg("image/jpeg; charset=binary");
    }

    #[test]
    fn strips_a_jpeg_mis_declared_as_a_generic_octet_stream() {
        // No `image/*` spelling at all: magic-byte sniffing, not the
        // declared manifest type, is what has to catch this one.
        assert_strips_mis_declared_jpeg("application/octet-stream");
    }

    /// A complete, parseable PNG (signature through `IEND`) carrying a
    /// droppable `tEXt` chunk with `payload` - the PNG counterpart of
    /// `jpeg_with_exif_payload`, exercising the `image_kind` magic-byte
    /// branch [`strip_png`] itself never runs on nothing else covers.
    fn png_with_text_payload(payload: &[u8]) -> Vec<u8> {
        let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        push_png_chunk(&mut out, b"tEXt", payload);
        push_png_chunk(&mut out, b"IEND", &[]);
        out
    }

    /// `strip()` a single PNG resource declared as `media_type` and assert
    /// its `tEXt` payload is gone and a `generic-media` transformation is
    /// reported - the PNG counterpart of `assert_strips_mis_declared_jpeg`.
    fn assert_strips_mis_declared_png(media_type: &str) {
        let payload = b"Comment\0BUYER-635962014-SECRET";
        let png = png_with_text_payload(payload);
        let mut resources = IndexMap::new();
        resources.insert(
            "pic.png".to_string(),
            Resource {
                data: png,
                media_type: media_type.to_string(),
                ..Default::default()
            },
        );
        let mut book = Book {
            metadata: Metadata::default(),
            resources,
            spine: Vec::new(),
            toc: Vec::new(),
            cover: None,
            opf_path: "content.opf".to_string(),
            nav_path: None,
            ncx_path: None,
        };
        let mut transformations = Vec::new();
        strip(&mut book, &mut transformations);
        let out = &book.resources["pic.png"].data;
        assert!(
            !out.windows(9).any(|w| w == b"BUYER-635"),
            "{media_type:?}: the tEXt payload must be stripped, not shipped intact"
        );
        assert_eq!(
            transformations.len(),
            1,
            "{media_type:?}: a transformation must be reported, not a silent no-op"
        );
    }

    #[test]
    fn strips_a_png_mis_declared_as_the_jpeg_extension_spelling() {
        assert_strips_mis_declared_png("image/jpg");
    }

    #[test]
    fn strips_a_png_with_an_upper_case_media_type() {
        assert_strips_mis_declared_png("IMAGE/PNG");
    }

    #[test]
    fn strips_a_png_media_type_carrying_a_parameter() {
        assert_strips_mis_declared_png("image/png; charset=binary");
    }

    #[test]
    fn strips_a_png_mis_declared_as_a_generic_octet_stream() {
        // No `image/*` spelling at all: magic-byte sniffing, not the
        // declared manifest type, is what has to catch this one.
        assert_strips_mis_declared_png("application/octet-stream");
    }
}
