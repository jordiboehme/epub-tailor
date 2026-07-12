//! Developer task runner for epub-tailor.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, ExitStatus};

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "xtask")]
struct Cli {
    #[command(subcommand)]
    command: Task,
}

#[derive(Subcommand)]
enum Task {
    /// Build the test fixture EPUBs used by the integration tests.
    BuildFixtures,
    /// Validate an EPUB with epubcheck (from `PATH`, else `java -jar
    /// $EPUBCHECK_JAR`), forwarding its exit code.
    Epubcheck {
        /// Path to the EPUB to validate.
        path: PathBuf,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Task::BuildFixtures => {
            println!("fixture builder: not implemented yet");
            ExitCode::SUCCESS
        }
        Task::Epubcheck { path } => run_epubcheck(&path),
    }
}

/// Run epubcheck against `path`, preferring `epubcheck` on `PATH` and falling
/// back to `java -jar $EPUBCHECK_JAR`. The child's exit code is forwarded.
fn run_epubcheck(path: &Path) -> ExitCode {
    match Command::new("epubcheck").arg(path).status() {
        Ok(status) => return exit_code_from(status),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            eprintln!("xtask: failed to run epubcheck: {e}");
            return ExitCode::FAILURE;
        }
    }

    if let Ok(jar) = std::env::var("EPUBCHECK_JAR") {
        match Command::new("java")
            .arg("-jar")
            .arg(&jar)
            .arg(path)
            .status()
        {
            Ok(status) => return exit_code_from(status),
            Err(e) => {
                eprintln!("xtask: failed to run `java -jar {jar}`: {e}");
                return ExitCode::FAILURE;
            }
        }
    }

    eprintln!(
        "xtask: epubcheck not found. Install it on PATH (e.g. `brew install epubcheck`) \
         or set EPUBCHECK_JAR to the path of epubcheck.jar."
    );
    ExitCode::FAILURE
}

fn exit_code_from(status: ExitStatus) -> ExitCode {
    match status.code() {
        Some(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
        None => ExitCode::FAILURE,
    }
}
