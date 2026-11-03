use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args as ClapArgs, ValueEnum};
use glint_check::{CheckOptions, Config, DocumentRef, Report, check_document};
use glint_diag::{SarifInput, Severity, render_human, render_json, render_sarif};
use glint_extract::{Format, detect_format};

use crate::common::{
    Color, EXIT_ERRORS, EXIT_FAILURE, EXIT_OK, display_name, find_config, load_corpus, read_input,
};
use crate::net::fetch_document;

#[derive(ClapArgs)]
pub struct Args {
    /// Documents to check (RFCXML, Markdown, plain text or a bare `.asn` module); `-` reads standard input
    #[arg(required_unless_present = "draft")]
    files: Vec<PathBuf>,
    /// Fetch an Internet-Draft (`draft-ietf-lamps-foo-03`, or without a revision for the latest) or an RFC (`rfc5280`) from the IETF archive and check it
    #[arg(long, value_name = "NAME")]
    draft: Vec<String>,
    #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
    format: OutputFormat,
    /// Path to `glint.toml`; by default the nearest one above the input
    #[arg(long)]
    config: Option<PathBuf>,
    /// Exit with status 1 when there are warnings too
    #[arg(long)]
    deny_warnings: bool,
    /// Corpus bundle to use instead of the embedded one
    #[arg(long)]
    corpus: Option<PathBuf>,
    /// Resolve imports of `Name` to the module published in `rfcNNNN`
    #[arg(long = "pin", value_name = "NAME=rfcNNNN")]
    pins: Vec<String>,
    /// Do not compare snippets in the text with the modules
    #[arg(long)]
    no_snippets: bool,
    #[arg(long, value_enum, default_value_t = Color::Auto)]
    color: Color,
}

#[derive(Clone, Copy, ValueEnum)]
enum OutputFormat {
    Human,
    Json,
    /// SARIF 2.1.0, for code-scanning annotations
    Sarif,
}

/// A document to check and where to look for `glint.toml`.
struct Input {
    name: String,
    text: String,
    format: Format,
    config_from: PathBuf,
}

pub fn run(args: &Args) -> ExitCode {
    let corpus = match load_corpus(args.corpus.as_deref()) {
        Ok(corpus) => corpus,
        Err(message) => {
            eprintln!("glint: {message}");
            return ExitCode::from(EXIT_FAILURE);
        }
    };
    let mut pins = Vec::new();
    for pin in &args.pins {
        match pin.split_once('=') {
            Some((name, rfc)) if !name.is_empty() && rfc.starts_with("rfc") => {
                pins.push((name.to_owned(), rfc.to_owned()));
            }
            _ => {
                eprintln!("glint: --pin expects `Name=rfcNNNN`, got `{pin}`");
                return ExitCode::from(EXIT_FAILURE);
            }
        }
    }
    let inputs = match gather_inputs(args) {
        Ok(inputs) => inputs,
        Err(message) => {
            eprintln!("glint: {message}");
            return ExitCode::from(EXIT_FAILURE);
        }
    };
    let mut exit = EXIT_OK;
    let mut reports: Vec<Report> = Vec::new();
    for input in &inputs {
        let config = match load_config(args, &input.config_from) {
            Ok(config) => config,
            Err(message) => {
                eprintln!("glint: {message}");
                return ExitCode::from(EXIT_FAILURE);
            }
        };
        let options = CheckOptions {
            config,
            pins: pins.clone(),
            no_snippets: args.no_snippets,
        };
        let report = check_document(&input.name, &input.text, input.format, &corpus, &options);
        let output = match args.format {
            OutputFormat::Human => {
                let rendered = render_human(
                    &report.diagnostics,
                    &report.files,
                    args.color.enabled_for_stderr(),
                );
                write!(std::io::stderr(), "{rendered}")
            }
            OutputFormat::Json => {
                writeln!(std::io::stdout(), "{}", render_json(&report.diagnostics))
            }
            OutputFormat::Sarif => Ok(()),
        };
        if output.is_err() {
            return ExitCode::from(EXIT_FAILURE);
        }
        let errors = report
            .diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error);
        let warnings = report
            .diagnostics
            .iter()
            .any(|d| d.severity == Severity::Warning);
        if errors || (args.deny_warnings && warnings) {
            exit = EXIT_ERRORS;
        }
        reports.push(report);
    }
    if matches!(args.format, OutputFormat::Sarif) {
        let sarif_inputs: Vec<SarifInput<'_>> = reports
            .iter()
            .map(|report| SarifInput {
                diagnostics: &report.diagnostics,
                files: &report.files,
            })
            .collect();
        let rendered = render_sarif(&sarif_inputs, env!("CARGO_PKG_VERSION"));
        if writeln!(std::io::stdout(), "{rendered}").is_err() {
            return ExitCode::from(EXIT_FAILURE);
        }
    }
    ExitCode::from(exit)
}

fn gather_inputs(args: &Args) -> Result<Vec<Input>, String> {
    let mut inputs = Vec::new();
    for path in &args.files {
        let text = read_input(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        inputs.push(Input {
            name: display_name(path),
            format: detect_format(Some(path), &text),
            text,
            config_from: path.clone(),
        });
    }
    for name in &args.draft {
        let reference = DocumentRef::parse(name).map_err(|e| format!("`{name}`: {e}"))?;
        let fetched = fetch_document(&reference)?;
        inputs.push(Input {
            name: fetched.name,
            text: fetched.text,
            format: fetched.format,
            config_from: PathBuf::from("-"),
        });
    }
    Ok(inputs)
}

fn load_config(args: &Args, input: &std::path::Path) -> Result<Config, String> {
    let path = args.config.clone().or_else(|| find_config(input));
    let Some(path) = path else {
        return Ok(Config::default());
    };
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    Config::from_toml(&text).map_err(|e| format!("{}: {e}", path.display()))
}
