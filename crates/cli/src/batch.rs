//! The per-file seam every conversion subcommand goes through: read one
//! input, run the core, hand back a result. Nothing here prints or writes,
//! so a single-file run and a batch loop share exactly one code path.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use epub_tailor_core::profile::{self, Profile};
use epub_tailor_core::{
    ConvertOptions, ConvertReport, Converted, FsResolver, Input, LintFinding, Severity, convert,
    lint_epub,
};

use crate::discover::{self, DiscoveredFile};
use crate::{ERROR_EXIT_CODE, ReportArg, SCHEMA_VERSION, UNREADABLE_EXIT_CODE};

/// A failure tied to one file: a stable machine code plus prose.
pub struct FileFailure {
    pub code: String,
    pub message: String,
}

/// Which conversion a job runs. `md` reads text and resolves images next to
/// the source; `fit` reads EPUB bytes.
#[derive(Clone, Copy)]
pub enum JobKind {
    Fit,
    Md,
}

/// Convert one input file. Reads from disk, runs the core `convert`, never
/// prints and never writes.
pub fn convert_file(
    input: &Path,
    kind: JobKind,
    opts: &ConvertOptions,
) -> Result<Converted, FileFailure> {
    let result = match kind {
        JobKind::Fit => {
            let bytes = std::fs::read(input).map_err(|e| read_failure(input, &e))?;
            convert(Input::Epub(bytes), opts)
        }
        JobKind::Md => {
            let text = std::fs::read_to_string(input).map_err(|e| read_failure(input, &e))?;
            let root = match input.parent() {
                Some(dir) if !dir.as_os_str().is_empty() => dir.to_path_buf(),
                _ => PathBuf::from("."),
            };
            let assets = Box::new(FsResolver::new(root));
            convert(Input::Markdown { text, assets }, opts)
        }
    };
    result.map_err(|e| FileFailure {
        code: e.code().to_string(),
        message: e.to_string(),
    })
}

/// Lint one EPUB. The error is the read-error prose (`check`'s exit-2 class).
pub fn lint_file(input: &Path, resolved: &Profile) -> Result<Vec<LintFinding>, String> {
    let bytes =
        std::fs::read(input).map_err(|e| format!("cannot read {}: {e}", input.display()))?;
    Ok(lint_epub(&bytes, &resolved.caps, &resolved.features))
}

fn read_failure(input: &Path, e: &std::io::Error) -> FileFailure {
    FileFailure {
        code: "read-failed".to_string(),
        message: format!("cannot read {}: {e}", input.display()),
    }
}

/// Why a discovered file was left alone. Only scanned folders skip; a file
/// the user named on the command line is always processed.
pub enum SkipReason {
    /// The file name ends in a known output appendix, so it is itself the
    /// product of a previous run.
    PriorOutput,
    /// The output this input maps to already exists on disk.
    OutputExists(PathBuf),
}

/// What happened to one file of a batch run.
pub enum FileOutcome {
    Converted {
        input: PathBuf,
        output: PathBuf,
        report: ConvertReport,
    },
    Skipped {
        input: PathBuf,
        reason: SkipReason,
    },
    Failed {
        input: PathBuf,
        code: String,
        message: String,
    },
}

/// Everything a conversion batch needs to plan and run, resolved once up
/// front so no per-file work touches profiles, metadata or stdin.
pub struct ConvertBatch {
    pub kind: JobKind,
    /// Extension scanned for inside folder inputs, without the dot.
    pub input_extension: &'static str,
    /// Extension the output name swaps in, e.g. `x4.epub` or `epub`.
    pub output_extension: String,
    /// `.{appendix}.epub` suffixes marking a file as a prior run's output.
    pub prior_output_suffixes: Vec<String>,
    pub recursive: bool,
    pub force: bool,
    pub output_dir: Option<PathBuf>,
}

enum Planned {
    Job { input: PathBuf, output: PathBuf },
    Skip { input: PathBuf, reason: SkipReason },
}

