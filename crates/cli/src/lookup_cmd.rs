//! The `metadata` command family.
//!
//! Four commands, one rule: **the ones that reach the network never write a
//! book, and the one that writes a book never reaches the network.**
//!
//! ```text
//!   metadata show    offline   reads a book, prints what it has and lacks
//!   metadata search  NETWORK   prints candidates
//!   metadata fetch   NETWORK   prints one complete record
//!   metadata pick    NETWORK   the above, interactively, then writes a document
//!
//!   fit --metadata   offline   applies a record to a book
//! ```
//!
//! That separation is what keeps `fit` reproducible, and it is what a GUI wants
//! anyway: search, show the user the candidates, let them choose, then convert.
//! `pick` is the only command that ever prompts, and it refuses to run without a
//! terminal - so a UI driving this binary can never be left hanging on a
//! question.

#[cfg(feature = "online")]
use std::io::{IsTerminal, Write};
use std::path::Path;
#[cfg(feature = "online")]
use std::path::PathBuf;
use std::process::ExitCode;

use epub_tailor_core::metadata::{MetadataDoc, missing_fields};
use epub_tailor_core::read_epub;

use crate::{ERROR_EXIT_CODE, MetadataCommand, ReportArg, SCHEMA_VERSION, UNREADABLE_EXIT_CODE};

pub fn run(command: MetadataCommand) -> ExitCode {
    match command {
        MetadataCommand::Show {
            input,
            cover_out,
            report,
        } => show(&input, cover_out.as_deref(), report),
        MetadataCommand::Search {
            input,
            title,
            author,
            isbn,
            limit,
            report,
        } => search(
            input.as_deref(),
            title.as_deref(),
            author.as_deref(),
            isbn.as_deref(),
            limit,
            report,
        ),
        MetadataCommand::Fetch {
            reference,
            cover_out,
            report,
        } => fetch(&reference, cover_out.as_deref(), report),
        MetadataCommand::Pick {
            input,
            output,
            limit,
        } => pick(&input, output.as_deref(), limit),
    }
}

/// A parsed book's metadata document, the list of fields it lacks, and its
/// own cover image bytes if it has one.
type BookRead = (MetadataDoc, Vec<&'static str>, Option<Vec<u8>>);

/// Read a book's metadata into a document, plus the list of what it lacks,
/// plus its own cover image bytes if it has one. The cover is cloned out of
/// the already-in-memory book regardless of whether the caller wants it -
/// cheap, since nothing here re-reads the file.
fn read_book(input: &Path) -> Result<BookRead, String> {
    let bytes =
        std::fs::read(input).map_err(|e| format!("cannot read {}: {e}", input.display()))?;
    let book = read_epub(&bytes).map_err(|e| e.to_string())?.book;
    let m = &book.metadata;

    let doc = MetadataDoc {
        title: (!m.title.is_empty()).then(|| m.title.clone()),
        authors: (!m.authors.is_empty())
            .then(|| epub_tailor_core::metadata::OneOrMany::Many(m.authors.clone())),
        language: (!m.language.is_empty()).then(|| m.language.clone()),
        identifier: m.identifier.clone(),
        identifiers: (!m.identifiers.is_empty()).then(|| m.identifiers.clone()),
        description: m.description.clone(),
        publisher: m.publisher.clone(),
        subjects: (!m.subjects.is_empty())
            .then(|| epub_tailor_core::metadata::OneOrMany::Many(m.subjects.clone())),
        date: m.date.clone(),
        rights: m.rights.clone(),
        series: m.series.as_ref().map(|s| s.name.clone()),
        series_index: m.series.as_ref().and_then(|s| s.index.clone()),
        ..MetadataDoc::default()
    };
    let cover = book
        .cover
        .as_ref()
        .and_then(|path| book.resources.get(path))
        .map(|r| r.data.clone());
    Ok((doc, missing_fields(m), cover))
}

fn show(input: &Path, cover_out: Option<&Path>, report: ReportArg) -> ExitCode {
    let (mut doc, missing, cover) = match read_book(input) {
        Ok(triple) => triple,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(UNREADABLE_EXIT_CODE);
        }
    };

    if let Some(path) = cover_out {
        match cover {
            None => eprintln!("warning: this book has no cover"),
            Some(data) => {
                if let Err(e) = std::fs::write(path, &data) {
                    eprintln!("error: cannot write {}: {e}", path.display());
                    return ExitCode::from(ERROR_EXIT_CODE);
                }
                doc.cover = Some(path.display().to_string());
            }
        }
    }

    match report {
        ReportArg::Json => {
            let payload = serde_json::json!({
                "schema": SCHEMA_VERSION,
                "metadata": doc,
                "missing": missing,
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&payload).expect("serializable")
            );
        }
        ReportArg::Human => {
            let json = serde_json::to_value(&doc).expect("serializable");
            let map = json.as_object().cloned().unwrap_or_default();
            if map.is_empty() {
                println!("This book carries no metadata at all.");
            } else {
                for (key, value) in &map {
                    println!("{key:<14} {}", render(value));
                }
            }
            if missing.is_empty() {
                println!("\nNothing missing.");
            } else {
                println!("\nMissing: {}", missing.join(", "));
                println!(
                    "Look it up with: epub-tailor metadata search {}",
                    input.display()
                );
            }
        }
    }
    ExitCode::SUCCESS
}

