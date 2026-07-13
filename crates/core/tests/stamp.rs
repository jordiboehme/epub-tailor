//! The provenance stamp: `fit` marks its output OPF so a later folder scan
//! can tell a product from a source. `md` output never carries it.

mod common;

use common::epub3_minimal;
use epub_tailor_core::{ConvertOptions, FsResolver, Input, convert, read_epub, read_stamp};

const STAMPED_META: &str = r#"<meta property="tailor:fitted">x4 9.9.9</meta>"#;
const PREFIX_DECL: &str = r#"prefix="tailor: https://github.com/jordiboehme/epub-tailor#""#;

fn stamped_opts(value: &str) -> ConvertOptions {
    ConvertOptions {
        output_stamp: Some(value.to_string()),
        ..ConvertOptions::default()
    }
}

fn opf_of(epub: &[u8]) -> String {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(epub)).expect("valid zip");
    let name = (0..archive.len())
        .map(|i| archive.by_index(i).unwrap().name().to_string())
        .find(|n| n.ends_with(".opf"))
        .expect("an OPF entry");
    let mut file = archive.by_name(&name).expect("entry exists");
    let mut buf = String::new();
    std::io::Read::read_to_string(&mut file, &mut buf).expect("utf8 opf");
    buf
}

#[test]
fn fit_options_stamp_the_opf() {
    let converted =
        convert(Input::Epub(epub3_minimal()), &stamped_opts("x4 9.9.9")).expect("convert");
    let opf = opf_of(&converted.epub);
    assert!(opf.contains(STAMPED_META), "got:\n{opf}");
    assert!(opf.contains(PREFIX_DECL), "got:\n{opf}");
}

#[test]
fn default_options_write_no_stamp() {
    let converted =
        convert(Input::Epub(epub3_minimal()), &ConvertOptions::default()).expect("convert");
    let opf = opf_of(&converted.epub);
    assert!(!opf.contains("tailor:fitted"), "got:\n{opf}");
    assert!(!opf.contains("prefix="), "got:\n{opf}");

    let markdown = convert(
        Input::Markdown {
            text: "# One\n\nHello.\n".to_string(),
            assets: Box::new(FsResolver::new(std::env::temp_dir())),
        },
        &ConvertOptions::default(),
    )
    .expect("convert markdown");
    let opf = opf_of(&markdown.epub);
    assert!(
        !opf.contains("tailor:fitted"),
        "md output is a source, not a product"
    );
    assert!(!opf.contains("prefix="), "got:\n{opf}");
}

#[test]
fn read_stamp_roundtrips() {
    let converted =
        convert(Input::Epub(epub3_minimal()), &stamped_opts("x4 9.9.9")).expect("convert");
    assert_eq!(read_stamp(&converted.epub), Some("x4 9.9.9".to_string()));
}

#[test]
fn read_stamp_is_none_on_unstamped_and_garbage() {
    let converted =
        convert(Input::Epub(epub3_minimal()), &ConvertOptions::default()).expect("convert");
    assert_eq!(read_stamp(&converted.epub), None);
    assert_eq!(read_stamp(&epub3_minimal()), None, "sources are unstamped");
    assert_eq!(read_stamp(b"not a zip at all"), None);
    assert_eq!(read_stamp(&[]), None);
}

#[test]
fn refit_replaces_the_stamp() {
    let first =
        convert(Input::Epub(epub3_minimal()), &stamped_opts("x4 9.9.9")).expect("first fit");
    let second = convert(Input::Epub(first.epub), &stamped_opts("kindle 10.0.0")).expect("refit");
    assert_eq!(read_stamp(&second.epub), Some("kindle 10.0.0".to_string()));
    let opf = opf_of(&second.epub);
    assert_eq!(
        opf.matches("tailor:fitted").count(),
        1,
        "a refit replaces the stamp, never stacks it:\n{opf}"
    );
}

#[test]
fn stamped_book_rereads_without_new_warnings() {
    let converted =
        convert(Input::Epub(epub3_minimal()), &stamped_opts("x4 9.9.9")).expect("convert");
    let read = read_epub(&converted.epub).expect("stamped output reads back");
    let offenders: Vec<String> = read
        .warnings
        .iter()
        .map(|w| w.message.clone())
        .filter(|m| m.contains("tailor") || m.contains("prefix"))
        .collect();
    assert!(offenders.is_empty(), "stamp caused warnings: {offenders:?}");
}