/// Run a whole conversion batch: plan every file first (so fresh outputs are
/// never rescanned), convert one at a time, keep going past failures, then
/// summarize. Exits 1 if any file failed, 0 otherwise.
pub fn run_convert_batch(
    inputs: &[PathBuf],
    cfg: &ConvertBatch,
    opts: &ConvertOptions,
    report_format: ReportArg,
) -> ExitCode {
    let planned = match plan(inputs, cfg) {
        Ok(planned) => planned,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::from(ERROR_EXIT_CODE);
        }
    };

    let mut produced: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    let mut outcomes = Vec::with_capacity(planned.len());
    for item in planned {
        let outcome = match item {
            Planned::Skip { input, reason } => FileOutcome::Skipped { input, reason },
            Planned::Job { input, output } => {
                if produced.insert(output.clone()) {
                    run_job(input, output, cfg, opts)
                } else {
                    FileOutcome::Failed {
                        input,
                        code: "output-collision".to_string(),
                        message: format!(
                            "{} is already produced by an earlier input in this run",
                            output.display()
                        ),
                    }
                }
            }
        };
        if let FileOutcome::Failed { input, message, .. } = &outcome {
            eprintln!("error: {}: {message}", input.display());
        }
        if matches!(report_format, ReportArg::Human) {
            print_outcome_line(&outcome, opts.dry_run);
        }
        outcomes.push(outcome);
    }

    match report_format {
        ReportArg::Human => print_summary(&outcomes, opts.dry_run),
        ReportArg::Json => {
            // One aggregate document; per-file failures are results in it,
            // never separate error payloads, or stdout stops being one
            // parseable document.
            let payload = json_report(&outcomes, opts.dry_run);
            match serde_json::to_string_pretty(&payload) {
                Ok(json) => println!("{json}"),
                Err(e) => {
                    eprintln!("error: could not serialize report: {e}");
                    return ExitCode::from(ERROR_EXIT_CODE);
                }
            }
        }
    }

    let any_failed = outcomes
        .iter()
        .any(|o| matches!(o, FileOutcome::Failed { .. }));
    if any_failed {
        ExitCode::from(ERROR_EXIT_CODE)
    } else {
        ExitCode::SUCCESS
    }
}

fn json_report(outcomes: &[FileOutcome], dry_run: bool) -> serde_json::Value {
    let results: Vec<serde_json::Value> = outcomes
        .iter()
        .map(|outcome| match outcome {
            FileOutcome::Converted {
                input,
                output,
                report,
            } => serde_json::json!({
                "input": input.display().to_string(),
                "status": "converted",
                // Under --dry-run this is where the file would land.
                "output": output.display().to_string(),
                "transformations": report.transformations,
                "warnings": report.warnings,
                "stats": report.stats,
            }),
            FileOutcome::Skipped {
                input,
                reason: SkipReason::PriorOutput,
            } => serde_json::json!({
                "input": input.display().to_string(),
                "status": "skipped",
                "reason": "prior-output",
            }),
            FileOutcome::Skipped {
                input,
                reason: SkipReason::OutputExists(output),
            } => serde_json::json!({
                "input": input.display().to_string(),
                "status": "skipped",
                "reason": "output-exists",
                "output": output.display().to_string(),
            }),
            FileOutcome::Failed {
                input,
                code,
                message,
            } => serde_json::json!({
                "input": input.display().to_string(),
                "status": "failed",
                "error": { "code": code, "message": message },
            }),
        })
        .collect();

    let converted = outcomes
        .iter()
        .filter(|o| matches!(o, FileOutcome::Converted { .. }))
        .count();
    let skipped = outcomes
        .iter()
        .filter(|o| matches!(o, FileOutcome::Skipped { .. }))
        .count();
    serde_json::json!({
        "schema": SCHEMA_VERSION,
        "dry_run": dry_run,
        "results": results,
        "summary": {
            "converted": converted,
            "skipped": skipped,
            "failed": outcomes.len() - converted - skipped,
        },
    })
}

/// Expand the command-line inputs into per-file work, applying the skip
/// rules to scanned folders only.
fn plan(inputs: &[PathBuf], cfg: &ConvertBatch) -> Result<Vec<Planned>, String> {
    if let Some(dir) = &cfg.output_dir
        && dir.is_file()
    {
        return Err(format!(
            "{} is a file; -o must be a folder when processing folders or several files",
            dir.display()
        ));
    }
    let mut planned = Vec::new();
    for input in inputs {
        if input.is_dir() {
            let found = discover::discover(input, cfg.input_extension, cfg.recursive)
                .map_err(|e| format!("cannot scan {}: {e}", input.display()))?;
            if found.is_empty() {
                let hint = if cfg.recursive {
                    ""
                } else {
                    " (use -r to scan subfolders)"
                };
                eprintln!(
                    "no .{} files found in {}{hint}",
                    cfg.input_extension,
                    input.display()
                );
            }
            for file in found {
                planned.push(classify(file, cfg));
            }
        } else {
            let relative = PathBuf::from(input.file_name().unwrap_or(input.as_os_str()));
            let output = map_output(input, &relative, cfg);
            planned.push(Planned::Job {
                input: input.clone(),
                output,
            });
        }
    }
    Ok(planned)
}

