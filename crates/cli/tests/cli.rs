use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_epub-tailor"))
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("epub-tailor-cli-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

#[test]
fn help_exits_zero_and_mentions_all_subcommands() {
    let output = bin().arg("--help").output().expect("failed to run binary");
    assert!(output.status.success(), "--help should exit 0");
    let stdout = String::from_utf8_lossy(&output.stdout);
    for subcommand in ["fit", "md", "check", "profiles"] {
        assert!(
            stdout.contains(subcommand),
            "--help output should mention `{subcommand}`, got:\n{stdout}"
        );
    }
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
    assert_eq!(json["name"], "x4");
    assert_eq!(json["appendix"], "x4");
    assert_eq!(json["features"]["strip_fonts"], true);
    assert_eq!(json["caps"]["screen_w"], 480);
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
