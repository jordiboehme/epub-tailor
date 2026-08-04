//! EPUB fixture primitives shared by every test binary in the workspace.
//!
//! These pieces used to be copied into each `tests/common/mod.rs` and into
//! several individual test binaries, because an integration test cannot import
//! another crate's test module. Copies drift: on one branch every srcset
//! fixture carried an 8-byte PNG stub the pipeline could not decode, so the
//! rename path was never exercised and a real defect stayed hidden. A dev-only
//! crate is the one place all of them can share.
//!
//! Dev-only: `publish = false`, and it is a `[dev-dependencies]` entry, so
//! nothing here ships in the library or the binary.

use std::io::{Cursor, Write};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

/// A minimal, valid `META-INF/container.xml` pointing at `OEBPS/content.opf`.
/// Shared by fixtures that build their own OPF/chapters but still need a
/// container document.
pub const CONTAINER_XML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#;

/// A minimal EPUB3 nav document with a single TOC entry pointing at
/// `chapter.xhtml`. Shared by fixtures that need a nav doc but do not care
/// about its contents.
pub const NAV_XHTML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Nav</title></head>
<body>
<nav epub:type="toc">
<ol>
<li><a href="chapter.xhtml">InventedWatermark.example Chapter</a></li>
</ol>
</nav>
</body>
</html>"#;

/// Build a ZIP archive from `entries` (path, raw bytes), in the given order.
/// `mimetype` (if present) is written STORED (uncompressed); everything else
/// is written DEFLATE. Callers are responsible for ordering `entries` so that
/// `mimetype` comes first, matching the EPUB OCF requirement.
pub fn build_epub(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    for (name, data) in entries {
        let options = if *name == "mimetype" {
            stored
        } else {
            deflated
        };
        writer.start_file(*name, options).expect("start_file");
        writer.write_all(data).expect("write entry data");
    }
    writer.finish().expect("finish zip").into_inner()
}

/// Read one zip entry's raw bytes by name, if present.
pub fn entry(epub: &[u8], name: &str) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut zip = zip::ZipArchive::new(Cursor::new(epub)).ok()?;
    let mut file = zip.by_name(name).ok()?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).ok()?;
    Some(buf)
}

/// A real, decodable 24x24 grayscale PNG. Every other srcset fixture here
/// uses a bare 8-byte PNG *signature* that the image pipeline cannot decode,
/// so it never re-encodes it and never renames it - which is exactly why a
/// whole class of stranding went unseen. This one is a genuine image, so the
/// optimizer really does re-encode it (to `.jpg`) and really does rename it.
pub fn real_png() -> Vec<u8> {
    let img =
        image::GrayImage::from_fn(24, 24, |x, y| image::Luma([((x * 10 + y * 3) % 240) as u8]));
    let mut out = Cursor::new(Vec::new());
    image::DynamicImage::ImageLuma8(img)
        .write_to(&mut out, image::ImageFormat::Png)
        .expect("encode png");
    out.into_inner()
}