/// Render a JSON value for the human view, without the quotes and brackets.
fn render(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(items) => items.iter().map(render).collect::<Vec<_>>().join(", "),
        serde_json::Value::Object(map) => map
            .get("name")
            .map(render)
            .unwrap_or_else(|| value.to_string()),
        other => other.to_string(),
    }
}

#[cfg(not(feature = "online"))]
fn offline_build() -> ExitCode {
    eprintln!(
        "error: this build has no online lookup (built with --no-default-features). \
         Supply metadata by hand instead: epub-tailor fit book.epub --publisher '...' --description '...'"
    );
    ExitCode::from(ERROR_EXIT_CODE)
}

#[cfg(not(feature = "online"))]
fn search(
    _input: Option<&Path>,
    _title: Option<&str>,
    _author: Option<&str>,
    _isbn: Option<&str>,
    _limit: usize,
    _report: ReportArg,
) -> ExitCode {
    offline_build()
}

#[cfg(not(feature = "online"))]
fn fetch(_reference: &str, _cover_out: Option<&Path>, _report: ReportArg) -> ExitCode {
    offline_build()
}

#[cfg(not(feature = "online"))]
fn pick(_input: &Path, _output: Option<&Path>, _limit: usize) -> ExitCode {
    offline_build()
}

#[cfg(feature = "online")]
use crate::lookup::Lookup;
#[cfg(feature = "online")]
use epub_tailor_core::metadata::openlibrary::Candidate;

/// Take the title and author to search for from the book itself, unless the user
/// said otherwise. This is the whole ergonomic point of
/// `metadata search book.epub`.
#[cfg(feature = "online")]
fn seed_query(
    input: Option<&Path>,
    title: Option<&str>,
    author: Option<&str>,
) -> Result<(Option<String>, Option<String>), String> {
    let (mut t, mut a) = (title.map(str::to_string), author.map(str::to_string));
    if let Some(path) = input
        && (t.is_none() || a.is_none())
    {
        let bytes =
            std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let book = read_epub(&bytes).map_err(|e| e.to_string())?.book;
        if t.is_none() && !book.metadata.title.is_empty() && book.metadata.title != "Untitled" {
            t = Some(book.metadata.title.clone());
        }
        if a.is_none()
            && let Some(first) = book.metadata.authors.first()
        {
            a = Some(first.name.clone());
        }
    }
    Ok((t, a))
}

#[cfg(feature = "online")]
fn search(
    input: Option<&Path>,
    title: Option<&str>,
    author: Option<&str>,
    isbn: Option<&str>,
    limit: usize,
    report: ReportArg,
) -> ExitCode {
    let (title, author) = match seed_query(input, title, author) {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(ERROR_EXIT_CODE);
        }
    };

    let mut lookup = Lookup::new();
    let candidates = match lookup.search(title.as_deref(), author.as_deref(), isbn, limit) {
        Ok(candidates) => candidates,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(ERROR_EXIT_CODE);
        }
    };

    match report {
        ReportArg::Json => print_candidates_json(&candidates),
        ReportArg::Human => print_candidates_human(&candidates),
    }
    ExitCode::SUCCESS
}

#[cfg(feature = "online")]
fn print_candidates_json(candidates: &[Candidate]) {
    let payload = serde_json::json!({
        "schema": SCHEMA_VERSION,
        "candidates": candidates,
        // Said out loud so a UI can show it, and so nobody has to go and read a
        // licence page to find out whether they may keep what they just fetched.
        "source_licence": "Open Library metadata is CC0; cover images are not.",
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&payload).expect("serializable")
    );
}

