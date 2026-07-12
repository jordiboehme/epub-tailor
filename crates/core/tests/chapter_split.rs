//! End-to-end gate for M8's oversize-chapter split: convert a book whose
//! first chapter is padded past 600KB across many headed sections, check the
//! bookkeeping (spine order, TOC fragment retargeting, cross-chapter and
//! same-document href retargeting), and validate the result with epubcheck
//! (skip-if-unavailable, the same harness the other integration tests use).

mod common;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use common::epub3_oversize_chapter;
use epub_tailor_core::{ConvertOptions, Input, convert, read_epub};

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

/// Which ids live in each spine resource, per the re-read book.
fn ids_by_path(book: &epub_tailor_core::Book) -> HashMap<String, Vec<String>> {
    let mut out = HashMap::new();
    for path in &book.spine {
        let Some(resource) = book.resources.get(path) else {
            continue;
        };
        let text = String::from_utf8_lossy(&resource.data);
        let mut ids = Vec::new();
        // A simple id="..." scan is enough here (real parsing is already
        // covered by the html/dom test suite); this test cares about
        // bookkeeping, not parsing.
        let mut rest = text.as_ref();
        while let Some(idx) = rest.find("id=\"") {
            rest = &rest[idx + 4..];
            let Some(end) = rest.find('"') else { break };
            ids.push(rest[..end].to_string());
            rest = &rest[end..];
        }
        out.insert(path.clone(), ids);
    }
    out
}

fn owning_path(ids_by_path: &HashMap<String, Vec<String>>, id: &str) -> String {
    ids_by_path
        .iter()
        .find(|(_, ids)| ids.iter().any(|i| i == id))
        .map(|(path, _)| path.clone())
        .unwrap_or_else(|| panic!("no spine resource holds id {id}"))
}

#[test]
fn oversize_chapter_splits_with_consistent_bookkeeping() {
    let converted = convert(
        Input::Epub(epub3_oversize_chapter()),
        &ConvertOptions::default(),
    )
    .expect("conversion should succeed");

    assert_eq!(
        converted.report.stats.chapters_split, 1,
        "exactly the oversize chapter should have split"
    );
    assert!(
        converted
            .report
            .transformations
            .iter()
            .any(|t| t.kind == "chapter-split"),
        "expected a chapter-split transformation: {:?}",
        converted.report.transformations
    );

    // Re-read our own output to inspect the bookkeeping directly.
    let read = read_epub(&converted.epub).expect("re-reading our own output should succeed");
    let book = read.book;

    let chapter1_parts: Vec<&String> = book
        .spine
        .iter()
        .filter(|p| p.contains("chapter1"))
        .collect();
    assert!(
        chapter1_parts.len() > 1,
        "chapter 1 should have split into multiple spine parts: {:?}",
        book.spine
    );
    // Part order in the spine matches numbering, and chapter 2 follows all of
    // chapter 1's parts.
    for (i, part) in chapter1_parts.iter().enumerate() {
        assert_eq!(
            part.as_str(),
            format!("OEBPS/text/chapter1-{}.xhtml", i + 1)
        );
    }
    assert_eq!(
        book.spine.last().map(String::as_str),
        Some("OEBPS/text/chapter2.xhtml")
    );

    // TOC: the no-fragment "Chapter 1" entry always lands on part 1; each
    // fragment entry lands on whichever part actually holds that section.
    let ids = ids_by_path(&book);
    let toc_chapter1 = book
        .toc
        .iter()
        .find(|e| e.title == "Chapter 1")
        .expect("TOC has a Chapter 1 entry");
    assert_eq!(toc_chapter1.href, "OEBPS/text/chapter1-1.xhtml");

    for section in ["sec1", "sec30", "sec60"] {
        let toc_entry = book
            .toc
            .iter()
            .find(|e| e.href.ends_with(&format!("#{section}")))
            .unwrap_or_else(|| panic!("TOC has a fragment entry for {section}"));
        let (toc_path, _) = toc_entry.href.split_once('#').unwrap();
        assert_eq!(
            toc_path,
            owning_path(&ids, section),
            "TOC entry for {section} must point at its real owning part"
        );
    }

    // Cross-chapter href: chapter 2's link to chapter 1's middle section, and
    // its no-fragment link, must retarget to sec30's/part 1's real paths.
    let chapter2_text =
        String::from_utf8_lossy(&book.resources["OEBPS/text/chapter2.xhtml"].data).into_owned();
    assert!(
        !chapter2_text.contains("chapter1.xhtml#sec30"),
        "must not still reference the vanished single chapter1.xhtml: {chapter2_text}"
    );
    let sec30_path = owning_path(&ids, "sec30");
    let sec30_basename = sec30_path.rsplit('/').next().unwrap();
    assert!(
        chapter2_text.contains(&format!("href=\"{sec30_basename}#sec30\"")),
        "chapter 2 must reference sec30's real part ({sec30_path}): {chapter2_text}"
    );
    assert!(
        chapter2_text.contains("href=\"chapter1-1.xhtml\""),
        "chapter 2's no-fragment link must land on part 1: {chapter2_text}"
    );

    // Same-document href across parts: section 1 links forward to section 60;
    // since a 600KB/60-section chapter cannot fit a single ~200KB part, they
    // must land in different parts, and the forward link must follow.
    let sec1_path = owning_path(&ids, "sec1");
    let sec60_path = owning_path(&ids, "sec60");
    assert_ne!(
        sec1_path, sec60_path,
        "the fixture must actually force sec1 and sec60 apart for this to test anything"
    );
    let sec1_text = String::from_utf8_lossy(&book.resources[&sec1_path].data).into_owned();
    let sec60_basename = sec60_path.rsplit('/').next().unwrap();
    assert!(
        sec1_text.contains(&format!("href=\"{sec60_basename}#sec60\"")),
        "sec1's part must reference sec60's real part ({sec60_path}): {sec1_text}"
    );
    let sec60_text = String::from_utf8_lossy(&book.resources[&sec60_path].data).into_owned();
    let sec1_basename = sec1_path.rsplit('/').next().unwrap();
    assert!(
        sec60_text.contains(&format!("href=\"{sec1_basename}#sec1\"")),
        "sec60's part must reference sec1's real part ({sec1_path}): {sec60_text}"
    );

    // Every part is comfortably under the device's chapter byte cap.
    for part in &chapter1_parts {
        let size = book.resources[part.as_str()].data.len();
        assert!(
            size <= ConvertOptions::default().max_chapter_bytes,
            "{part} is {size} bytes, over the chapter cap"
        );
    }

    // epubcheck gate: 0 errors, 0 warnings.
    let out_path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("chapter_split.tailored.epub");
    std::fs::write(&out_path, &converted.epub).expect("write converted epub to temp dir");
    match run_epubcheck(&out_path) {
        None => {
            eprintln!(
                "SKIP: epubcheck not found (not on PATH, EPUBCHECK_JAR unset); skipping the \
                 chapter-split epubcheck validation"
            );
        }
        Some(output) => {
            let combined = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
            let offenders: Vec<&str> = combined
                .lines()
                .filter(|line| {
                    line.contains("FATAL(") || line.contains("ERROR(") || line.contains("WARNING(")
                })
                .collect();
            assert!(
                offenders.is_empty(),
                "epubcheck reported {} problem(s) (status {:?}):\n{}",
                offenders.len(),
                output.status.code(),
                combined,
            );
        }
    }
}