fn classify(file: DiscoveredFile, cfg: &ConvertBatch) -> Planned {
    if !cfg.force && is_prior_output(&file.path, &cfg.prior_output_suffixes) {
        return Planned::Skip {
            input: file.path,
            reason: SkipReason::PriorOutput,
        };
    }
    let output = map_output(&file.path, &file.relative, cfg);
    if !cfg.force && output.exists() {
        return Planned::Skip {
            input: file.path,
            reason: SkipReason::OutputExists(output),
        };
    }
    Planned::Job {
        input: file.path,
        output,
    }
}

/// The batch twin of `default_output_path`: same extension swap, but under
/// `-o` the input's path relative to its scanned root is mirrored into the
/// output folder.
fn map_output(input: &Path, relative: &Path, cfg: &ConvertBatch) -> PathBuf {
    match &cfg.output_dir {
        None => input.with_extension(&cfg.output_extension),
        Some(dir) => dir.join(relative).with_extension(&cfg.output_extension),
    }
}

fn run_job(
    input: PathBuf,
    output: PathBuf,
    cfg: &ConvertBatch,
    opts: &ConvertOptions,
) -> FileOutcome {
    let converted = match convert_file(&input, cfg.kind, opts) {
        Ok(converted) => converted,
        Err(e) => {
            return FileOutcome::Failed {
                input,
                code: e.code,
                message: e.message,
            };
        }
    };
    if !opts.dry_run {
        if let Some(parent) = output.parent()
            && !parent.as_os_str().is_empty()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            return FileOutcome::Failed {
                input,
                code: "write-failed".to_string(),
                message: format!("cannot create {}: {e}", parent.display()),
            };
        }
        if let Err(e) = std::fs::write(&output, &converted.epub) {
            return FileOutcome::Failed {
                input,
                code: "write-failed".to_string(),
                message: format!("cannot write {}: {e}", output.display()),
            };
        }
    }
    FileOutcome::Converted {
        input,
        output,
        report: converted.report,
    }
}

/// The `.{appendix}.epub` suffixes that mark a file as a prior run's output:
/// every built-in profile's, the default and the resolved chain's own (which
/// covers custom JSON profiles). All lowercase; matching is case-insensitive.
pub fn output_suffixes(resolved: &Profile) -> Vec<String> {
    let mut suffixes: Vec<String> = profile::builtins()
        .iter()
        .map(|p| format!(".{}.epub", p.appendix_or_default()))
        .collect();
    suffixes.push(format!(".{}.epub", profile::DEFAULT_APPENDIX));
    suffixes.push(format!(".{}.epub", resolved.appendix_or_default()));
    for suffix in &mut suffixes {
        *suffix = suffix.to_ascii_lowercase();
    }
    suffixes.sort();
    suffixes.dedup();
    suffixes
}

fn is_prior_output(path: &Path, suffixes: &[String]) -> bool {
    let Some(name) = path.file_name() else {
        return false;
    };
    let name = name.to_string_lossy().to_ascii_lowercase();
    suffixes
        .iter()
        .any(|suffix| name.ends_with(suffix.as_str()))
}

fn print_outcome_line(outcome: &FileOutcome, dry_run: bool) {
    match outcome {
        FileOutcome::Converted { input, output, .. } => {
            if dry_run {
                println!("{} -> {} (dry run)", input.display(), output.display());
            } else {
                println!("{} -> {}", input.display(), output.display());
            }
        }
        FileOutcome::Skipped {
            input,
            reason: SkipReason::PriorOutput,
        } => {
            println!("{}: skipped (output of a previous run)", input.display());
        }
        FileOutcome::Skipped {
            input,
            reason: SkipReason::OutputExists(output),
        } => {
            println!(
                "{}: skipped ({} already exists, use --force)",
                input.display(),
                output.display()
            );
        }
        FileOutcome::Failed {
            input,
            code,
            message,
        } => {
            println!("{}: failed ({code}: {message})", input.display());
        }
    }
}

fn print_summary(outcomes: &[FileOutcome], dry_run: bool) {
    let written = outcomes
        .iter()
        .filter(|o| matches!(o, FileOutcome::Converted { .. }))
        .count();
    let skipped = outcomes
        .iter()
        .filter(|o| matches!(o, FileOutcome::Skipped { .. }))
        .count();
    let failed = outcomes.len() - written - skipped;
    let verb = if dry_run {
        "would be written"
    } else {
        "written"
    };
    println!();
    println!(
        "{} file(s): {written} {verb}, {skipped} skipped, {failed} failed",
        outcomes.len()
    );
}

