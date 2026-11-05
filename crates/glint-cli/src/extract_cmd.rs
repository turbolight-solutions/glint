use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args as ClapArgs, ValueEnum};
use glint_diag::{
    Diagnostic, Files, Label, LineIndex, Severity, Versions, render_human, render_json,
};
use glint_extract::{Block, BlockKind, ExtractNote, Format, detect_format, extract, module_slices};

const EXIT_OK: u8 = 0;
const EXIT_ERRORS: u8 = 1;
const EXIT_FAILURE: u8 = 2;

#[derive(ClapArgs)]
pub struct Args {
    /// Document to extract from; `-` reads standard input
    file: PathBuf,
    /// Write each extracted module to `<DIR>/<ModuleName>.asn`; without it, list the blocks found
    #[arg(short, long, value_name = "DIR")]
    output: Option<PathBuf>,
    /// Also write blocks that are not complete modules, as `fragment-<n>.asn`
    #[arg(long)]
    fragments: bool,
    #[arg(long, value_enum, default_value_t = InputFormat::Auto)]
    input_format: InputFormat,
    #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
    format: OutputFormat,
    #[arg(long, value_enum, default_value_t = Color::Auto)]
    color: Color,
}

#[derive(Clone, Copy, ValueEnum)]
enum InputFormat {
    Auto,
    Xml,
    Markdown,
    Text,
    Bare,
}

#[derive(Clone, Copy, ValueEnum)]
enum OutputFormat {
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
            Self::Auto => std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none(),
        }
    }
}

pub fn run(args: &Args) -> ExitCode {
    let text = match read_input(&args.file) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("glint: cannot read {}: {error}", args.file.display());
            return ExitCode::from(EXIT_FAILURE);
        }
    };
    let format = match args.input_format {
        InputFormat::Auto => detect_format(Some(&args.file), &text),
        InputFormat::Xml => Format::Xml,
        InputFormat::Markdown => Format::Markdown,
        InputFormat::Text => Format::Text,
        InputFormat::Bare => Format::Bare,
    };
    let extraction = extract(&text, format);

    let mut files = Files::new();
    let (id, index) = files.add_document(display_name(&args.file), text);
    let diagnostics: Vec<Diagnostic> = extraction
        .notes
        .iter()
        .map(|note| convert(note, id, &index))
        .collect();

    let outcome = match &args.output {
        Some(dir) => write_modules(dir, &extraction.blocks, args.fragments),
        None => {
            list_blocks(&extraction.blocks, &index);
            Ok(())
        }
    };
    if let Err(error) = outcome {
        eprintln!("glint: {error}");
        return ExitCode::from(EXIT_FAILURE);
    }

    let mut stderr = std::io::stderr().lock();
    let rendered = match args.format {
        OutputFormat::Human => write!(
            stderr,
            "{}",
            render_human(&diagnostics, &files, args.color.enabled())
        ),
        OutputFormat::Json => writeln!(
            stderr,
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
    if rendered.is_err() {
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

fn convert(note: &ExtractNote, file: glint_diag::FileId, index: &LineIndex) -> Diagnostic {
    let severity = match note.severity {
        glint_extract::Severity::Error => Severity::Error,
        glint_extract::Severity::Warning => Severity::Warning,
        glint_extract::Severity::Info => Severity::Info,
    };
    Diagnostic::new(
        note.code,
        note.message.clone(),
        Label::new(file, note.span.clone(), index),
    )
    .with_severity(severity)
}

fn kind_name(kind: BlockKind) -> &'static str {
    match kind {
        BlockKind::Module => "module",
        BlockKind::ModuleList => "modules",
        BlockKind::Fragment => "fragment",
    }
}

fn list_blocks(blocks: &[Block], index: &LineIndex) {
    let mut stdout = std::io::stdout().lock();
    for block in blocks {
        let names: Vec<String> = module_slices(block)
            .into_iter()
            .map(|slice| slice.name.unwrap_or_else(|| "?".to_owned()))
            .collect();
        let first = index.line_col(block.origin.span.start).0;
        let last = index.line_col(block.origin.span.end).0;
        let place = block
            .origin
            .section
            .as_ref()
            .map_or_else(String::new, |section| format!("  {section}"));
        let _ = writeln!(
            stdout,
            "{:<8} {}{}  lines {first}-{last}",
            kind_name(block.kind),
            names.join(", "),
            place
        );
    }
}

fn write_modules(dir: &Path, blocks: &[Block], fragments: bool) -> Result<(), String> {
    std::fs::create_dir_all(dir)
        .map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
    let mut used: Vec<String> = Vec::new();
    let mut fragment_count = 0_u32;
    for block in blocks {
        let slices = module_slices(block);
        if block.kind == BlockKind::Fragment || slices.is_empty() {
            if fragments {
                fragment_count = fragment_count.saturating_add(1);
                let name = format!("fragment-{fragment_count}");
                write_file(dir, &name, &block.text, &mut used)?;
            }
            continue;
        }
        for slice in slices {
            let name = slice.name.unwrap_or_else(|| "module".to_owned());
            let body = block.text.get(slice.range).unwrap_or_default();
            write_file(dir, &name, body, &mut used)?;
        }
    }
    Ok(())
}

fn write_file(dir: &Path, name: &str, body: &str, used: &mut Vec<String>) -> Result<(), String> {
    let mut file_name = format!("{name}.asn");
    let mut copy = 1_u32;
    while used.contains(&file_name) {
        copy = copy.saturating_add(1);
        file_name = format!("{name}-{copy}.asn");
    }
    let path = dir.join(&file_name);
    let mut contents = body.trim_end().to_owned();
    contents.push('\n');
    std::fs::write(&path, contents)
        .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    used.push(file_name);
    Ok(())
}
