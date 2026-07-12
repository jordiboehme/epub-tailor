//! `epub-tailor`: a CLI that cleans, fixes and transforms EPUB (and Markdown)
//! books, cut to measure for a target device by composable JSON profiles.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand, ValueEnum};
use epub_tailor_core::profile::{self, Profile};
use epub_tailor_core::{
    ConvertOptions, Converted, FsResolver, Input, LintFinding, Severity, TableMode, convert,
    lint_epub,
};

/// Exit code used when the input cannot be read at all (`check` only).
const UNREADABLE_EXIT_CODE: u8 = 2;
/// Exit code used when a conversion fails, or `check` finds an `Error`-severity finding.
const ERROR_EXIT_CODE: u8 = 1;

/// Clean, fix and tailor EPUB books to fit your e-reader.
#[derive(Parser)]
#[command(name = "epub-tailor", version, about, long_about = None)]
struct Cli {
    /// Increase logging verbosity (-v, -vv).
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Clean, fix and transform an EPUB according to the selected profiles.
    #[command(aliases = ["optimize", "clean"])]
    Fit {
        /// Path to the source EPUB.
        input: PathBuf,
        /// Replace the original file in place instead of writing a copy.
        /// Lets. Get. Dangerous.
        #[arg(long, conflicts_with = "output")]
        lets_get_dangerous: bool,
        #[command(flatten)]
        common: CommonArgs,
    },
    /// Convert a Markdown file into an EPUB.
    Md {
        /// Path to the source Markdown file.
        input: PathBuf,
        #[command(flatten)]
        common: CommonArgs,
        /// Heading level to split chapters on (Markdown input only).
        #[arg(long, value_parser = clap::value_parser!(u8).range(1..=2), default_value_t = 1)]
        split_level: u8,
    },
    /// Validate an EPUB against the selected profiles without converting it.
    Check {
        /// Path to the EPUB to check.
        input: PathBuf,
        /// Profiles to check against, composed left to right (a built-in name
        /// or a path to a .json file). Defaults to `epub`, structural checks
        /// only.
        #[arg(long = "profile", value_name = "NAME|PATH")]
        profiles: Vec<String>,
        /// Report format.
        #[arg(long, value_enum, default_value_t = ReportArg::Human)]
        report: ReportArg,
    },
    /// List the built-in profiles, or print a resolved profile composition.
    Profiles {
        /// Profile specs to resolve and print as JSON; with none given, the
        /// built-ins are listed.
        specs: Vec<String>,
    },
}

#[derive(Args)]
struct CommonArgs {
    /// Where to write the converted EPUB.
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Profiles to apply, composed left to right (a built-in name or a path
    /// to a .json file). Defaults to `epub`, the repair-only profile.
    #[arg(long = "profile", value_name = "NAME|PATH")]
    profiles: Vec<String>,

    /// JPEG quality: `low` (70), `std` (82), `high` (90) or a raw number
    /// 1-100. Overrides the profile value.
    #[arg(long, value_parser = parse_quality)]
    quality: Option<u8>,

    /// How to represent tables when the profile linearizes them: text
    /// flattens them to paragraphs, image rasterizes complex tables and
    /// linearizes simple ones, image-all rasterizes every table it safely
    /// can. Overrides the profile value.
    #[arg(long, value_enum)]
    tables: Option<TablesArg>,

    /// Split images taller than the screen into multiple images. Overrides
    /// the profile value.
    #[arg(long)]
    split_tall_images: bool,

    /// Maximum chapter size in KiB before splitting at a heading boundary.
    /// Overrides the profile value.
    #[arg(long)]
    max_chapter_kb: Option<u32>,

    /// Analyze and report what would change, without writing any output.
    #[arg(long)]
    dry_run: bool,

    /// Report format.
    #[arg(long, value_enum, default_value_t = ReportArg::Human)]
    report: ReportArg,
}

#[derive(Clone, Copy, ValueEnum)]
enum TablesArg {
    Text,
    Image,
    ImageAll,
}

#[derive(Clone, Copy, ValueEnum)]
enum ReportArg {
    Human,
    Json,
}

impl CommonArgs {
    /// Resolve the `--profile` layers into one composed [`Profile`].
    fn resolve_profile(&self) -> Result<Profile, profile::ProfileError> {
        profile::resolve(&self.profiles)
    }

