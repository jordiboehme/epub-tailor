mod common;

use std::path::{Path, PathBuf};

use common::{bin, book_in, temp_dir};

/// A real, tiny, valid baseline JPEG (same fixture style as the core tests).
const TINY_JPEG: &[u8] = &[
    0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00, 0x01,
    0x00, 0x01, 0x00, 0x00, 0xFF, 0xDB, 0x00, 0x43, 0x00, 0x06, 0x04, 0x05, 0x06, 0x05, 0x04, 0x06,
    0x06, 0x05, 0x06, 0x07, 0x07, 0x06, 0x08, 0x0A, 0x10, 0x0A, 0x0A, 0x09, 0x09, 0x0A, 0x14, 0x0E,
    0x0F, 0x0C, 0x10, 0x17, 0x14, 0x18, 0x18, 0x17, 0x14, 0x16, 0x16, 0x1A, 0x1D, 0x25, 0x1F, 0x1A,
    0x1B, 0x23, 0x1C, 0x16, 0x16, 0x20, 0x2C, 0x20, 0x23, 0x26, 0x27, 0x29, 0x2A, 0x29, 0x19, 0x1F,
    0x2D, 0x30, 0x2D, 0x28, 0x30, 0x25, 0x28, 0x29, 0x28, 0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x02,
    0x00, 0x02, 0x01, 0x01, 0x11, 0x00, 0xFF, 0xC4, 0x00, 0x1F, 0x00, 0x00, 0x01, 0x05, 0x01, 0x01,
    0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x02, 0x03, 0x04,
    0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0xFF, 0xC4, 0x00, 0xB5, 0x10, 0x00, 0x02, 0x01, 0x03,
    0x03, 0x02, 0x04, 0x03, 0x05, 0x05, 0x04, 0x04, 0x00, 0x00, 0x01, 0x7D, 0x01, 0x02, 0x03, 0x00,
    0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07, 0x22, 0x71, 0x14, 0x32,
    0x81, 0x91, 0xA1, 0x08, 0x23, 0x42, 0xB1, 0xC1, 0x15, 0x52, 0xD1, 0xF0, 0x24, 0x33, 0x62, 0x72,
    0x82, 0x09, 0x0A, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2A, 0x34, 0x35,
    0x36, 0x37, 0x38, 0x39, 0x3A, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4A, 0x53, 0x54, 0x55,
    0x56, 0x57, 0x58, 0x59, 0x5A, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6A, 0x73, 0x74, 0x75,
    0x76, 0x77, 0x78, 0x79, 0x7A, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8A, 0x92, 0x93, 0x94,
    0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xB2,
    0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xC2, 0xC3, 0xC4, 0xC5, 0xC6, 0xC7, 0xC8, 0xC9,
    0xCA, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6, 0xD7, 0xD8, 0xD9, 0xDA, 0xE1, 0xE2, 0xE3, 0xE4, 0xE5, 0xE6,
    0xE7, 0xE8, 0xE9, 0xEA, 0xF1, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8, 0xF9, 0xFA, 0xFF, 0xDA,
    0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3F, 0x00, 0xF6, 0x0A, 0xFF, 0xD9,
];

/// Build a one-chapter EPUB with a cover image, by running `md --cover`.
fn book_with_cover_in(dir: &Path, name: &str) -> PathBuf {
    let md = dir.join(format!("{name}.md"));
    std::fs::write(
        &md,
        "---\ntitle: A Book\nauthor: Jane Author\n---\n\n# One\n\nHello.\n",
    )
    .expect("write markdown");
    let cover = dir.join(format!("{name}-cover.jpg"));
    std::fs::write(&cover, TINY_JPEG).expect("write cover fixture");
    let out = dir.join(format!("{name}.epub"));
    let status = bin()
        .args([
            "md",
            md.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--cover",
            cover.to_str().unwrap(),
        ])
        .output()
        .expect("failed to run binary");
    assert!(
        status.status.success(),
        "md should build a book with a cover"
    );
    out
}

