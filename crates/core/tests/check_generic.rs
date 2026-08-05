//! `check`'s per-copy lints, against the fixtures the destructive pass was
//! hardened on.
//!
//! The point of this file is not that the new findings fire - unit tests in
//! `validate.rs` cover that - but that they agree with what `--profile
//! generic` *actually does*. A lint that reports a file the fixer keeps is
//! worse than one that reports nothing: it tells the user to delete real
//! content. So the load-bearing assertion here is a subset property, and it
//! runs over the exact fixtures `reachable::prune` accumulated after a dead
//! `xlink:href` lookup once silently deleted real cover art.

mod common;

use std::collections::BTreeSet;

use epub_tailor_core::profile::resolve;
use epub_tailor_core::validate::{Category, Severity, lint_epub};
use epub_tailor_core::{ConvertOptions, Input, convert};

fn opts_for(specs: &[&str]) -> ConvertOptions {
    let specs: Vec<String> = specs.iter().map(|s| s.to_string()).collect();
    resolve(&specs).expect("profile resolves").to_options()
}

/// Lint `epub` under a profile stack, as the CLI's `check` does.
fn lint(epub: &[u8], specs: &[&str]) -> Vec<epub_tailor_core::validate::LintFinding> {
    let opts = opts_for(specs);
    lint_epub(epub, &opts.device, &opts.features, &opts.filters)
}

/// The paths `check` claims nothing references.
fn unreferenced_paths(epub: &[u8]) -> BTreeSet<String> {
    lint(epub, &["epub", "generic"])
        .into_iter()
        .filter(|f| f.code == "unreferenced")
        .filter_map(|f| f.path)
        .collect()
}

/// The paths a real `generic` conversion drops as unreferenced.
fn actually_dropped(epub: Vec<u8>) -> BTreeSet<String> {
    let out = convert(Input::Epub(epub), &opts_for(&["epub", "generic"])).expect("converts");
    out.report
        .transformations
        .into_iter()
        .filter(|t| t.kind == "generic-unreferenced")
        .filter_map(|t| t.file)
        .collect()
}

/// Every fixture `prune`'s reachability walk is pinned against, by name so a
/// failure says which edge type broke.
fn reachability_corpus() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        (
            "a stray file nothing references",
            common::book_with_extra_file("OEBPS/vendor-id.txt", b"SHTX001.635962014"),
        ),
        (
            "referenced only from CSS url()",
            common::book_with_css_referenced_asset(),
        ),
        (
            "an @import target past a malformed rule",
            common::book_with_css_import_after_a_malformed_rule(),
        ),
        (
            "an SVG cover raster reached only via xlink:href",
            common::book_with_svg_cover_wrapper(),
        ),
        (
            "an img srcset target with no src",
            common::book_with_srcset_only_image(),
        ),
        (
            "an srcset target owned by an orphan chapter",
            common::book_with_orphan_chapter_srcset_image(),
        ),
        ("a minimal EPUB 3", common::epub3_minimal()),
        ("a narrated EPUB 3", common::epub3_narrated()),
    ]
}

#[test]
fn check_never_reports_a_file_generic_would_keep() {
    // The subset property, and the reason this file exists. Under-reporting is
    // acceptable - a standalone walk may miss an edge the full pipeline
    // resolves - but over-reporting is a bug, because the app turns this into
    // "you have dead weight, here is a button to remove it".
    for (name, epub) in reachability_corpus() {
        let claimed = unreferenced_paths(&epub);
        let dropped = actually_dropped(epub);
        let over: Vec<&String> = claimed.difference(&dropped).collect();
        assert!(
            over.is_empty(),
            "check reported files generic keeps, in the fixture with {name}: {over:?} \
             (generic drops {dropped:?})"
        );
    }
}

#[test]
fn check_finds_a_manifested_file_nothing_references() {
    // The other direction: the subset property above is satisfied vacuously by
    // a check that reports nothing at all, so pin that real dead weight is
    // found, with its size.
    let epub = common::book_with_manifested_orphan();
    let finding = lint(&epub, &["epub", "generic"])
        .into_iter()
        .find(|f| f.code == "unreferenced")
        .expect("a manifested orphan must be reported");
    assert_eq!(finding.path.as_deref(), Some("OEBPS/orphan.png"));
    assert_eq!(finding.category, Category::Waste);
    assert_eq!(finding.severity, Severity::Info);
    assert!(
        finding.bytes.is_some_and(|b| b > 0),
        "the reported dead weight must carry its size: {finding:?}"
    );
}

#[test]
fn an_unmanifested_stray_file_stays_a_structural_error_not_dead_weight() {
    // The division of labour, and the reason `check_unreferenced` filters on
    // manifest membership. A file missing from the manifest is malformed, not
    // merely wasteful, and `check_manifest_sync` already says so at Error
    // severity - reporting it twice, once as a defect and once as a note about
    // wasted space, would double-count it in every rollup built on these.
    let epub = common::book_with_extra_file("OEBPS/vendor-id.txt", b"SHTX001.635962014");
    let findings = lint(&epub, &["epub", "generic"]);
    assert!(
        findings
            .iter()
            .any(|f| f.code == "manifest-sync" && f.severity == Severity::Error),
        "an unmanifested entry must still be a structural error: {findings:?}"
    );
    assert!(
        !unreferenced_paths(&epub).contains("OEBPS/vendor-id.txt"),
        "and must not ALSO be reported as dead weight"
    );
}