    /// Translate the resolved profile plus any explicit CLI overrides into a
    /// [`ConvertOptions`]. Flags the user did not pass leave the profile
    /// values untouched.
    fn to_options(&self, resolved: &Profile) -> ConvertOptions {
        let mut opts = resolved.to_options();
        if let Some(quality) = self.quality {
            opts.jpeg_quality = quality;
        }
        if let Some(tables) = self.tables {
            opts.tables = match tables {
                TablesArg::Text => TableMode::Text,
                TablesArg::Image => TableMode::Image,
                TablesArg::ImageAll => TableMode::ImageAll,
            };
        }
        if self.split_tall_images {
            opts.split_tall_images = true;
        }
        if let Some(kb) = self.max_chapter_kb {
            opts.max_chapter_bytes = kb as usize * 1024;
        }
        opts.dry_run = self.dry_run;
        opts
    }
}

/// Parse a `--quality` value: `low` (70), `std` (82), `high` (90) or a raw number 1-100.
fn parse_quality(s: &str) -> Result<u8, String> {
    match s {
        "low" => Ok(70),
        "std" => Ok(82),
        "high" => Ok(90),
        _ => {
            let n: u8 = s.parse().map_err(|_| {
                format!("invalid quality `{s}` (expected `low`, `std`, `high` or a number 1-100)")
            })?;
            if (1..=100).contains(&n) {
                Ok(n)
            } else {
                Err(format!("quality must be between 1 and 100, got {n}"))
            }
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    match cli.command {
        Command::Profiles { specs } => run_profiles(&specs),
        Command::Fit {
            input,
            lets_get_dangerous,
            common,
        } => run_fit(&input, lets_get_dangerous, &common),
        Command::Md {
            input,
            common,
            split_level,
        } => run_md(&input, split_level, &common),
        Command::Check {
            input,
            profiles,
            report,
        } => run_check(&input, &profiles, report),
    }
}

/// Run the `fit` subcommand: read the input EPUB, convert it according to the
/// resolved profiles, write the output (unless `--dry-run`), and print a
/// human or JSON report. With `--lets-get-dangerous` the original file is
/// replaced in place (written via a sibling temp file and renamed, so a
/// failed write never leaves a half-book behind).
fn run_fit(input: &Path, in_place: bool, common: &CommonArgs) -> ExitCode {
    let resolved = match common.resolve_profile() {
        Ok(resolved) => resolved,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(ERROR_EXIT_CODE);
        }
    };

    let bytes = match std::fs::read(input) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("error: cannot read {}: {e}", input.display());
            return ExitCode::from(ERROR_EXIT_CODE);
        }
    };

    let opts = common.to_options(&resolved);
    let converted = match convert(Input::Epub(bytes), &opts) {
        Ok(converted) => converted,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(ERROR_EXIT_CODE);
        }
    };

    let output_path = if in_place {
        input.to_path_buf()
    } else {
        common.output.clone().unwrap_or_else(|| {
            default_output_path(input, &format!("{}.epub", resolved.appendix_or_default()))
        })
    };
    finish_conversion(
        converted,
        &output_path,
        opts.dry_run,
        in_place,
        common.report,
    )
}

/// Run the `md` subcommand: read the Markdown source, resolve its local
/// images relative to its own directory, convert it according to the resolved
/// profiles, write the output (unless `--dry-run`), and print a report.
fn run_md(input: &Path, split_level: u8, common: &CommonArgs) -> ExitCode {
    let resolved = match common.resolve_profile() {
        Ok(resolved) => resolved,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(ERROR_EXIT_CODE);
        }
    };

    let text = match std::fs::read_to_string(input) {
        Ok(text) => text,
        Err(e) => {
            eprintln!("error: cannot read {}: {e}", input.display());
            return ExitCode::from(ERROR_EXIT_CODE);
        }
    };

    let root = match input.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let assets = Box::new(FsResolver::new(root));

    let mut opts = common.to_options(&resolved);
    opts.split_level = split_level;

    let converted = match convert(Input::Markdown { text, assets }, &opts) {
        Ok(converted) => converted,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(ERROR_EXIT_CODE);
        }
    };

    let output_path = common
        .output
        .clone()
        .unwrap_or_else(|| default_output_path(input, "epub"));
    finish_conversion(converted, &output_path, opts.dry_run, false, common.report)
}

