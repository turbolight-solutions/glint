use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args as ClapArgs, ValueEnum};
use glint_diag::{
    Diagnostic, Files, Label, LineIndex, Severity, Versions, render_human, render_json,
};
use glint_syntax::{ParseOptions, SyntaxError, parse_source_with};

const EXIT_OK: u8 = 0;
const EXIT_ERRORS: u8 = 1;
const EXIT_FAILURE: u8 = 2;

#[derive(ClapArgs)]
pub struct Args {
    /// Files to parse; `-` reads standard input
    #[arg(required = true)]
    files: Vec<PathBuf>,
    /// Print the concrete syntax tree
    #[arg(long)]
    tree: bool,
    /// Treat names reserved only by later editions as ordinary names, as for old RFC modules
    #[arg(long)]
    legacy_names: bool,
    #[arg(long, value_enum, default_value_t = Format::Human)]
    format: Format,
    #[arg(long, value_enum, default_value_t = Color::Auto)]
    color: Color,
}

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    Human,
    Json,
}

#[derive(Clone, Copy, ValueEnum)]
enum Color {
    Auto,
    Always,
    Never,
}

impl Color {
    fn enabled(self) -> bool {
        match self {
            Self::Always => true,
            Self::Never => false,
            Self::Auto => std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none(),
        }
    }
}

pub fn run(args: &Args) -> ExitCode {
    let mut files = Files::new();
    let mut diagnostics = Vec::new();
    let mut trees = Vec::new();
    let options = ParseOptions {
        legacy_names: args.legacy_names,
    };

    for path in &args.files {
        let text = match read_input(path) {
            Ok(text) => text,
            Err(error) => {
                eprintln!("glint: cannot read {}: {error}", path.display());
                return ExitCode::from(EXIT_FAILURE);
            }
        };
        let parse = parse_source_with(&text, options);
        if args.tree {
            trees.push(format!("{:#?}", parse.syntax()));
        }
        let (id, index) = files.add(display_name(path), text);
        diagnostics.extend(
            parse
                .errors()
                .iter()
                .map(|error| convert(error, id, &index)),
        );
    }

    let mut stdout = std::io::stdout().lock();
    let written = match args.format {
        Format::Human => {
            for tree in &trees {
                let _ = writeln!(stdout, "{tree}");
            }
            write!(
                stdout,
                "{}",
                render_human(&diagnostics, &files, args.color.enabled())
            )
        }
        Format::Json => writeln!(
            stdout,
            "{}",
            render_json(
                &diagnostics,
                Versions {
                    glint: env!("CARGO_PKG_VERSION"),
                    data: None,
                },
            )
        ),
    };
    if written.is_err() {
        return ExitCode::from(EXIT_FAILURE);
    }

    let failed = diagnostics.iter().any(|d| d.severity == Severity::Error);
    ExitCode::from(if failed { EXIT_ERRORS } else { EXIT_OK })
}

fn read_input(path: &Path) -> std::io::Result<String> {
    if path.as_os_str() == "-" {
        let mut text = String::new();
        std::io::stdin().read_to_string(&mut text)?;
        Ok(text)
    } else {
        std::fs::read_to_string(path)
    }
}

fn display_name(path: &Path) -> String {
    if path.as_os_str() == "-" {
        "<stdin>".to_owned()
    } else {
        path.display().to_string()
    }
}

fn convert(error: &SyntaxError, file: glint_diag::FileId, index: &LineIndex) -> Diagnostic {
    let span = usize::from(error.range.start())..usize::from(error.range.end());
    let severity = match error.severity {
        glint_syntax::Severity::Error => Severity::Error,
        glint_syntax::Severity::Warning => Severity::Warning,
        glint_syntax::Severity::Info => Severity::Info,
    };
    Diagnostic::new(
        error.code,
        error.message.clone(),
        Label::new(file, span, index),
    )
    .with_severity(severity)
}