#[cfg(feature = "online")]
fn print_candidates_human(candidates: &[Candidate]) {
    if candidates.is_empty() {
        println!("Nothing found. Try --title/--author, or an --isbn if you have one.");
        return;
    }
    for (i, c) in candidates.iter().enumerate() {
        let m = &c.metadata;
        let authors = m
            .authors
            .as_ref()
            .map(|a| {
                let json = serde_json::to_value(a).expect("serializable");
                render(&json)
            })
            .unwrap_or_else(|| "unknown author".to_string());
        let year = m.date.as_deref().unwrap_or("?");
        let publisher = m.publisher.as_deref().unwrap_or("-");
        println!(
            "{}) {} - {} ({}, {})",
            i + 1,
            m.title.as_deref().unwrap_or("untitled"),
            authors,
            year,
            publisher
        );
        println!("   {}", c.r#ref);
    }
    println!(
        "\nFetch one with:  epub-tailor metadata fetch {}",
        candidates[0].r#ref
    );
    println!("Then apply it:   epub-tailor fit book.epub --metadata meta.json");
}

#[cfg(feature = "online")]
fn fetch(reference: &str, cover_out: Option<&Path>, report: ReportArg) -> ExitCode {
    let mut lookup = Lookup::new();
    let (mut doc, cover_url) = match lookup.fetch(reference) {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(ERROR_EXIT_CODE);
        }
    };

    if let Some(path) = cover_out {
        match cover_url {
            None => eprintln!("warning: Open Library has no cover for {reference}"),
            Some(url) => {
                // Said plainly, every time: the metadata is CC0 but the cover art
                // is not, and the user is about to embed it in a file.
                eprintln!(
                    "note: Open Library's metadata is CC0, but its cover images come from many \
                     sources and are not. Check that you may use this one."
                );
                match lookup.fetch_cover(&url) {
                    Ok((data, _media_type)) => {
                        if let Err(e) = std::fs::write(path, &data) {
                            eprintln!("error: cannot write {}: {e}", path.display());
                            return ExitCode::from(ERROR_EXIT_CODE);
                        }
                        doc.cover = Some(path.display().to_string());
                    }
                    Err(e) => {
                        eprintln!("error: {e}");
                        return ExitCode::from(ERROR_EXIT_CODE);
                    }
                }
            }
        }
    }

    match report {
        // The document is the payload, unwrapped: this is what `fit --metadata`
        // eats, so `metadata fetch REF | fit book.epub --metadata -` just works.
        ReportArg::Json => println!(
            "{}",
            serde_json::to_string_pretty(&doc).expect("serializable")
        ),
        ReportArg::Human => {
            let json = serde_json::to_value(&doc).expect("serializable");
            for (key, value) in json.as_object().cloned().unwrap_or_default() {
                println!("{key:<14} {}", render(&value));
            }
        }
    }
    ExitCode::SUCCESS
}

#[cfg(feature = "online")]
fn pick(input: &Path, output: Option<&Path>, limit: usize) -> ExitCode {
    // The quarantine. Everything else in this binary is safe to drive from a
    // script or a GUI because it never asks anything; this one does, so it
    // insists on a human being there.
    if !std::io::stdin().is_terminal() {
        eprintln!(
            "error: `metadata pick` is interactive and stdin is not a terminal.\n\
             Drive it non-interactively instead:\n\
             \x20 epub-tailor metadata search {} --report json\n\
             \x20 epub-tailor metadata fetch <ref> > meta.json\n\
             \x20 epub-tailor fit {} --metadata meta.json",
            input.display(),
            input.display()
        );
        return ExitCode::from(ERROR_EXIT_CODE);
    }

    let (title, author) = match seed_query(Some(input), None, None) {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(ERROR_EXIT_CODE);
        }
    };

    let mut lookup = Lookup::new();
    let candidates = match lookup.search(title.as_deref(), author.as_deref(), None, limit) {
        Ok(candidates) => candidates,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(ERROR_EXIT_CODE);
        }
    };
    if candidates.is_empty() {
        eprintln!("Nothing found for this book. Try `metadata search` with --title/--author.");
        return ExitCode::from(ERROR_EXIT_CODE);
    }

    print_candidates_human(&candidates);
    print!("\nWhich one? [1-{}, or q to cancel]: ", candidates.len());
    let _ = std::io::stdout().flush();

    let mut answer = String::new();
    if std::io::stdin().read_line(&mut answer).is_err() {
        eprintln!("error: could not read your answer");
        return ExitCode::from(ERROR_EXIT_CODE);
    }
    let answer = answer.trim();
    if answer.eq_ignore_ascii_case("q") || answer.is_empty() {
        eprintln!("Cancelled; nothing written.");
        return ExitCode::SUCCESS;
    }
    let Some(chosen) = answer
        .parse::<usize>()
        .ok()
        .filter(|n| (1..=candidates.len()).contains(n))
        .and_then(|n| candidates.get(n - 1))
    else {
        eprintln!("error: \"{answer}\" is not one of the choices");
        return ExitCode::from(ERROR_EXIT_CODE);
    };

    // Fetch the full record: the search doc has no description.
    let doc = match lookup.fetch(&chosen.r#ref) {
        Ok((doc, _cover)) => doc,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(ERROR_EXIT_CODE);
        }
    };

    let out = output
        .map(PathBuf::from)
        .unwrap_or_else(|| default_doc_path(input));
    let json = serde_json::to_string_pretty(&doc).expect("serializable");
    if let Err(e) = std::fs::write(&out, format!("{json}\n")) {
        eprintln!("error: cannot write {}: {e}", out.display());
        return ExitCode::from(ERROR_EXIT_CODE);
    }

    println!("\nWrote {}", out.display());
    println!(
        "Apply it with:  epub-tailor fit {} --metadata {}",
        input.display(),
        out.display()
    );
    ExitCode::SUCCESS
}

/// `book.epub` -> `book.metadata.json`.
#[cfg(feature = "online")]
fn default_doc_path(input: &Path) -> PathBuf {
    input.with_extension("metadata.json")
}
