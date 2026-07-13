//! End-to-end gate: convert the in-code EPUB2 and EPUB3 fixtures, write the
//! result to disk, and validate it with epubcheck. The output must be clean
//! (no errors, no warnings). If epubcheck is not available (not on `PATH` and
//! `EPUBCHECK_JAR` unset), the test SKIPs rather than failing, so the suite
//! stays green on machines without a Java/epubcheck install; CI installs
//! epubcheck so the gate runs for real there.

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use common::{epub2_minimal, epub3_kitchen_sink, epub3_minimal};
use epub_tailor_core::profile::{DeviceCaps, Features};
use epub_tailor_core::{ConvertOptions, Input, convert};

/// Run epubcheck against `path`, preferring the `epubcheck` launcher on `PATH`
/// and falling back to `java -jar $EPUBCHECK_JAR`. Returns `None` if neither is
/// available.
fn run_epubcheck(path: &Path) -> Option<Output> {
    if let Ok(output) = Command::new("epubcheck").arg(path).output() {
        return Some(output);
    }
    if let Ok(jar) = std::env::var("EPUBCHECK_JAR")
        && let Ok(output) = Command::new("java").arg("-jar").arg(jar).arg(path).output()
    {
        return Some(output);
    }
    None
}

fn assert_clean_roundtrip_with(name: &str, epub: Vec<u8>, opts: &ConvertOptions) {
    let converted = convert(Input::Epub(epub), opts).expect("conversion should succeed");
    assert!(
        !converted.epub.is_empty(),
        "converted EPUB for {name} should not be empty"
    );

    let out_path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.tailored.epub"));
    std::fs::write(&out_path, &converted.epub).expect("write converted epub to temp dir");

    match run_epubcheck(&out_path) {
        None => {
            eprintln!(
                "SKIP: epubcheck not found (not on PATH, EPUBCHECK_JAR unset); \
                 skipping the {name} round-trip validation"
            );
        }
        Some(output) => {
            let combined = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
            // epubcheck localizes its summary text, but every FATAL/ERROR/WARNING
            // message keeps its English severity prefix and message code (e.g.
            // `ERROR(PKG-021)`), so checking for those is locale-independent.
            let offenders: Vec<&str> = combined
                .lines()
                .filter(|line| {
                    line.contains("FATAL(") || line.contains("ERROR(") || line.contains("WARNING(")
                })
                .collect();
            assert!(
                offenders.is_empty(),
                "epubcheck reported {} problem(s) for {name} (status {:?}):\n{}",
                offenders.len(),
                output.status.code(),
                combined,
            );
        }
    }
}

fn assert_clean_roundtrip(name: &str, epub: Vec<u8>) {
    assert_clean_roundtrip_with(name, epub, &ConvertOptions::default());
}

/// The resolved built-in `epub` (repair-only) profile as ConvertOptions.
fn repair_only_opts() -> ConvertOptions {
    ConvertOptions {
        device: DeviceCaps::permissive(),
        features: Features::repair_only(),
        ..ConvertOptions::default()
    }
}

#[test]
fn epub3_fixture_roundtrips_clean_through_epubcheck() {
    assert_clean_roundtrip("epub3", epub3_minimal());
}

#[test]
fn stamped_output_roundtrips_clean_through_epubcheck() {
    // The provenance stamp rides on a custom `tailor:` prefix; epubcheck is
    // the authority on whether that declaration is well-formed.
    let opts = ConvertOptions {
        output_stamp: Some("x4 0.0.0-test".to_string()),
        ..ConvertOptions::default()
    };
    assert_clean_roundtrip_with("epub3-stamped", epub3_minimal(), &opts);
}

#[test]
fn epub2_fixture_roundtrips_clean_through_epubcheck() {
    assert_clean_roundtrip("epub2", epub2_minimal());
}

#[test]
fn epub3_fixture_roundtrips_clean_under_the_repair_profile() {
    assert_clean_roundtrip_with("epub3-repair", epub3_minimal(), &repair_only_opts());
}

#[test]
fn epub2_fixture_roundtrips_clean_under_the_repair_profile() {
    assert_clean_roundtrip_with("epub2-repair", epub2_minimal(), &repair_only_opts());
}

#[test]
fn kitchen_sink_roundtrips_clean_under_the_repair_profile() {
    // Tables, asides, figures, code blocks and definition lists all pass
    // through the repair profile; the result must still be a valid EPUB.
    assert_clean_roundtrip_with("kitchen-repair", epub3_kitchen_sink(), &repair_only_opts());
}