/// Write the converted EPUB (unless `--dry-run`) and print the report, shared
/// by every conversion subcommand. An in-place write goes through a sibling
/// temp file plus rename so the original survives any write failure.
fn finish_conversion(
    converted: Converted,
    output_path: &Path,
    dry_run: bool,
    in_place: bool,
    report_format: ReportArg,
) -> ExitCode {
    if !dry_run && let Err(e) = write_output(output_path, &converted.epub, in_place) {
        eprintln!("error: cannot write {}: {e}", output_path.display());
        return ExitCode::from(ERROR_EXIT_CODE);
    }

    match report_format {
        ReportArg::Human => print_human_report(&converted, output_path, dry_run),
        ReportArg::Json => match serde_json::to_string_pretty(&converted.report) {
            Ok(json) => println!("{json}"),
            Err(e) => {
                eprintln!("error: could not serialize report: {e}");
                return ExitCode::from(ERROR_EXIT_CODE);
            }
        },
    }

    ExitCode::SUCCESS
}

/// Default output path: `<input stem>.<extension>`, next to the input file.
fn default_output_path(input: &Path, extension: &str) -> PathBuf {
    input.with_extension(extension)
}

/// Write `data` to `path`. An in-place replacement is staged in a sibling
/// temp file and renamed over the original, so a failed or interrupted write
/// never truncates the book being replaced.
fn write_output(path: &Path, data: &[u8], in_place: bool) -> std::io::Result<()> {
    if !in_place {
        return std::fs::write(path, data);
    }
    let mut temp = path.as_os_str().to_owned();
    temp.push(format!(".tmp-{}", std::process::id()));
    let temp = PathBuf::from(temp);
    std::fs::write(&temp, data)?;
    if let Err(e) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(e);
    }
    Ok(())
}

/// Human report for a conversion, in three sections: "Transformed" (counts
/// per transformation kind), "Warnings" (one line each) and "Stats"
/// (aligned counters).
fn print_human_report(converted: &Converted, output_path: &Path, dry_run: bool) {
    let report = &converted.report;
    if dry_run {
        println!("dry run: no output written");
    } else {
        println!("wrote {}", output_path.display());
    }
    println!();

    println!("Transformed:");
    if report.transformations.is_empty() {
        println!("  nothing");
    } else {
        let mut counts: Vec<(&str, usize)> = Vec::new();
        for t in &report.transformations {
            match counts.iter_mut().find(|(kind, _)| *kind == t.kind) {
                Some((_, n)) => *n += 1,
                None => counts.push((&t.kind, 1)),
            }
        }
        let width = counts.iter().map(|(kind, _)| kind.len()).max().unwrap_or(0);
        for (kind, n) in &counts {
            println!("  {kind:<width$}  {n:>4}");
        }
    }
    println!();

    println!("Warnings:");
    if report.warnings.is_empty() {
        println!("  none");
    } else {
        for warning in &report.warnings {
            match &warning.file {
                Some(file) => println!("  - [{file}] {}", warning.message),
                None => println!("  - {}", warning.message),
            }
        }
    }
    println!();

    println!("Stats:");
    let stats = &report.stats;
    let rows = [
        ("bytes in", stats.bytes_in.to_string()),
        ("bytes out", stats.bytes_out.to_string()),
        ("images processed", stats.images_processed.to_string()),
        ("chapters", stats.chapters.to_string()),
        ("chapters split", stats.chapters_split.to_string()),
        ("warnings", stats.warnings.to_string()),
    ];
    let label_width = rows.iter().map(|(l, _)| l.len()).max().unwrap_or(0);
    let value_width = rows.iter().map(|(_, v)| v.len()).max().unwrap_or(0);
    for (label, value) in &rows {
        println!("  {label:<label_width$}  {value:>value_width$}");
    }
}