/// What happened to one file of a batch `check` run.
pub enum CheckOutcome {
    Checked {
        input: PathBuf,
        findings: Vec<LintFinding>,
    },
    /// The file is a prior run's output; linting it against the profile it
    /// was cut for tells the user nothing new.
    Skipped {
        input: PathBuf,
    },
    Unreadable {
        input: PathBuf,
        message: String,
    },
}

/// What a batch `check` needs beyond the resolved profile.
pub struct CheckBatch {
    pub prior_output_suffixes: Vec<String>,
    pub recursive: bool,
    pub force: bool,
}

/// Run `check` over folders and several files, keep going past unreadable
/// inputs, then summarize. Exits 2 if any input was unreadable, else 1 on
/// any `Error`-severity finding, else 0 - the batch twins of the
/// single-file codes.
pub fn run_check_batch(
    inputs: &[PathBuf],
    cfg: &CheckBatch,
    resolved: &Profile,
    report_format: ReportArg,
) -> ExitCode {
    let mut outcomes = Vec::new();
    for input in inputs {
        if input.is_dir() {
            let found = match discover::discover(input, "epub", cfg.recursive) {
                Ok(found) => found,
                Err(e) => {
                    eprintln!("error: cannot scan {}: {e}", input.display());
                    return ExitCode::from(ERROR_EXIT_CODE);
                }
            };
            if found.is_empty() {
                let hint = if cfg.recursive {
                    ""
                } else {
                    " (use -r to scan subfolders)"
                };
                eprintln!("no .epub files found in {}{hint}", input.display());
            }
            for file in found {
                let outcome =
                    if !cfg.force && is_prior_output(&file.path, &cfg.prior_output_suffixes) {
                        CheckOutcome::Skipped { input: file.path }
                    } else {
                        check_one(file.path, resolved)
                    };
                push_check_outcome(outcome, report_format, &mut outcomes);
            }
        } else {
            let outcome = check_one(input.clone(), resolved);
            push_check_outcome(outcome, report_format, &mut outcomes);
        }
    }

    match report_format {
        ReportArg::Human => print_check_summary(&outcomes),
        ReportArg::Json => {
            let payload = check_json_report(&outcomes);
            match serde_json::to_string_pretty(&payload) {
                Ok(json) => println!("{json}"),
                Err(e) => {
                    eprintln!("error: could not serialize report: {e}");
                    return ExitCode::from(ERROR_EXIT_CODE);
                }
            }
        }
    }

    let unreadable = outcomes
        .iter()
        .any(|o| matches!(o, CheckOutcome::Unreadable { .. }));
    if unreadable {
        ExitCode::from(UNREADABLE_EXIT_CODE)
    } else if count_findings(&outcomes, Severity::Error) > 0 {
        ExitCode::from(ERROR_EXIT_CODE)
    } else {
        ExitCode::SUCCESS
    }
}

fn check_one(input: PathBuf, resolved: &Profile) -> CheckOutcome {
    match lint_file(&input, resolved) {
        Ok(findings) => CheckOutcome::Checked { input, findings },
        Err(message) => CheckOutcome::Unreadable { input, message },
    }
}

fn push_check_outcome(
    outcome: CheckOutcome,
    report_format: ReportArg,
    outcomes: &mut Vec<CheckOutcome>,
) {
    if let CheckOutcome::Unreadable { message, .. } = &outcome {
        eprintln!("error: {message}");
    }
    if matches!(report_format, ReportArg::Human) {
        print_check_outcome_line(&outcome);
    }
    outcomes.push(outcome);
}

fn severity_count(findings: &[LintFinding], severity: Severity) -> usize {
    findings.iter().filter(|f| f.severity == severity).count()
}

fn count_findings(outcomes: &[CheckOutcome], severity: Severity) -> usize {
    outcomes
        .iter()
        .map(|o| match o {
            CheckOutcome::Checked { findings, .. } => severity_count(findings, severity),
            _ => 0,
        })
        .sum()
}