#[test]
fn the_repair_only_profile_reports_nothing_per_copy() {
    // The no-regression pin: `check` with no profile (or the app's historical
    // `epub`) must behave exactly as it did before the per-copy checks
    // existed. Every new code is gated on a `generic` feature, so none of them
    // may appear here.
    for (name, epub) in reachability_corpus() {
        let findings = lint(&epub, &["epub"]);
        let leaked: Vec<&str> = findings
            .iter()
            .filter(|f| f.category == Category::Watermark || f.category == Category::Waste)
            .map(|f| f.code)
            .collect();
        assert!(
            leaked.is_empty(),
            "the epub profile must stay purely structural, but {name} produced {leaked:?}"
        );
    }
}

#[test]
fn a_generic_conversions_own_output_is_no_longer_watermarked() {
    // Convergence, mirroring `nfc_convergence.rs`. This is what catches a
    // diagnostic that flags the writer's own synthesized
    // `urn:epub-tailor:<hash>` identifier: without the exemption in
    // `identity::classify` a clear majority of hashes read as per-copy, so
    // every cleaned book would go on claiming to be watermarked and the
    // app's chip would never clear no matter how often the user cleaned.
    for (name, epub) in reachability_corpus() {
        let out = convert(Input::Epub(epub), &opts_for(&["epub", "generic"])).expect("converts");
        let leftover: Vec<&str> = lint(&out.epub, &["epub", "generic"])
            .iter()
            .filter(|f| f.category == Category::Watermark)
            .map(|f| f.code)
            .collect();
        assert!(
            leftover.is_empty(),
            "a generic conversion of the fixture with {name} still reports {leftover:?}"
        );
    }
}

#[test]
fn an_img_srcset_with_no_src_leaves_dead_weight_the_converter_keeps_on_purpose() {
    // The one place `waste` does NOT converge, pinned rather than hidden.
    //
    // `image::rewrite_refs` strips `<img srcset>` from every conversion. Where
    // that was the image's only reference, the converter faces a choice
    // between deleting a raster it cannot prove is unused and keeping bytes
    // nothing points at; it deliberately keeps them (see the `rewrite_refs`
    // docs - the alternative once made images "go permanently missing with
    // zero warnings"). So the output genuinely does carry an unreferenced
    // image, and `check` saying so is correct, not a false positive.
    //
    // Worth knowing because it means a user can clean such a book twice and
    // still be told it has dead weight. That is honest; silently suppressing
    // it would not be.
    let epub = common::book_with_srcset_only_image();
    let out = convert(Input::Epub(epub), &opts_for(&["epub", "generic"])).expect("converts");
    assert!(
        !unreferenced_paths(&out.epub).is_empty(),
        "the srcset target the converter deliberately keeps is genuinely unreferenced \
         in the output, and check should not pretend otherwise"
    );
}

#[test]
fn a_per_copy_identifier_is_reported_but_never_as_an_error() {
    // A watermark is a fact about provenance, not a defect: reporting it as an
    // `Error` would make `check` exit non-zero on a legitimately purchased
    // book and break anyone gating CI on it.
    let epub = common::book_with_refined_identifier("arthur.dent@example.com", "URN");
    let findings = lint(&epub, &["epub", "generic"]);
    let watermarks: Vec<_> = findings
        .iter()
        .filter(|f| f.code == "watermark-identifier")
        .collect();
    assert!(
        !watermarks.is_empty(),
        "an email-shaped identifier must be reported: {findings:?}"
    );
    assert!(
        findings.iter().all(|f| f.severity != Severity::Error),
        "no per-copy finding may be an Error: {findings:?}"
    );
}

#[test]
fn the_reported_metadata_size_is_what_a_strip_would_actually_save() {
    // The byte figure comes from running the real strip and measuring, so it
    // must equal what the conversion reports saving - not an estimate from a
    // second size walk that could drift.
    let epub = common::book_with_image("OEBPS/photo.jpg", &common::jpeg_with_exif());
    let finding = lint(&epub, &["epub", "generic"])
        .into_iter()
        .find(|f| f.code == "media-metadata")
        .expect("EXIF must be reported");
    let bytes = finding
        .bytes
        .expect("a media-metadata finding carries bytes");
    assert!(
        bytes > 0,
        "a zero saving must not be reported at all: {finding:?}"
    );

    let out = convert(Input::Epub(epub), &opts_for(&["epub", "generic"])).expect("converts");
    let detail = out
        .report
        .transformations
        .iter()
        .find(|t| t.kind == "generic-media")
        .map(|t| t.detail.clone())
        .expect("generic strips the EXIF");
    assert!(
        detail.contains(&bytes.to_string()),
        "check reported {bytes} bytes but the conversion said {detail:?}"
    );
}