#[test]
fn help_exits_zero_and_mentions_all_subcommands() {
    let output = bin().arg("--help").output().expect("failed to run binary");
    assert!(output.status.success(), "--help should exit 0");
    let stdout = String::from_utf8_lossy(&output.stdout);
    for subcommand in ["fit", "md", "check", "profiles", "metadata"] {
        assert!(
            stdout.contains(subcommand),
            "--help output should mention `{subcommand}`, got:\n{stdout}"
        );
    }
}

#[test]
fn metadata_show_reports_what_the_book_lacks() {
    let dir = temp_dir("meta-show");
    let book = book_in(&dir, "show");

    let output = bin()
        .args([
            "metadata",
            "show",
            book.to_str().unwrap(),
            "--report",
            "json",
        ])
        .output()
        .expect("failed to run binary");
    assert!(output.status.success());

    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).expect("valid JSON");
    assert_eq!(json["schema"], 1, "every payload carries a schema version");
    assert_eq!(json["metadata"]["title"], "A Book");
    let missing: Vec<String> = json["missing"]
        .as_array()
        .expect("missing is a list")
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(missing.contains(&"description".to_string()));
    assert!(missing.contains(&"publisher".to_string()));
    assert!(!missing.contains(&"title".to_string()), "it has a title");
    assert!(
        json["fitted"].is_null(),
        "a plain source carries no fit stamp, got: {}",
        json["fitted"]
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn metadata_show_reports_the_fit_stamp_of_a_produced_copy() {
    let dir = temp_dir("meta-show-fitted");
    let book = book_in(&dir, "source");
    let out = dir.join("source.tailored.epub");

    let fit = bin()
        .args([
            "fit",
            book.to_str().unwrap(),
            "--profile",
            "epub",
            "-o",
            out.to_str().unwrap(),
        ])
        .output()
        .expect("failed to run binary");
    assert!(
        fit.status.success(),
        "fit failed: {}",
        String::from_utf8_lossy(&fit.stderr)
    );

    let output = bin()
        .args([
            "metadata",
            "show",
            out.to_str().unwrap(),
            "--report",
            "json",
        ])
        .output()
        .expect("failed to run binary");
    assert!(output.status.success());

    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).expect("valid JSON");
    let fitted = &json["fitted"];
    // The `epub` profile has no appendix of its own, so the stamp's first
    // token falls back to the default appendix while the profile meta keeps
    // the real profile name - exactly the distinction the app needs.
    assert_eq!(fitted["appendix"], "tailored", "got: {fitted}");
    assert_eq!(fitted["profile"], "epub", "got: {fitted}");
    assert_eq!(
        fitted["version"],
        env!("CARGO_PKG_VERSION"),
        "got: {fitted}"
    );
    assert_eq!(
        fitted["stamp"],
        format!("tailored {}", env!("CARGO_PKG_VERSION")),
        "got: {fitted}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn metadata_show_cover_out_writes_the_cover_and_the_json_points_at_it() {
    let dir = temp_dir("meta-show-cover");
    let book = book_with_cover_in(&dir, "show-cover");
    let cover_out = dir.join("cover-out.jpg");

    let output = bin()
        .args([
            "metadata",
            "show",
            book.to_str().unwrap(),
            "--cover-out",
            cover_out.to_str().unwrap(),
            "--report",
            "json",
        ])
        .output()
        .expect("failed to run binary");
    assert!(
        output.status.success(),
        "show failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let bytes = std::fs::read(&cover_out).expect("cover-out should have been written");
    assert!(!bytes.is_empty(), "the written cover must not be empty");

    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).expect("valid JSON");
    assert_eq!(json["metadata"]["cover"], cover_out.to_str().unwrap());

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn metadata_show_cover_out_on_a_coverless_book_warns_and_writes_nothing() {
    let dir = temp_dir("meta-show-no-cover");
    let book = book_in(&dir, "show-no-cover");
    let cover_out = dir.join("cover-out.jpg");

    let output = bin()
        .args([
            "metadata",
            "show",
            book.to_str().unwrap(),
            "--cover-out",
            cover_out.to_str().unwrap(),
            "--report",
            "json",
        ])
        .output()
        .expect("failed to run binary");
    assert!(
        output.status.success(),
        "show should still exit 0 without a cover: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("warning: this book has no cover"),
        "expected a no-cover warning, got:\n{stderr}"
    );
    assert!(
        !cover_out.exists(),
        "no file should be written when the book has no cover"
    );

    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).expect("valid JSON");
    assert!(
        json["metadata"]["cover"].is_null(),
        "metadata.cover must stay absent, got: {json}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn metadata_flags_land_in_the_book() {
    let dir = temp_dir("meta-flags");
    let book = book_in(&dir, "flags");
    let out = dir.join("out.epub");

    let output = bin()
        .args([
            "fit",
            book.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--publisher",
            "Acme Press",
            "--description",
            "A blurb.",
            "--subject",
            "Fantasy",
            "--isbn",
            "9780261102217",
        ])
        .output()
        .expect("failed to run binary");
    assert!(
        output.status.success(),
        "fit failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Read it back through `metadata show` - the tool's own view of the book.
    let shown = bin()
        .args([
            "metadata",
            "show",
            out.to_str().unwrap(),
            "--report",
            "json",
        ])
        .output()
        .expect("failed to run binary");
    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&shown.stdout)).expect("valid JSON");
    assert_eq!(json["metadata"]["publisher"], "Acme Press");
    assert_eq!(json["metadata"]["description"], "A blurb.");
    assert_eq!(json["metadata"]["subjects"][0], "Fantasy");
    // The ISBN is a *secondary* identifier; the book's own id is untouched.
    assert_eq!(json["metadata"]["identifiers"][0]["value"], "9780261102217");
    assert!(
        json["metadata"]["identifier"]
            .as_str()
            .unwrap()
            .starts_with("urn:epub-tailor:"),
        "the unique identifier must not be replaced by the ISBN"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_metadata_document_can_arrive_on_stdin() {
    // This is the pipe the whole design rests on:
    //   metadata fetch REF | fit book.epub --metadata -
    use std::io::Write;
    use std::process::Stdio;

    let dir = temp_dir("meta-stdin");
    let book = book_in(&dir, "stdin");
    let out = dir.join("out.epub");

    let mut child = bin()
        .args([
            "fit",
            book.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--metadata",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(br#"{"publisher": "Piped Press"}"#)
        .expect("write stdin");
    let status = child.wait().expect("wait");
    assert!(status.success(), "fit --metadata - should succeed");

    let shown = bin()
        .args([
            "metadata",
            "show",
            out.to_str().unwrap(),
            "--report",
            "json",
        ])
        .output()
        .expect("failed to run binary");
    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&shown.stdout)).expect("valid JSON");
    assert_eq!(json["metadata"]["publisher"], "Piped Press");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn metadata_pick_refuses_to_prompt_when_stdin_is_not_a_terminal() {
    // The quarantine that makes every other command safe for a GUI: the one
    // interactive command must fail fast rather than hang waiting for an answer
    // nobody is there to give.
    use std::process::Stdio;

    let dir = temp_dir("meta-pick");
    let book = book_in(&dir, "pick");

    let output = bin()
        .args(["metadata", "pick", book.to_str().unwrap()])
        .stdin(Stdio::null())
        .output()
        .expect("failed to run binary");

    assert_eq!(
        output.status.code(),
        Some(1),
        "pick must refuse, not hang or succeed"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not a terminal"),
        "it should say why, got: {stderr}"
    );
    assert!(
        stderr.contains("metadata search"),
        "and point at the non-interactive route, got: {stderr}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_failure_is_machine_readable_under_report_json() {
    // Without this a GUI's only way to tell "the book has DRM" from "the file is
    // missing" is to grep English prose off stderr.
    let dir = temp_dir("meta-err");
    let junk = dir.join("junk.epub");
    std::fs::write(&junk, b"not an epub at all").expect("write junk");

    let output = bin()
        .args(["fit", junk.to_str().unwrap(), "--report", "json"])
        .output()
        .expect("failed to run binary");
    assert_eq!(output.status.code(), Some(1));

    let json: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&output.stdout))
        .expect("a failure must still emit valid JSON on stdout");
    assert_eq!(json["schema"], 1);
    assert!(
        json["error"]["code"].is_string(),
        "the error must carry a stable code, got: {json}"
    );
    assert!(json["error"]["message"].is_string());
    // ...and the prose still goes to stderr for a human.
    assert!(String::from_utf8_lossy(&output.stderr).starts_with("error:"));

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_json_report_says_where_the_book_landed() {
    // A GUI cannot otherwise learn the output path without reimplementing the
    // naming rule.
    let dir = temp_dir("meta-out");
    let book = book_in(&dir, "outpath");
    let out = dir.join("named.epub");

    let output = bin()
        .args([
            "fit",
            book.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--report",
            "json",
        ])
        .output()
        .expect("failed to run binary");
    assert!(output.status.success());

    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).expect("valid JSON");
    assert_eq!(json["schema"], 1);
    assert_eq!(json["output"], out.to_str().unwrap());
    assert_eq!(json["dry_run"], false);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_builtin_profile_list_is_available_as_json() {
    let output = bin()
        .args(["profiles", "--report", "json"])
        .output()
        .expect("failed to run binary");
    assert!(output.status.success());

    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).expect("valid JSON");
    assert_eq!(json["schema"], 1);
    let profiles = json["profiles"].as_array().expect("a list of profiles");
    assert!(profiles.len() >= 13, "got {} profiles", profiles.len());
    assert!(
        profiles.iter().any(|p| p["name"] == "kobo-clara-bw"),
        "the list should carry every built-in"
    );
}

#[test]
fn version_flag_exits_zero() {
    let output = bin()
        .arg("--version")
        .output()
        .expect("failed to run binary");
    assert!(output.status.success(), "--version should exit 0");
}

#[test]
fn profiles_exits_zero_and_lists_builtins() {
    let output = bin()
        .arg("profiles")
        .output()
        .expect("failed to run binary");
    assert!(output.status.success(), "profiles should exit 0");
    let stdout = String::from_utf8_lossy(&output.stdout);
    for expected in ["epub", "x4", "x3", "480", "tailored"] {
        assert!(
            stdout.contains(expected),
            "profiles output should mention {expected}, got:\n{stdout}"
        );
    }
}

#[test]
fn profiles_with_specs_prints_the_resolved_composition_as_json() {
    let output = bin()
        .args(["profiles", "x4"])
        .output()
        .expect("failed to run binary");
    assert!(output.status.success(), "profiles x4 should exit 0");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    // The payload is versioned now, like every other JSON this tool emits, so a
    // GUI can pin the shape instead of the binary's release number.
    assert_eq!(json["schema"], 1);
    let profile = &json["profile"];
    assert_eq!(profile["name"], "x4");
    assert_eq!(profile["appendix"], "x4");
    assert_eq!(profile["features"]["strip_fonts"], true);
    assert_eq!(profile["caps"]["screen_w"], 480);
}

#[test]
fn fit_on_missing_file_exits_1() {
    let output = bin()
        .args(["fit", "definitely-missing.epub"])
        .output()
        .expect("failed to run binary");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cannot read"),
        "expected a read error on stderr, got:\n{stderr}"
    );
}

#[test]
fn optimize_and_clean_are_hidden_aliases_for_fit() {
    for alias in ["optimize", "clean"] {
        let output = bin()
            .args([alias, "definitely-missing.epub"])
            .output()
            .expect("failed to run binary");
        assert_eq!(
            output.status.code(),
            Some(1),
            "`{alias}` should behave like `fit` (runtime read error, not a usage error)"
        );
    }
}

#[test]
fn fit_rejects_an_unknown_profile_with_exit_1() {
    let output = bin()
        .args(["fit", "book.epub", "--profile", "x5"])
        .output()
        .expect("failed to run binary");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unknown profile"),
        "expected an unknown-profile error, got:\n{stderr}"
    );
    assert!(
        stderr.contains("x4"),
        "the error should list the built-ins, got:\n{stderr}"
    );
}

#[test]
fn fit_default_output_uses_the_profile_appendix() {
    let dir = temp_dir("appendix");
    let md = dir.join("book.md");
    std::fs::write(&md, "# Hello World\n\nJust a paragraph.\n").expect("write fixture");
    let md_out = bin()
        .args(["md", md.to_str().unwrap()])
        .output()
        .expect("failed to run binary");
    assert!(md_out.status.success(), "md conversion should succeed");
    let epub = dir.join("book.epub");

    // No profile: the generic `tailored` appendix.
    let output = bin()
        .args(["fit", epub.to_str().unwrap()])
        .output()
        .expect("failed to run binary");
    assert!(
        output.status.success(),
        "fit should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        dir.join("book.tailored.epub").exists(),
        "expected book.tailored.epub next to the input"
    );

    // x4 profile: its own appendix.
    let output = bin()
        .args(["fit", epub.to_str().unwrap(), "--profile", "x4"])
        .output()
        .expect("failed to run binary");
    assert!(
        output.status.success(),
        "fit --profile x4 should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        dir.join("book.x4.epub").exists(),
        "expected book.x4.epub next to the input"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn md_on_missing_file_exits_1() {
    let output = bin()
        .args(["md", "definitely-missing.md"])
        .output()
        .expect("failed to run binary");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cannot read"),
        "expected a read error on stderr, got:\n{stderr}"
    );
}

#[test]
fn md_converts_a_minimal_file_and_writes_an_epub_next_to_it() {
    let dir = temp_dir("md");
    let input = dir.join("book.md");
    std::fs::write(&input, "# Hello World\n\nJust a paragraph.\n").expect("write fixture");

    let output = bin()
        .args(["md", input.to_str().unwrap()])
        .output()
        .expect("failed to run binary");
    assert!(
        output.status.success(),
        "expected success, got {:?}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    let expected_output = dir.join("book.epub");
    assert!(
        expected_output.exists(),
        "expected {} to be written",
        expected_output.display()
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("book.epub"),
        "expected the report to mention the output file, got:\n{stdout}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn fit_help_lists_all_table_modes() {
    let output = bin()
        .args(["fit", "--help"])
        .output()
        .expect("failed to run binary");
    assert!(output.status.success(), "fit --help should exit 0");
    let stdout = String::from_utf8_lossy(&output.stdout);
    for mode in ["text", "image", "image-all"] {
        assert!(
            stdout.contains(mode),
            "--tables help should list `{mode}`, got:\n{stdout}"
        );
    }
}

#[test]
fn remap_colors_takes_a_bool_and_rejects_anything_else() {
    let output = bin()
        .args(["fit", "book.epub", "--remap-colors", "banana"])
        .output()
        .expect("failed to run binary");
    assert_eq!(output.status.code(), Some(2), "clap usage error exits 2");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("invalid value 'banana'"),
        "expected a value error on stderr, got:\n{stderr}"
    );
}

#[test]
fn remap_colors_override_is_accepted_in_both_directions() {
    let dir = temp_dir("remap-colors");
    let book = book_in(&dir, "plain");
    for value in ["true", "false"] {
        let out_path = dir.join(format!("remap-{value}.epub"));
        let output = bin()
            .args([
                "fit",
                book.to_str().unwrap(),
                "--profile",
                "x4",
                "--remap-colors",
                value,
                "--output",
                out_path.to_str().unwrap(),
            ])
            .output()
            .expect("failed to run binary");
        assert!(
            output.status.success(),
            "`--remap-colors {value}` should be accepted, got {:?}\nstderr: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(out_path.exists());
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn tables_rejects_an_unknown_mode() {
    let output = bin()
        .args(["fit", "book.epub", "--tables", "bogus"])
        .output()
        .expect("failed to run binary");
    assert_eq!(output.status.code(), Some(2), "clap usage error exits 2");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("invalid value 'bogus'"),
        "expected a value error on stderr, got:\n{stderr}"
    );
}

#[test]
fn tables_image_and_image_all_are_accepted() {
    let dir = temp_dir("tables");
    let md = dir.join("book.md");
    std::fs::write(
        &md,
        "# Heading\n\n| A | B | C |\n|---|---|---|\n| 1 | 2 | 3 |\n",
    )
    .expect("write fixture");

    for (mode, out) in [
        ("text", "text.epub"),
        ("image", "image.epub"),
        ("image-all", "image-all.epub"),
    ] {
        let out_path = dir.join(out);
        let output = bin()
            .args([
                "md",
                md.to_str().unwrap(),
                "--profile",
                "x4",
                "--tables",
                mode,
                "--output",
                out_path.to_str().unwrap(),
            ])
            .output()
            .expect("failed to run binary");
        assert!(
            output.status.success(),
            "`--tables {mode}` should be accepted, got {:?}\nstderr: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            out_path.exists(),
            "expected {} to be written",
            out_path.display()
        );
    }

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn check_on_missing_file_exits_2() {
    let output = bin()
        .args(["check", "missing.epub"])
        .output()
        .expect("failed to run binary");
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn check_on_unparsable_bytes_exits_1_with_an_error_finding() {
    let dir = temp_dir("check-garbage");
    let bad = dir.join("bad.epub");
    std::fs::write(&bad, b"not a zip file at all").expect("write fixture");

    let output = bin()
        .args(["check", bad.to_str().unwrap(), "--report", "json"])
        .output()
        .expect("failed to run binary");
    assert_eq!(output.status.code(), Some(1));

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON report");
    assert!(json["errors"].as_u64().unwrap() >= 1);
    assert_eq!(json["warnings"].as_u64().unwrap(), 0);
    assert!(
        json["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["code"] == "unreadable"),
        "got: {json}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn check_on_a_converted_book_exits_0_with_zero_errors() {
    let dir = temp_dir("check-clean");
    let md = dir.join("book.md");
    std::fs::write(&md, "# Hello World\n\nJust a paragraph.\n").expect("write fixture");

    let md_output = bin()
        .args(["md", md.to_str().unwrap(), "--profile", "x4"])
        .output()
        .expect("failed to run binary");
    assert!(md_output.status.success(), "md conversion should succeed");

    let epub = dir.join("book.epub");
    let output = bin()
        .args([
            "check",
            epub.to_str().unwrap(),
            "--profile",
            "x4",
            "--report",
            "json",
        ])
        .output()
        .expect("failed to run binary");
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON report");
    assert_eq!(json["errors"].as_u64().unwrap(), 0, "got: {json}");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn check_human_report_has_a_one_line_summary() {
    let dir = temp_dir("check-human");
    let md = dir.join("book.md");
    std::fs::write(&md, "# Hello World\n\nJust a paragraph.\n").expect("write fixture");
    bin()
        .args(["md", md.to_str().unwrap()])
        .output()
        .expect("failed to run binary");
    let epub = dir.join("book.epub");

    let output = bin()
        .args(["check", epub.to_str().unwrap()])
        .output()
        .expect("failed to run binary");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("0 error(s)"),
        "expected a one-line summary, got:\n{stdout}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn lets_get_dangerous_replaces_the_original_in_place() {
    let dir = temp_dir("dangerous");
    let md = dir.join("book.md");
    // The ordered list survives the default (repair) md conversion but is
    // baked to text by the x4 profile, so the in-place fit provably rewrites.
    std::fs::write(&md, "# Hello World\n\n1. first\n2. second\n").expect("write fixture");
    let md_out = bin()
        .args(["md", md.to_str().unwrap()])
        .output()
        .expect("failed to run binary");
    assert!(md_out.status.success(), "md conversion should succeed");
    let epub = dir.join("book.epub");
    let original = std::fs::read(&epub).expect("read original");

    let output = bin()
        .args([
            "fit",
            epub.to_str().unwrap(),
            "--profile",
            "x4",
            "--lets-get-dangerous",
        ])
        .output()
        .expect("failed to run binary");
    assert!(
        output.status.success(),
        "fit --lets-get-dangerous should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let replaced = std::fs::read(&epub).expect("read replaced");
    assert_ne!(replaced, original, "the original file must be rewritten");
    assert!(
        replaced.starts_with(b"PK"),
        "the replacement must still be a zip archive"
    );
    assert!(
        !dir.join("book.tailored.epub").exists() && !dir.join("book.x4.epub").exists(),
        "no separate output file may be written"
    );
    assert!(
        !std::fs::read_dir(&dir).expect("list dir").any(|e| e
            .expect("entry")
            .file_name()
            .to_string_lossy()
            .contains(".tmp-")),
        "the staging temp file must not linger"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn lets_get_dangerous_conflicts_with_output() {
    let output = bin()
        .args([
            "fit",
            "book.epub",
            "--lets-get-dangerous",
            "--output",
            "elsewhere.epub",
        ])
        .output()
        .expect("failed to run binary");
    assert_eq!(
        output.status.code(),
        Some(2),
        "combining in-place replacement with -o is a usage error"
    );
}

/// Build a one-chapter EPUB whose front-matter carries a publisher and a
/// series, so there is something for `--clear` to remove.
fn book_with_rich_meta_in(dir: &Path, name: &str) -> PathBuf {
    let md = dir.join(format!("{name}.md"));
    std::fs::write(
        &md,
        "---\ntitle: A Book\nauthor: Jane Author\npublisher: Acme Press\nseries: The Cycle\nseries_index: \"2\"\n---\n\n# One\n\nHello.\n",
    )
    .expect("write markdown");
    let out = dir.join(format!("{name}.epub"));
    let status = bin()
        .args(["md", md.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .expect("failed to run binary");
    assert!(status.status.success(), "md should build the fixture book");
    out
}

#[test]
fn clear_removes_fields_and_the_report_says_so() {
    let dir = temp_dir("clear-fields");
    let book = book_with_rich_meta_in(&dir, "rich");
    let out = dir.join("out.epub");

    let output = bin()
        .args([
            "fit",
            book.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--clear",
            "publisher",
            "--clear",
            "series",
            "--report",
            "json",
        ])
        .output()
        .expect("failed to run binary");
    assert!(
        output.status.success(),
        "fit failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).expect("valid JSON");
    let kinds: Vec<&str> = report["transformations"]
        .as_array()
        .expect("transformations array")
        .iter()
        .filter_map(|t| t["kind"].as_str())
        .collect();
    assert!(
        kinds.contains(&"metadata-clear"),
        "the report must record the clears, got kinds: {kinds:?}"
    );

    // Read the output back through `metadata show` - the tool's own view.
    let shown = bin()
        .args([
            "metadata",
            "show",
            out.to_str().unwrap(),
            "--report",
            "json",
        ])
        .output()
        .expect("failed to run binary");
    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&shown.stdout)).expect("valid JSON");
    assert!(
        json["metadata"]["publisher"].is_null(),
        "publisher must be gone"
    );
    assert!(json["metadata"]["series"].is_null(), "series must be gone");
    assert_eq!(json["metadata"]["title"], "A Book", "title is untouched");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn clear_conflicts_with_setting_the_same_field() {
    let dir = temp_dir("clear-conflict");
    let book = book_in(&dir, "book");
    let out = dir.join("out.epub");

    let output = bin()
        .args([
            "fit",
            book.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--clear",
            "series",
            "--series",
            "The Cycle",
        ])
        .output()
        .expect("failed to run binary");
    assert!(
        !output.status.success(),
        "clear+set of one field must be refused"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--series"),
        "the error should name the conflicting flag, got: {stderr}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn clear_refuses_protected_fields() {
    let dir = temp_dir("clear-protected");
    let book = book_in(&dir, "book");

    let output = bin()
        .args(["fit", book.to_str().unwrap(), "--clear", "title"])
        .output()
        .expect("failed to run binary");
    assert!(!output.status.success(), "title must not be clearable");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("invalid value 'title'"),
        "clap should reject the value with its list, got: {stderr}"
    );

    std::fs::remove_dir_all(&dir).ok();
}
