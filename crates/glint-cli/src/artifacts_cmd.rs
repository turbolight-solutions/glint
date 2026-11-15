use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args as ClapArgs, Subcommand, ValueEnum};
use glint_corpus::{Artifact, ArtifactsData, Confidence};
use glint_decode::{base64, canonical_label, pem_armor};
use glint_extract::artifacts::pem_blocks;
use glint_extract::detect_format;
use serde::Serialize;

use crate::common::{EXIT_FAILURE, EXIT_OK, display_name, load_artifacts, read_input};

#[derive(ClapArgs)]
pub struct Args {
    /// Example artifacts (`artifacts.json`) to use instead of the ones beside the corpus data
    #[arg(long, global = true)]
    artifacts: Option<PathBuf>,
    /// Corpus data (`corpus.json`) whose `artifacts.json` to use
    #[arg(long, global = true)]
    corpus: Option<PathBuf>,
    #[command(subcommand)]
    command: Sub,
}

#[derive(Clone, Copy, ValueEnum)]
enum ConfidenceArg {
    Label,
    Decoded,
    Unknown,
}

impl ConfidenceArg {
    fn confidence(self) -> Confidence {
        match self {
            Self::Label => Confidence::Label,
            Self::Decoded => Confidence::Decoded,
            Self::Unknown => Confidence::Unknown,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    Pem,
    Base64,
    Der,
}

#[derive(Subcommand)]
enum Sub {
    /// List the example artifacts, with their label, identified type, confidence and decode findings
    List {
        /// Only the artifacts of this document (`rfc9500`, `draft-ietf-lamps-foo-03`)
        document: Option<String>,
        /// Only artifacts with this PEM label
        #[arg(long)]
        label: Option<String>,
        /// Only artifacts whose identified type contains this text (`Certificate`, `rfc5912#`)
        #[arg(long = "type")]
        ty: Option<String>,
        /// Only artifacts identified with this confidence
        #[arg(long, value_enum)]
        confidence: Option<ConfidenceArg>,
        /// Print JSON instead of text
        #[arg(long)]
        json: bool,
    },
    /// Write an artifact: PEM when its label is a known one, else base64 of the octets; DER on request
    Get {
        /// The artifact id (`rfc9500-4`)
        id: String,
        #[arg(long, value_enum)]
        format: Option<Format>,
        /// Write to this file instead of stdout
        #[arg(short, long, value_name = "FILE")]
        output: Option<PathBuf>,
    },
    /// List the PEM blocks of a local document, and the blocks that are not usable; needs no corpus data
    Extract {
        /// An RFCXML, Markdown or plain-text document
        file: PathBuf,
        /// Print JSON instead of text
        #[arg(long)]
        json: bool,
    },
}

fn failure(message: &str) -> ExitCode {
    eprintln!("glint: {message}");
    ExitCode::from(EXIT_FAILURE)
}

pub fn run(args: &Args) -> ExitCode {
    if let Sub::Extract { file, json } = &args.command {
        return extract(file, *json);
    }
    let data = match load_artifacts(args.artifacts.as_deref(), args.corpus.as_deref()) {
        Ok(data) => data,
        Err(message) => return failure(&message),
    };
    match &args.command {
        Sub::List {
            document,
            label,
            ty,
            confidence,
            json,
        } => {
            let selected: Vec<&Artifact> = data
                .artifacts
                .iter()
                .filter(|a| {
                    document
                        .as_ref()
                        .is_none_or(|d| a.document.eq_ignore_ascii_case(d))
                })
                .filter(|a| {
                    label
                        .as_ref()
                        .is_none_or(|l| a.label.eq_ignore_ascii_case(l))
                })
                .filter(|a| {
                    ty.as_ref().is_none_or(|t| {
                        a.root
                            .as_ref()
                            .is_some_and(|r| r.to_lowercase().contains(&t.to_lowercase()))
                    })
                })
                .filter(|a| confidence.is_none_or(|c| a.confidence == c.confidence()))
                .collect();
            list(&data, &selected, *json)
        }
        Sub::Get { id, format, output } => get(&data, id, *format, output.as_deref()),
        Sub::Extract { .. } => ExitCode::from(EXIT_OK),
    }
}

fn list(data: &ArtifactsData, selected: &[&Artifact], json: bool) -> ExitCode {
    let text = if json {
        match serde_json::to_string_pretty(selected) {
            Ok(json) => json + "\n",
            Err(error) => return failure(&format!("cannot write JSON: {error}")),
        }
    } else {
        let mut out = String::new();
        for artifact in selected {
            let mut line = format!(
                "{:<40} {:<24} {:<8} {}",
                artifact.id,
                artifact.label,
                artifact.confidence.as_str(),
                artifact.root.as_deref().unwrap_or("-")
            );
            if !artifact.findings.is_empty() {
                line.push_str(&format!("  [{}]", artifact.findings.join(" ")));
            }
            if artifact.obsoleted {
                line.push_str("  (obsoleted)");
            }
            out.push_str(&line);
            out.push('\n');
        }
        out
    };
    if std::io::stdout().lock().write_all(text.as_bytes()).is_err() {
        return ExitCode::from(EXIT_FAILURE);
    }
    eprintln!(
        "{} of {} example artifacts, data {}",
        selected.len(),
        data.artifacts.len(),
        data.build
    );
    ExitCode::from(EXIT_OK)
}

fn get(data: &ArtifactsData, id: &str, format: Option<Format>, output: Option<&Path>) -> ExitCode {
    let Some(artifact) = data.get(id) else {
        return failure(&format!(
            "no example artifact `{id}` in data {}",
            data.build
        ));
    };
    let Some(der) = base64(artifact.der.as_bytes()) else {
        return failure(&format!("the octets of `{id}` are damaged"));
    };
    let canonical = canonical_label(&artifact.label);
    let format = format.unwrap_or(if canonical.is_some() {
        Format::Pem
    } else {
        Format::Base64
    });
    let bytes = match format {
        Format::Der => der,
        Format::Pem => pem_armor(canonical.unwrap_or(&artifact.label), &der).into_bytes(),
        Format::Base64 => {
            if canonical.is_none() {
                eprintln!("glint: `{id}` is labelled `{}`", artifact.label);
            }
            glint_decode::base64_lines(&der).into_bytes()
        }
    };
    let written = match output {
        Some(path) => std::fs::write(path, &bytes)
            .map_err(|e| format!("cannot write {}: {e}", path.display())),
        None => std::io::stdout()
            .lock()
            .write_all(&bytes)
            .map_err(|e| e.to_string()),
    };
    match written {
        Ok(()) => ExitCode::from(EXIT_OK),
        Err(message) => failure(&message),
    }
}

/// One block of `glint artifacts extract --json`.
#[derive(Serialize)]
struct Extracted<'a> {
    index: usize,
    label: &'a str,
    line: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    section: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    caption: Option<&'a str>,
    /// The octets, base64; absent for a block that is not usable.
    #[serde(skip_serializing_if = "Option::is_none")]
    der: Option<String>,
    /// Why the block is not usable.
    #[serde(skip_serializing_if = "Option::is_none")]
    problem: Option<&'a str>,
}

fn extract(path: &Path, json: bool) -> ExitCode {
    let text = match read_input(path) {
        Ok(text) => text,
        Err(error) => return failure(&format!("cannot read {}: {error}", display_name(path))),
    };
    let scan = pem_blocks(&text, detect_format(Some(path), &text));
    let mut rows: Vec<Extracted<'_>> = scan
        .blocks
        .iter()
        .map(|b| Extracted {
            index: b.index,
            label: &b.label,
            line: b.place.line,
            section: b.place.section.as_ref().map(ToString::to_string),
            caption: b.place.caption.as_deref(),
            der: Some(glint_decode::base64_encode(&b.der)),
            problem: None,
        })
        .chain(scan.findings.iter().map(|f| Extracted {
            index: f.index,
            label: &f.label,
            line: f.place.line,
            section: f.place.section.as_ref().map(ToString::to_string),
            caption: f.place.caption.as_deref(),
            der: None,
            problem: Some(&f.message),
        }))
        .collect();
    rows.sort_by_key(|row| row.index);
    let out = if json {
        match serde_json::to_string_pretty(&rows) {
            Ok(json) => json + "\n",
            Err(error) => return failure(&format!("cannot write JSON: {error}")),
        }
    } else {
        let mut out = String::new();
        for row in &rows {
            let place = row.section.as_ref().map_or_else(
                || format!("line {}", row.line),
                |s| format!("line {}, {s}", row.line),
            );
            let what = match (&row.der, row.problem) {
                (_, Some(problem)) => problem.to_owned(),
                (Some(der), None) => {
                    format!("{} octets", base64(der.as_bytes()).map_or(0, |b| b.len()))
                }
                (None, None) => String::new(),
            };
            out.push_str(&format!(
                "{:>3}  {:<24} {place}: {what}\n",
                row.index, row.label
            ));
        }
        for (label, line) in &scan.openpgp {
            out.push_str(&format!(
                "  -  {label:<24} line {line}: OpenPGP armor, left out\n"
            ));
        }
        out
    };
    if std::io::stdout().lock().write_all(out.as_bytes()).is_err() {
        return ExitCode::from(EXIT_FAILURE);
    }
    ExitCode::from(if scan.findings.is_empty() {
        EXIT_OK
    } else {
        crate::common::EXIT_ERRORS
    })
}