/// Run the `check` subcommand: lint an EPUB against the resolved profiles
/// without converting it. Structural checks always run; device checks run
/// only for features the profile enables. Exits 0 with no `Error`-severity
/// findings, 1 otherwise, 2 if the input cannot even be read from disk.
fn run_check(input: &Path, profiles: &[String], report_format: ReportArg) -> ExitCode {
    let resolved = match profile::resolve(profiles) {
        Ok(resolved) => resolved,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(ERROR_EXIT_CODE);
        }
    };

    let bytes = match std::fs::read(input) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("error: cannot read {}: {e}", input.display());
            return ExitCode::from(UNREADABLE_EXIT_CODE);
        }
    };

    let findings = lint_epub(&bytes, &resolved.caps, &resolved.features);
    let errors = findings
        .iter()
        .filter(|f| f.severity == Severity::Error)
        .count();
    let warnings = findings
        .iter()
        .filter(|f| f.severity == Severity::Warning)
        .count();

    match report_format {
        ReportArg::Human => print_check_report(&findings, errors, warnings),
        ReportArg::Json => {
            let payload = serde_json::json!({
                "findings": findings,
                "errors": errors,
                "warnings": warnings,
            });
            match serde_json::to_string_pretty(&payload) {
                Ok(json) => println!("{json}"),
                Err(e) => {
                    eprintln!("error: could not serialize report: {e}");
                    return ExitCode::from(ERROR_EXIT_CODE);
                }
            }
        }
    }

    if errors > 0 {
        ExitCode::from(ERROR_EXIT_CODE)
    } else {
        ExitCode::SUCCESS
    }
}

/// Human `check` output: findings grouped by severity (errors first, then
/// warnings, then info), each with its code, then a one-line summary.
fn print_check_report(findings: &[LintFinding], errors: usize, warnings: usize) {
    for severity in [Severity::Error, Severity::Warning, Severity::Info] {
        let group: Vec<&LintFinding> = findings.iter().filter(|f| f.severity == severity).collect();
        if group.is_empty() {
            continue;
        }
        println!("{}:", severity_label(severity));
        for finding in group {
            match &finding.path {
                Some(path) => println!("  [{}] {path}: {}", finding.code, finding.message),
                None => println!("  [{}] {}", finding.code, finding.message),
            }
        }
    }
    println!("{errors} error(s), {warnings} warning(s)");
}

fn severity_label(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "Errors",
        Severity::Warning => "Warnings",
        Severity::Info => "Info",
    }
}

/// Run the `profiles` subcommand: with no specs, list the built-ins; with
/// specs, resolve the composition and print it as pretty JSON.
fn run_profiles(specs: &[String]) -> ExitCode {
    if specs.is_empty() {
        print_builtin_profiles();
        return ExitCode::SUCCESS;
    }
    match profile::resolve(specs) {
        Ok(resolved) => match serde_json::to_string_pretty(&resolved) {
            Ok(json) => {
                println!("{json}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("error: could not serialize profile: {e}");
                ExitCode::from(ERROR_EXIT_CODE)
            }
        },
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(ERROR_EXIT_CODE)
        }
    }
}

/// The built-in profile listing: one line per profile with its screen size
/// (or "-" for the device-neutral epub profile), output appendix and
/// description.
fn print_builtin_profiles() {
    let profiles = profile::builtins();
    let name_w = profiles
        .iter()
        .map(|p| p.name.len())
        .max()
        .unwrap_or(4)
        .max(4);
    let out_w = profiles
        .iter()
        .map(|p| p.appendix_or_default().len() + 6)
        .max()
        .unwrap_or(6)
        .max(6);
    println!(
        "{:<name_w$} {:<11} {:<out_w$} DESCRIPTION",
        "NAME", "SCREEN", "OUTPUT"
    );
    for profile in &profiles {
        let screen = if profile.caps.screen_w == u32::MAX {
            "-".to_string()
        } else {
            format!("{}x{}", profile.caps.screen_w, profile.caps.screen_h)
        };
        println!(
            "{:<name_w$} {:<11} {:<out_w$} {}",
            profile.name,
            screen,
            format!(".{}.epub", profile.appendix_or_default()),
            profile.description,
        );
    }
}