fn print_check_outcome_line(outcome: &CheckOutcome) {
    match outcome {
        CheckOutcome::Checked { input, findings } => {
            println!(
                "{}: {} error(s), {} warning(s)",
                input.display(),
                severity_count(findings, Severity::Error),
                severity_count(findings, Severity::Warning)
            );
            // Error findings are shown under their file, so an exit code of
            // 1 is never a mystery; the full detail stays in --report json.
            for finding in findings.iter().filter(|f| f.severity == Severity::Error) {
                match &finding.path {
                    Some(path) => println!("  [{}] {path}: {}", finding.code, finding.message),
                    None => println!("  [{}] {}", finding.code, finding.message),
                }
            }
        }
        CheckOutcome::Skipped { input } => {
            println!("{}: skipped (output of a previous run)", input.display());
        }
        CheckOutcome::Unreadable { input, .. } => {
            println!("{}: unreadable", input.display());
        }
    }
}

fn print_check_summary(outcomes: &[CheckOutcome]) {
    let checked = outcomes
        .iter()
        .filter(|o| matches!(o, CheckOutcome::Checked { .. }))
        .count();
    let skipped = outcomes
        .iter()
        .filter(|o| matches!(o, CheckOutcome::Skipped { .. }))
        .count();
    let unreadable = outcomes.len() - checked - skipped;
    println!();
    println!(
        "{} file(s): {checked} checked, {skipped} skipped, {unreadable} unreadable; {} error(s), {} warning(s)",
        outcomes.len(),
        count_findings(outcomes, Severity::Error),
        count_findings(outcomes, Severity::Warning)
    );
}

fn check_json_report(outcomes: &[CheckOutcome]) -> serde_json::Value {
    let results: Vec<serde_json::Value> = outcomes
        .iter()
        .map(|outcome| match outcome {
            CheckOutcome::Checked { input, findings } => serde_json::json!({
                "input": input.display().to_string(),
                "status": "checked",
                "findings": findings,
                "errors": severity_count(findings, Severity::Error),
                "warnings": severity_count(findings, Severity::Warning),
            }),
            CheckOutcome::Skipped { input } => serde_json::json!({
                "input": input.display().to_string(),
                "status": "skipped",
                "reason": "prior-output",
            }),
            CheckOutcome::Unreadable { input, message } => serde_json::json!({
                "input": input.display().to_string(),
                "status": "unreadable",
                "message": message,
            }),
        })
        .collect();

    let checked = outcomes
        .iter()
        .filter(|o| matches!(o, CheckOutcome::Checked { .. }))
        .count();
    let skipped = outcomes
        .iter()
        .filter(|o| matches!(o, CheckOutcome::Skipped { .. }))
        .count();
    serde_json::json!({
        "schema": SCHEMA_VERSION,
        "results": results,
        "summary": {
            "checked": checked,
            "skipped": skipped,
            "unreadable": outcomes.len() - checked - skipped,
            "errors": count_findings(outcomes, Severity::Error),
            "warnings": count_findings(outcomes, Severity::Warning),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(output_dir: Option<PathBuf>) -> ConvertBatch {
        ConvertBatch {
            kind: JobKind::Fit,
            input_extension: "epub",
            output_extension: "x4.epub".to_string(),
            prior_output_suffixes: Vec::new(),
            recursive: false,
            force: false,
            output_dir,
        }
    }

    #[test]
    fn output_suffixes_cover_builtins_default_and_the_resolved_chain() {
        let resolved = profile::resolve(&["x4".to_string()]).expect("resolve x4");
        let suffixes = output_suffixes(&resolved);
        assert!(suffixes.contains(&".tailored.epub".to_string()));
        assert!(suffixes.contains(&".x4.epub".to_string()));
    }

    #[test]
    fn prior_output_matching_is_case_insensitive_and_needs_the_dot() {
        let suffixes = vec![".x4.epub".to_string()];
        assert!(is_prior_output(Path::new("Book.X4.EPUB"), &suffixes));
        assert!(!is_prior_output(Path::new("book.epub"), &suffixes));
        assert!(
            !is_prior_output(Path::new("x4.epub"), &suffixes),
            "a book named exactly x4.epub is not a prior output"
        );
    }

    #[test]
    fn map_output_swaps_the_extension_in_place_or_mirrors_into_the_output_dir() {
        let next_to_input = map_output(
            Path::new("/books/sub/my.book.epub"),
            Path::new("sub/my.book.epub"),
            &cfg(None),
        );
        assert_eq!(next_to_input, Path::new("/books/sub/my.book.x4.epub"));

        let mirrored = map_output(
            Path::new("/books/sub/a.epub"),
            Path::new("sub/a.epub"),
            &cfg(Some(PathBuf::from("/out"))),
        );
        assert_eq!(mirrored, Path::new("/out/sub/a.x4.epub"));
    }
}
