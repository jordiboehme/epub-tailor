//! Lossless media metadata removal: container surgery only, never a re-encode,
//! so the repair path keeps image quality exactly.

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
fn strip_jpeg(data: &[u8]) -> Option<Vec<u8>> {
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

/// Rewrite a PNG without its ancillary text chunks. Returns `None` when the
/// input is not a parseable PNG - including a truncated one that never
/// reaches `IEND` - so the caller leaves it untouched rather than writing
/// back a shortened, no-longer-decodable file.
fn strip_png(data: &[u8]) -> Option<Vec<u8>> {
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
        let end = i + 12 + len; // length + type + data + CRC
        if end > data.len() {
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

/// Strip metadata from every raster image in the book.
pub(crate) fn strip(book: &mut Book, transformations: &mut Vec<Transformation>) {
    let paths: Vec<String> = book.resources.keys().cloned().collect();
    for path in paths {
        let resource = &mut book.resources[&path];
        let stripped = match resource.media_type.as_str() {
            "image/jpeg" => strip_jpeg(&resource.data),
            "image/png" => strip_png(&resource.data),
            _ => None,
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

    /// A truncated PNG: signature, then one droppable `tEXt` chunk, cut
    /// immediately after it - no `IEND`. Same failure shape as the JPEG case.
    fn truncated_png_with_a_droppable_chunk() -> Vec<u8> {
        let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        let text = b"comment-data";
        out.extend_from_slice(&(text.len() as u32).to_be_bytes());
        out.extend_from_slice(b"tEXt");
        out.extend_from_slice(text);
        out.extend_from_slice(&[0, 0, 0, 0]); // CRC, unchecked by the stripper
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
    fn strip_leaves_truncated_resources_byte_identical() {
        let jpeg = truncated_jpeg_with_a_droppable_segment();
        let png = truncated_png_with_a_droppable_chunk();
        let mut resources = IndexMap::new();
        resources.insert(
            "pic.jpg".to_string(),
            Resource {
                data: jpeg.clone(),
                media_type: "image/jpeg".to_string(),
            },
        );
        resources.insert(
            "pic.png".to_string(),
            Resource {
                data: png.clone(),
                media_type: "image/png".to_string(),
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
}
