//! Helpers shared by the CLI integration tests.
//!
//! This module is `mod`-included by more than one test binary (`cli.rs`,
//! `batch.rs`), each of which uses only a subset of the helpers, so
//! unused-in-one-binary helpers are expected.
#![allow(dead_code)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

pub fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_epub-tailor"))
}

pub fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("epub-tailor-cli-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// Build a one-chapter EPUB by running `md`, the way the other tests do.
pub fn book_in(dir: &Path, name: &str) -> PathBuf {
    let md = dir.join(format!("{name}.md"));
    std::fs::write(
        &md,
        "---\ntitle: A Book\nauthor: Jane Author\n---\n\n# One\n\nHello.\n",
    )
    .expect("write markdown");
    let out = dir.join(format!("{name}.epub"));
    let status = bin()
        .args(["md", md.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .expect("failed to run binary");
    assert!(status.status.success(), "md should build a book");
    out
}

/// A hand-built, minimal, valid EPUB3 book carrying one extra META-INF file
/// (e.g. `META-INF/cdp.info`) with the given payload - the shape `md` cannot
/// produce, needed to exercise the default human report's per-file
/// `meta-inf-dropped` output end to end through the real binary.
pub fn book_with_meta_inf_in(
    dir: &Path,
    name: &str,
    meta_inf_path: &str,
    payload: &str,
) -> PathBuf {
    const CONTAINER_XML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#;
    const CONTENT_OPF: &[u8] = br##"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="pub-id">urn:uuid:eeee1111-2222-4333-8444-555555555555</dc:identifier>
    <dc:title>Book</dc:title>
    <dc:language>en</dc:language>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ch" href="chapter.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine><itemref idref="ch"/></spine>
</package>"##;
    const NAV_XHTML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Nav</title></head>
<body><nav epub:type="toc"><ol><li><a href="chapter.xhtml">Chapter</a></li></ol></nav></body>
</html>"#;
    const CHAPTER: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>C</title></head>
<body><p>Text.</p></body></html>"#;

    let mut writer = ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let entries: [(&str, &[u8]); 5] = [
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER_XML),
        (meta_inf_path, payload.as_bytes()),
        ("OEBPS/content.opf", CONTENT_OPF),
        ("OEBPS/nav.xhtml", NAV_XHTML),
    ];
    for (entry_name, data) in entries {
        let options = if entry_name == "mimetype" {
            stored
        } else {
            deflated
        };
        writer.start_file(entry_name, options).expect("start_file");
        writer.write_all(data).expect("write entry data");
    }
    writer
        .start_file("OEBPS/chapter.xhtml", deflated)
        .expect("start_file");
    writer.write_all(CHAPTER).expect("write chapter");
    let bytes = writer.finish().expect("finish zip").into_inner();

    let out = dir.join(format!("{name}.epub"));
    std::fs::write(&out, bytes).expect("write fixture epub");
    out
}
