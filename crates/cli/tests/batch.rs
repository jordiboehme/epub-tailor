//! Batch-mode integration tests: folder inputs, `--recursive`, skip logic,
//! `--force`, `-o` tree mirroring, dry run and the aggregate JSON report.

mod common;

use common::{bin, book_in, temp_dir};

#[test]
fn single_file_json_shape_is_unchanged() {
    // The 0.2.0 contract: a single-file run emits the flat document with a
    // top-level output path. Batch mode must not leak into it.
    let dir = temp_dir("single-json-shape");
    let book = book_in(&dir, "one");

    let output = bin()
        .args(["fit", book.to_str().unwrap(), "--report", "json"])
        .output()
        .expect("failed to run binary");
    assert!(output.status.success());

    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).expect("valid JSON");
    assert_eq!(json["schema"], 1);
    assert!(json["output"].is_string(), "got: {json}");
    assert!(json["stats"].is_object(), "got: {json}");
    assert!(
        json.get("results").is_none(),
        "a single-file run must not use the batch shape, got: {json}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn fit_batch_processes_every_epub_in_a_folder() {
    let dir = temp_dir("batch-folder");
    let lib = dir.join("lib");
    std::fs::create_dir_all(&lib).expect("create lib");
    book_in(&lib, "a");
    book_in(&lib, "b");
    std::fs::write(lib.join("notes.txt"), "not a book").expect("write txt");

    let output = bin()
        .args(["fit", lib.to_str().unwrap(), "--profile", "x4"])
        .output()
        .expect("failed to run binary");
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        lib.join("a.x4.epub").exists(),
        "a.x4.epub should be written"
    );
    assert!(
        lib.join("b.x4.epub").exists(),
        "b.x4.epub should be written"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("a.x4.epub") && stdout.contains("b.x4.epub"),
        "one line per file, got:\n{stdout}"
    );
    assert!(
        stdout.contains("2 written, 0 skipped, 0 failed"),
        "expected a summary, got:\n{stdout}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn fit_batch_skips_prior_outputs_and_existing_outputs() {
    let dir = temp_dir("batch-idempotent");
    let lib = dir.join("lib");
    std::fs::create_dir_all(&lib).expect("create lib");
    book_in(&lib, "a");
    book_in(&lib, "b");

    let first = bin()
        .args(["fit", lib.to_str().unwrap(), "--profile", "x4"])
        .output()
        .expect("failed to run binary");
    assert_eq!(first.status.code(), Some(0));
    let mtime_before = std::fs::metadata(lib.join("a.x4.epub"))
        .expect("output metadata")
        .modified()
        .expect("mtime");

    let second = bin()
        .args(["fit", lib.to_str().unwrap(), "--profile", "x4"])
        .output()
        .expect("failed to run binary");
    assert_eq!(
        second.status.code(),
        Some(0),
        "an all-skipped rerun succeeds"
    );

    let stdout = String::from_utf8_lossy(&second.stdout);
    assert!(
        stdout.contains("skipped (output of a previous run)"),
        "prior outputs get a reason, got:\n{stdout}"
    );
    assert!(
        stdout.contains("already exists, use --force"),
        "existing outputs get a reason, got:\n{stdout}"
    );
    assert!(
        stdout.contains("0 written, 4 skipped, 0 failed"),
        "nothing is reprocessed, got:\n{stdout}"
    );

    let mtime_after = std::fs::metadata(lib.join("a.x4.epub"))
        .expect("output metadata")
        .modified()
        .expect("mtime");
    assert_eq!(
        mtime_before, mtime_after,
        "the output must not be rewritten"
    );
    assert!(
        !lib.join("a.x4.x4.epub").exists(),
        "a prior output must not be reprocessed"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn fit_batch_force_reprocesses_everything() {
    let dir = temp_dir("batch-force");
    let lib = dir.join("lib");
    std::fs::create_dir_all(&lib).expect("create lib");
    book_in(&lib, "a");

    let first = bin()
        .args(["fit", lib.to_str().unwrap(), "--profile", "x4"])
        .output()
        .expect("failed to run binary");
    assert_eq!(first.status.code(), Some(0));

    let forced = bin()
        .args(["fit", lib.to_str().unwrap(), "--profile", "x4", "--force"])
        .output()
        .expect("failed to run binary");
    assert_eq!(forced.status.code(), Some(0));

    let stdout = String::from_utf8_lossy(&forced.stdout);
    assert!(
        stdout.contains("2 written, 0 skipped, 0 failed"),
        "force reprocesses the book and the prior output, got:\n{stdout}"
    );
    assert!(
        lib.join("a.x4.x4.epub").exists(),
        "force literally reprocesses a prior output into a.x4.x4.epub"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn fit_recursive_only_with_flag() {
    let dir = temp_dir("batch-recursive");
    let lib = dir.join("lib");
    let sub = lib.join("sub");
    let hidden = lib.join(".hidden");
    std::fs::create_dir_all(&sub).expect("create sub");
    std::fs::create_dir_all(&hidden).expect("create hidden");
    book_in(&lib, "top");
    book_in(&sub, "deep");
    book_in(&hidden, "c");

    let flat = bin()
        .args(["fit", lib.to_str().unwrap(), "--profile", "x4"])
        .output()
        .expect("failed to run binary");
    assert_eq!(flat.status.code(), Some(0));
    assert!(lib.join("top.x4.epub").exists());
    assert!(
        !sub.join("deep.x4.epub").exists(),
        "subfolders need --recursive"
    );

    let deep = bin()
        .args(["fit", lib.to_str().unwrap(), "--profile", "x4", "-r"])
        .output()
        .expect("failed to run binary");
    assert_eq!(deep.status.code(), Some(0));
    assert!(
        sub.join("deep.x4.epub").exists(),
        "-r must reach nested folders"
    );
    assert!(
        !hidden.join("c.x4.epub").exists(),
        "dot-folders are never scanned"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn fit_batch_continues_past_failures_and_exits_1() {
    let dir = temp_dir("batch-failures");
    let lib = dir.join("lib");
    std::fs::create_dir_all(&lib).expect("create lib");
    book_in(&lib, "good");
    std::fs::write(lib.join("junk.epub"), b"not an epub at all").expect("write junk");

    let output = bin()
        .args(["fit", lib.to_str().unwrap(), "--profile", "x4"])
        .output()
        .expect("failed to run binary");
    assert_eq!(
        output.status.code(),
        Some(1),
        "a failed file must fail the run"
    );
    assert!(
        lib.join("good.x4.epub").exists(),
        "the good book is still converted"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("1 written, 0 skipped, 1 failed"),
        "the summary counts the failure, got:\n{stdout}"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("error:") && stderr.contains("junk.epub"),
        "the failure lands on stderr with its file, got:\n{stderr}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn multiple_positional_inputs_use_batch_mode() {
    let dir = temp_dir("batch-multi");
    let a = book_in(&dir, "a");
    let b = book_in(&dir, "b");

    let output = bin()
        .args([
            "fit",
            a.to_str().unwrap(),
            b.to_str().unwrap(),
            "--profile",
            "x4",
        ])
        .output()
        .expect("failed to run binary");
    assert_eq!(output.status.code(), Some(0));
    assert!(dir.join("a.x4.epub").exists());
    assert!(dir.join("b.x4.epub").exists());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("2 written, 0 skipped, 0 failed"),
        "several files use the batch report, got:\n{stdout}"
    );

    // Explicitly named files are never skipped: the user asked for them.
    let again = bin()
        .args([
            "fit",
            a.to_str().unwrap(),
            b.to_str().unwrap(),
            "--profile",
            "x4",
        ])
        .output()
        .expect("failed to run binary");
    assert_eq!(again.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&again.stdout);
    assert!(
        stdout.contains("2 written, 0 skipped, 0 failed"),
        "named files are reprocessed even when their output exists, got:\n{stdout}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn fit_batch_output_dir_mirrors_tree() {
    let dir = temp_dir("batch-mirror");
    let lib = dir.join("lib");
    let sub = lib.join("sub");
    std::fs::create_dir_all(&sub).expect("create sub");
    book_in(&lib, "top");
    book_in(&sub, "deep");
    let out = dir.join("out");

    let output = bin()
        .args([
            "fit",
            lib.to_str().unwrap(),
            "-r",
            "-o",
            out.to_str().unwrap(),
            "--profile",
            "x4",
        ])
        .output()
        .expect("failed to run binary");
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        out.join("top.x4.epub").exists(),
        "top-level output lands in the output folder"
    );
    assert!(
        out.join("sub/deep.x4.epub").exists(),
        "nested output mirrors the input tree"
    );
    assert!(
        !lib.join("top.x4.epub").exists() && !sub.join("deep.x4.epub").exists(),
        "the input tree must gain no files"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn fit_batch_output_must_be_a_directory() {
    let dir = temp_dir("batch-out-file");
    let lib = dir.join("lib");
    std::fs::create_dir_all(&lib).expect("create lib");
    book_in(&lib, "a");
    let not_a_dir = dir.join("occupied.epub");
    std::fs::write(&not_a_dir, b"already here").expect("write blocker");

    let output = bin()
        .args([
            "fit",
            lib.to_str().unwrap(),
            "-o",
            not_a_dir.to_str().unwrap(),
            "--profile",
            "x4",
        ])
        .output()
        .expect("failed to run binary");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("must be a folder"),
        "the error should explain the -o rule, got:\n{stderr}"
    );
    assert!(
        !lib.join("a.x4.epub").exists(),
        "nothing may be processed after a bad -o"
    );
    assert_eq!(
        std::fs::read(&not_a_dir).expect("read blocker"),
        b"already here",
        "the blocking file must not be touched"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn fit_batch_colliding_outputs_fail_instead_of_clobbering() {
    let dir = temp_dir("batch-collision");
    let one = dir.join("one");
    let two = dir.join("two");
    std::fs::create_dir_all(&one).expect("create one");
    std::fs::create_dir_all(&two).expect("create two");
    let a1 = book_in(&one, "same");
    let a2 = book_in(&two, "same");
    let out = dir.join("out");

    let output = bin()
        .args([
            "fit",
            a1.to_str().unwrap(),
            a2.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--profile",
            "x4",
        ])
        .output()
        .expect("failed to run binary");
    assert_eq!(
        output.status.code(),
        Some(1),
        "a collision is a per-file failure"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("1 written, 0 skipped, 1 failed"),
        "the first file wins, the second fails, got:\n{stdout}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// Every path under `root`, sorted, for before/after comparisons.
fn tree_listing(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    fn walk(dir: &std::path::Path, into: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("read dir") {
            let path = entry.expect("entry").path();
            into.push(path.clone());
            if path.is_dir() {
                walk(&path, into);
            }
        }
    }
    let mut listing = Vec::new();
    walk(root, &mut listing);
    listing.sort();
    listing
}

#[test]
fn fit_batch_dry_run_writes_nothing() {
    let dir = temp_dir("batch-dry-run");
    let lib = dir.join("lib");
    let sub = lib.join("sub");
    std::fs::create_dir_all(&sub).expect("create sub");
    book_in(&lib, "top");
    book_in(&sub, "deep");
    let out = dir.join("out");
    let before = tree_listing(&dir);

    let output = bin()
        .args([
            "fit",
            lib.to_str().unwrap(),
            "-r",
            "--dry-run",
            "-o",
            out.to_str().unwrap(),
            "--profile",
            "x4",
        ])
        .output()
        .expect("failed to run binary");
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert_eq!(
        tree_listing(&dir),
        before,
        "a dry run must not touch the filesystem"
    );
    assert!(!out.exists(), "not even the -o folder may be created");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("(dry run)"),
        "every would-be write is marked, got:\n{stdout}"
    );
    assert!(
        stdout.contains("2 would be written, 0 skipped, 0 failed"),
        "the summary speaks in the conditional, got:\n{stdout}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn fit_single_file_dry_run_writes_nothing() {
    let dir = temp_dir("single-dry-run");
    let book = book_in(&dir, "solo");
    let before = tree_listing(&dir);

    let output = bin()
        .args([
            "fit",
            book.to_str().unwrap(),
            "--profile",
            "x4",
            "--dry-run",
        ])
        .output()
        .expect("failed to run binary");
    assert_eq!(output.status.code(), Some(0));

    assert_eq!(
        tree_listing(&dir),
        before,
        "a single-file dry run must not write the output"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("dry run: no output written"),
        "got:\n{stdout}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn fit_batch_json_is_one_aggregate_document() {
    let dir = temp_dir("batch-json");
    let lib = dir.join("lib");
    std::fs::create_dir_all(&lib).expect("create lib");
    book_in(&lib, "good");
    std::fs::write(lib.join("junk.epub"), b"not an epub at all").expect("write junk");

    let output = bin()
        .args([
            "fit",
            lib.to_str().unwrap(),
            "--profile",
            "x4",
            "--report",
            "json",
        ])
        .output()
        .expect("failed to run binary");
    assert_eq!(output.status.code(), Some(1), "the junk file fails the run");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .expect("stdout must carry exactly one JSON document, even with failures");
    assert_eq!(json["schema"], 1);
    assert_eq!(json["dry_run"], false);

    let results = json["results"].as_array().expect("a results array");
    let converted = results
        .iter()
        .find(|r| r["status"] == "converted")
        .expect("a converted entry");
    assert!(converted["output"].is_string());
    assert!(converted["stats"].is_object());
    let failed = results
        .iter()
        .find(|r| r["status"] == "failed")
        .expect("the failure is a result, not a separate document");
    assert!(failed["error"]["code"].is_string());
    assert_eq!(json["summary"]["converted"], 1);
    assert_eq!(json["summary"]["failed"], 1);

    // A rerun reports its skips in the same shape.
    let rerun = bin()
        .args([
            "fit",
            lib.to_str().unwrap(),
            "--profile",
            "x4",
            "--report",
            "json",
        ])
        .output()
        .expect("failed to run binary");
    let stdout = String::from_utf8_lossy(&rerun.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("one JSON document");
    let results = json["results"].as_array().expect("a results array");
    assert!(
        results.iter().any(|r| r["reason"] == "output-exists"),
        "got: {json}"
    );
    assert!(
        results.iter().any(|r| r["reason"] == "prior-output"),
        "got: {json}"
    );
    assert_eq!(json["summary"]["skipped"], 2);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn md_batch_converts_and_skips_on_rerun() {
    let dir = temp_dir("md-batch");
    let lib = dir.join("lib");
    std::fs::create_dir_all(&lib).expect("create lib");
    std::fs::write(lib.join("one.md"), "# One\n\nHello.\n").expect("write md");
    std::fs::write(lib.join("two.md"), "# Two\n\nWorld.\n").expect("write md");

    let first = bin()
        .args(["md", lib.to_str().unwrap()])
        .output()
        .expect("failed to run binary");
    assert_eq!(
        first.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(lib.join("one.epub").exists());
    assert!(lib.join("two.epub").exists());
    assert!(
        String::from_utf8_lossy(&first.stdout).contains("2 written, 0 skipped, 0 failed"),
        "got:\n{}",
        String::from_utf8_lossy(&first.stdout)
    );

    let rerun = bin()
        .args(["md", lib.to_str().unwrap()])
        .output()
        .expect("failed to run binary");
    assert_eq!(rerun.status.code(), Some(0));
    assert!(
        String::from_utf8_lossy(&rerun.stdout).contains("0 written, 2 skipped, 0 failed"),
        "a rerun writes nothing, got:\n{}",
        String::from_utf8_lossy(&rerun.stdout)
    );

    let forced = bin()
        .args(["md", lib.to_str().unwrap(), "--force"])
        .output()
        .expect("failed to run binary");
    assert_eq!(forced.status.code(), Some(0));
    assert!(
        String::from_utf8_lossy(&forced.stdout).contains("2 written, 0 skipped, 0 failed"),
        "force redoes both, got:\n{}",
        String::from_utf8_lossy(&forced.stdout)
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn check_batch_exit_codes() {
    let dir = temp_dir("check-batch-codes");
    let lib = dir.join("lib");
    std::fs::create_dir_all(&lib).expect("create lib");
    book_in(&lib, "a");
    book_in(&lib, "b");

    let clean = bin()
        .args(["check", lib.to_str().unwrap()])
        .output()
        .expect("failed to run binary");
    assert_eq!(
        clean.status.code(),
        Some(0),
        "clean books exit 0, stderr: {}",
        String::from_utf8_lossy(&clean.stderr)
    );

    std::fs::write(lib.join("garbage.epub"), b"not a zip file at all").expect("write garbage");
    let findings = bin()
        .args(["check", lib.to_str().unwrap()])
        .output()
        .expect("failed to run binary");
    assert_eq!(
        findings.status.code(),
        Some(1),
        "error findings exit 1, stdout: {}",
        String::from_utf8_lossy(&findings.stdout)
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let locked = lib.join("locked.epub");
        std::fs::write(&locked, b"whatever").expect("write locked");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000))
            .expect("chmod 000");

        let unreadable = bin()
            .args(["check", lib.to_str().unwrap()])
            .output()
            .expect("failed to run binary");
        assert_eq!(
            unreadable.status.code(),
            Some(2),
            "an unreadable input trumps everything, stdout: {}",
            String::from_utf8_lossy(&unreadable.stdout)
        );

        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o644)).ok();
    }

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn check_batch_skips_prior_outputs() {
    let dir = temp_dir("check-batch-skips");
    let lib = dir.join("lib");
    std::fs::create_dir_all(&lib).expect("create lib");
    let book = book_in(&lib, "a");
    std::fs::copy(&book, lib.join("a.x4.epub")).expect("copy prior output");

    let output = bin()
        .args(["check", lib.to_str().unwrap()])
        .output()
        .expect("failed to run binary");
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("skipped (output of a previous run)"),
        "the skip is reported, got:\n{stdout}"
    );

    let json_run = bin()
        .args(["check", lib.to_str().unwrap(), "--report", "json"])
        .output()
        .expect("failed to run binary");
    let stdout = String::from_utf8_lossy(&json_run.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("one JSON document");
    assert_eq!(json["schema"], 1);
    let results = json["results"].as_array().expect("a results array");
    assert!(
        results
            .iter()
            .any(|r| r["status"] == "checked" && r["findings"].is_array()),
        "got: {json}"
    );
    assert!(
        results
            .iter()
            .any(|r| r["status"] == "skipped" && r["reason"] == "prior-output"),
        "got: {json}"
    );
    assert_eq!(json["summary"]["checked"], 1);
    assert_eq!(json["summary"]["skipped"], 1);

    std::fs::remove_dir_all(&dir).ok();
}

/// Fit `name`.epub in place so it keeps its innocent name but carries the
/// provenance stamp - the situation the filename heuristic cannot see.
fn stamp_in_place(book: &std::path::Path) {
    let output = bin()
        .args([
            "fit",
            book.to_str().unwrap(),
            "--profile",
            "x4",
            "--lets-get-dangerous",
        ])
        .output()
        .expect("failed to run binary");
    assert!(
        output.status.success(),
        "in-place fit should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn copy_mode_scan_skips_stamped_books_with_innocent_names() {
    let dir = temp_dir("stamped-copy-scan");
    let lib = dir.join("lib");
    std::fs::create_dir_all(&lib).expect("create lib");
    let book = book_in(&lib, "book");
    stamp_in_place(&book);

    let scan = bin()
        .args(["fit", lib.to_str().unwrap(), "--profile", "x4"])
        .output()
        .expect("failed to run binary");
    assert_eq!(scan.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&scan.stdout);
    assert!(
        stdout.contains("skipped (already tailored, use --force)"),
        "the stamp beats the innocent name, got:\n{stdout}"
    );
    assert!(
        !lib.join("book.x4.epub").exists(),
        "a stamped book must not be re-fitted"
    );

    let forced = bin()
        .args(["fit", lib.to_str().unwrap(), "--profile", "x4", "--force"])
        .output()
        .expect("failed to run binary");
    assert_eq!(forced.status.code(), Some(0));
    assert!(
        lib.join("book.x4.epub").exists(),
        "--force overrides the stamp skip"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn check_batch_skips_stamped_books() {
    let dir = temp_dir("stamped-check-scan");
    let lib = dir.join("lib");
    std::fs::create_dir_all(&lib).expect("create lib");
    let stamped = book_in(&lib, "stamped");
    stamp_in_place(&stamped);
    book_in(&lib, "fresh");

    let output = bin()
        .args(["check", lib.to_str().unwrap()])
        .output()
        .expect("failed to run binary");
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("skipped (already tailored, use --force)"),
        "got:\n{stdout}"
    );

    let json_run = bin()
        .args(["check", lib.to_str().unwrap(), "--report", "json"])
        .output()
        .expect("failed to run binary");
    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&json_run.stdout)).expect("one document");
    let results = json["results"].as_array().expect("results");
    assert!(
        results
            .iter()
            .any(|r| r["status"] == "skipped" && r["reason"] == "already-tailored"),
        "got: {json}"
    );
    assert_eq!(json["summary"]["checked"], 1);
    assert_eq!(json["summary"]["skipped"], 1);

    let forced = bin()
        .args([
            "check",
            lib.to_str().unwrap(),
            "--force",
            "--report",
            "json",
        ])
        .output()
        .expect("failed to run binary");
    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&forced.stdout)).expect("one document");
    assert_eq!(json["summary"]["checked"], 2, "got: {json}");
    assert_eq!(json["summary"]["skipped"], 0, "got: {json}");

    std::fs::remove_dir_all(&dir).ok();
}

fn in_place_fit(lib: &std::path::Path, extra: &[&str]) -> std::process::Output {
    let mut args = vec!["fit", lib.to_str().unwrap(), "--profile", "x4"];
    args.push("--lets-get-dangerous");
    args.extend_from_slice(extra);
    bin().args(&args).output().expect("failed to run binary")
}

#[test]
fn fit_batch_in_place_replaces_every_book_in_a_folder() {
    let dir = temp_dir("in-place-folder");
    let lib = dir.join("lib");
    std::fs::create_dir_all(&lib).expect("create lib");
    let a = book_in(&lib, "a");
    let b = book_in(&lib, "b");
    let a_before = std::fs::read(&a).expect("read a");
    let b_before = std::fs::read(&b).expect("read b");

    let output = in_place_fit(&lib, &[]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let a_after = std::fs::read(&a).expect("read a");
    let b_after = std::fs::read(&b).expect("read b");
    assert_ne!(a_after, a_before, "a.epub must be rewritten in place");
    assert_ne!(b_after, b_before, "b.epub must be rewritten in place");
    assert!(a_after.starts_with(b"PK"), "still a zip archive");
    assert!(
        !lib.join("a.x4.epub").exists() && !lib.join("b.x4.epub").exists(),
        "no copies may be written"
    );
    assert!(
        !std::fs::read_dir(&lib).expect("list lib").any(|e| e
            .expect("entry")
            .file_name()
            .to_string_lossy()
            .contains(".tmp-")),
        "no staging temp file may linger"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("replaced in place"),
        "in-place lines say so, got:\n{stdout}"
    );
    assert!(
        stdout.contains("2 written, 0 skipped, 0 failed"),
        "got:\n{stdout}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn fit_batch_in_place_rerun_skips_stamped_books() {
    let dir = temp_dir("in-place-rerun");
    let lib = dir.join("lib");
    std::fs::create_dir_all(&lib).expect("create lib");
    let a = book_in(&lib, "a");
    book_in(&lib, "b");

    assert_eq!(in_place_fit(&lib, &[]).status.code(), Some(0));
    let bytes_after_first = std::fs::read(&a).expect("read a");

    let rerun = in_place_fit(&lib, &[]);
    assert_eq!(rerun.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&rerun.stdout);
    assert!(
        stdout.contains("skipped (already tailored, use --force)"),
        "the stamp makes in-place reruns idempotent, got:\n{stdout}"
    );
    assert!(
        stdout.contains("0 written, 2 skipped, 0 failed"),
        "got:\n{stdout}"
    );
    assert_eq!(
        std::fs::read(&a).expect("read a"),
        bytes_after_first,
        "a skipped book must not be rewritten"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn fit_batch_in_place_force_refits() {
    let dir = temp_dir("in-place-force");
    let lib = dir.join("lib");
    std::fs::create_dir_all(&lib).expect("create lib");
    book_in(&lib, "a");

    assert_eq!(in_place_fit(&lib, &[]).status.code(), Some(0));
    let forced = in_place_fit(&lib, &["--force"]);
    assert_eq!(forced.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&forced.stdout);
    assert!(
        stdout.contains("1 written, 0 skipped, 0 failed"),
        "force refits the stamped book, got:\n{stdout}"
    );
    assert!(
        !lib.join("a.x4.epub").exists(),
        "in place means in place, even under --force"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn fit_batch_in_place_dry_run_writes_nothing() {
    let dir = temp_dir("in-place-dry-run");
    let lib = dir.join("lib");
    std::fs::create_dir_all(&lib).expect("create lib");
    book_in(&lib, "a");
    book_in(&lib, "b");
    let before = tree_listing(&dir);
    let bytes_before = std::fs::read(lib.join("a.epub")).expect("read a");

    let dry = in_place_fit(&lib, &["--dry-run"]);
    assert_eq!(dry.status.code(), Some(0));
    assert_eq!(tree_listing(&dir), before, "a dry run must not touch files");
    assert_eq!(
        std::fs::read(lib.join("a.epub")).expect("read a"),
        bytes_before,
        "not even in place"
    );
    let stdout = String::from_utf8_lossy(&dry.stdout);
    assert!(
        stdout.contains("would replace in place") && stdout.contains("(dry run)"),
        "got:\n{stdout}"
    );

    // The dry run stamped nothing, so a real run still converts both.
    let real = in_place_fit(&lib, &[]);
    assert!(
        String::from_utf8_lossy(&real.stdout).contains("2 written, 0 skipped, 0 failed"),
        "the dry run must not have stamped anything"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn fit_batch_in_place_json() {
    let dir = temp_dir("in-place-json");
    let lib = dir.join("lib");
    std::fs::create_dir_all(&lib).expect("create lib");
    let a = book_in(&lib, "a");

    let output = in_place_fit(&lib, &["--report", "json"]);
    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).expect("one document");
    assert_eq!(json["schema"], 1);
    assert_eq!(json["in_place"], true);
    let results = json["results"].as_array().expect("results");
    let converted = results
        .iter()
        .find(|r| r["status"] == "converted")
        .expect("a converted entry");
    assert_eq!(
        converted["output"],
        a.to_str().unwrap(),
        "in place, the output is the input"
    );

    let rerun = in_place_fit(&lib, &["--report", "json"]);
    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&rerun.stdout)).expect("one document");
    assert!(
        json["results"]
            .as_array()
            .expect("results")
            .iter()
            .any(|r| r["reason"] == "already-tailored"),
        "got: {json}"
    );
    assert_eq!(json["summary"]["skipped"], 1);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn named_files_bypass_the_stamp_skip() {
    let dir = temp_dir("in-place-named");
    let a = book_in(&dir, "a");
    let b = book_in(&dir, "b");

    for _ in 0..2 {
        let output = bin()
            .args([
                "fit",
                a.to_str().unwrap(),
                b.to_str().unwrap(),
                "--profile",
                "x4",
                "--lets-get-dangerous",
            ])
            .output()
            .expect("failed to run binary");
        assert_eq!(output.status.code(), Some(0));
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("2 written, 0 skipped, 0 failed"),
            "named files are always processed, stamped or not"
        );
    }

    std::fs::remove_dir_all(&dir).ok();
}
