//! Helpers shared by the CLI integration tests.
//!
//! This module is `mod`-included by more than one test binary (`cli.rs`,
//! `batch.rs`), each of which uses only a subset of the helpers, so
//! unused-in-one-binary helpers are expected.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

// The same fixture primitives `epub-tailor-core`'s integration tests build on,
// shared through a dev-only crate because an integration test cannot import
// another crate's test module.
use epub_tailor_testfixtures::{CONTAINER_XML, build_epub, real_png};

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
    // Local nav doc: the shared `NAV_XHTML` titles its one entry with the
    // watermark string the core tests scrub for, which would show up in the
    // human report this fixture exists to exercise.
    const NAV: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Nav</title></head>
<body><nav epub:type="toc"><ol><li><a href="chapter.xhtml">Chapter</a></li></ol></nav></body>
</html>"#;
    const CHAPTER: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>C</title></head>
<body><p>Text.</p></body></html>"#;

    let bytes = build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER_XML),
        (meta_inf_path, payload.as_bytes()),
        ("OEBPS/content.opf", CONTENT_OPF),
        ("OEBPS/nav.xhtml", NAV),
        ("OEBPS/chapter.xhtml", CHAPTER),
    ]);

    let out = dir.join(format!("{name}.epub"));
    std::fs::write(&out, bytes).expect("write fixture epub");
    out
}

/// A hand-built EPUB3 carrying every per-copy signal `check --profile generic`
/// reports: a per-copy `dc:identifier` as the unique one, a second one shaped
/// like an email, a zero-width space in prose, and a manifested image nothing
/// references. `md` cannot produce any of this, and the point of the fixture
/// is to exercise the real binary's JSON contract end to end.
pub fn marked_book_in(dir: &Path, name: &str) -> PathBuf {
    const CONTENT_OPF: &[u8] = br##"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="pub-id">SHTX001.635962014</dc:identifier>
    <dc:identifier>reader@example.com</dc:identifier>
    <dc:title>A Marked Book</dc:title>
    <dc:language>en</dc:language>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ch" href="chapter.xhtml" media-type="application/xhtml+xml"/>
    <item id="orphan" href="orphan.png" media-type="image/png"/>
  </manifest>
  <spine><itemref idref="ch"/></spine>
</package>"##;
    // A literal U+200B between "So" and "long".
    const CHAPTER: &[u8] = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>C</title></head>\n\
<body><p>So\u{200B} long.</p></body></html>"
        .as_bytes();
    const NAV: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Nav</title></head>
<body><nav epub:type="toc"><ol><li><a href="chapter.xhtml">One</a></li></ol></nav></body></html>"#;

    let bytes = build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER_XML),
        ("OEBPS/content.opf", CONTENT_OPF),
        ("OEBPS/nav.xhtml", NAV),
        ("OEBPS/chapter.xhtml", CHAPTER),
        ("OEBPS/orphan.png", &real_png()),
    ]);
    let out = dir.join(format!("{name}.epub"));
    std::fs::write(&out, bytes).expect("write marked fixture");
    out
}
