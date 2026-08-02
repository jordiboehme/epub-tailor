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
/// is not a parseable JPEG, so the caller leaves it untouched.
fn strip_jpeg(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
        return None;
    }
    let mut out = vec![0xFF, 0xD8];
    let mut i = 2usize;
    while i + 1 < data.len() {
        if data[i] != 0xFF {
            return None;
        }
        let marker = data[i + 1];
        // Start of scan: the entropy-coded data runs to the end, copy verbatim.
        if marker == 0xDA {
            out.extend_from_slice(&data[i..]);
            return Some(out);
        }
        if marker == 0xD9 {
            out.extend_from_slice(&data[i..i + 2]);
            return Some(out);
        }
        let len = u16::from_be_bytes([*data.get(i + 2)?, *data.get(i + 3)?]) as usize;
        let end = i + 2 + len;
        if end > data.len() {
            return None;
        }
        if !JPEG_DROP.contains(&marker) {
            out.extend_from_slice(&data[i..end]);
        }
        i = end;
    }
    Some(out)
}

/// Rewrite a PNG without its ancillary text chunks.
fn strip_png(data: &[u8]) -> Option<Vec<u8>> {
    const SIG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if !data.starts_with(SIG) {
        return None;
    }
    let mut out = SIG.to_vec();
    let mut i = SIG.len();
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
    }
    Some(out)
}

/// Strip metadata from every raster image and SVG in the book.
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
