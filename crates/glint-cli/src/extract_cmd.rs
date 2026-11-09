use std::collections::HashMap;
use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args as ClapArgs, ValueEnum};
use glint_diag::{
    Diagnostic, Files, Label, LineIndex, Severity, Versions, render_human, render_json,
};
use glint_extract::{Block, BlockKind, ExtractNote, Format, detect_format, extract, module_slices};
use glint_sema::{Evaluator, ModuleModel};
use glint_syntax::{ParseOptions, ast, parse_source_with};
use serde::Serialize;

/// Version of the `--format json` block listing; bumped when a field changes meaning or goes away.
const LISTING_SCHEMA: u32 = 1;

const EXIT_OK: u8 = 0;
const EXIT_ERRORS: u8 = 1;
const EXIT_FAILURE: u8 = 2;

#[derive(ClapArgs)]
pub struct Args {
    /// Document to extract from; `-` reads standard input
    file: PathBuf,
    /// Write each extracted module to `<DIR>/<ModuleName>.asn`. Without it the blocks found are listed; with
    /// `--format json` they are listed on standard output as JSON either way, naming the files written
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

    let outcome = args
        .output
        .as_ref()
        .map(|dir| write_modules(dir, &extraction.blocks, args.fragments))
        .transpose()
        .and_then(|written| match (args.format, written) {
            (OutputFormat::Json, written) => {
                list_blocks_json(&extraction.blocks, &index, written.as_deref())
            }
            (OutputFormat::Human, None) => {
                list_blocks(&extraction.blocks, &index);
                Ok(())
            }
            (OutputFormat::Human, Some(_)) => Ok(()),
        });
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
        let names: Vec<String> = modules_of(block)
            .into_iter()
            .map(|module| module.name.unwrap_or_else(|| "?".to_owned()))
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

/// A module of a block: its name, its text trimmed as the corpus build records it, and where that text starts in
/// the block's text.
struct Module<'a> {
    name: Option<String>,
    text: &'a str,
    start: usize,
}

/// The modules of `block`; a fragment has none, even when it quotes a module header (as corpus-build reads it).
fn modules_of(block: &Block) -> Vec<Module<'_>> {
    if block.kind == BlockKind::Fragment {
        return Vec::new();
    }
    module_slices(block)
        .into_iter()
        .filter_map(|slice| {
            let untrimmed = block.text.get(slice.range.clone())?;
            let text = untrimmed.trim();
            let leading = untrimmed.len().saturating_sub(untrimmed.trim_start().len());
            Some(Module {
                name: slice.name,
                text,
                start: slice.range.start.saturating_add(leading),
            })
        })
        .collect()
}

/// The files `-o` wrote for one block: the block itself when it is a fragment written with `--fragments`, and one
/// per module.
#[derive(Default)]
struct Written {
    fragment: Option<String>,
    modules: Vec<String>,
}

#[derive(Serialize)]
struct Listing<'a> {
    schema_version: u32,
    glint_version: &'static str,
    blocks: Vec<BlockEntry<'a>>,
}

/// One block. `file` is the `fragment-<n>.asn` written for it with `-o` and `--fragments`, else `null`.
#[derive(Serialize)]
struct BlockEntry<'a> {
    kind: &'static str,
    section: Option<String>,
    title: Option<&'a str>,
    first_line: u32,
    last_line: u32,
    file: Option<String>,
    modules: Vec<ModuleEntry>,
}

/// One module of a block. `oid` is the module identifier in dotted form, or `null` when the header has none or it
/// is not all numbers once arc names are read. `file` is the file `-o` wrote the module to, relative to its
/// directory (its text trimmed as the corpus build records it, plus a final newline), else `null`.
#[derive(Serialize)]
struct ModuleEntry {
    name: Option<String>,
    oid: Option<String>,
    first_line: u32,
    last_line: u32,
    file: Option<String>,
}

/// The first and last line (1-based) of the byte range `range` of the original document.
fn line_range(index: &LineIndex, range: std::ops::Range<usize>) -> (u32, u32) {
    let first = index.line_col(range.start).0;
    let last = index
        .line_col(range.end.saturating_sub(1).max(range.start))
        .0;
    (first, last)
}

/// The dotted module identifier of the module definition `text`, when its header gives every arc a number.
fn module_oid(text: &str) -> Option<String> {
    let parse = parse_source_with(text, ParseOptions::corpus());
    let file = ast::SourceFile::cast(parse.syntax())?;
    let module = file.modules().next()?;
    let model = ModuleModel::build(&module)?;
    let oid = model.oid_group.as_ref().and_then(Evaluator::literal_oid)?;
    Some(oid.to_string())
}

/// Print the block listing as JSON; `written` holds, block by block, the files `-o` wrote, when it was given.
fn list_blocks_json(
    blocks: &[Block],
    index: &LineIndex,
    written: Option<&[Written]>,
) -> Result<(), String> {
    let entries = blocks
        .iter()
        .enumerate()
        .map(|(number, block)| {
            let files = written.and_then(|written| written.get(number));
            let (first_line, last_line) = line_range(index, block.origin.span.clone());
            let modules = modules_of(block)
                .into_iter()
                .enumerate()
                .map(|(position, module)| {
                    let (first, last) = block
                        .original_range(
                            module.start..module.start.saturating_add(module.text.len()),
                        )
                        .map_or((first_line, last_line), |range| line_range(index, range));
                    ModuleEntry {
                        name: module.name,
                        oid: module_oid(module.text),
                        first_line: first,
                        last_line: last,
                        file: files.and_then(|files| files.modules.get(position)).cloned(),
                    }
                })
                .collect();
            BlockEntry {
                kind: kind_name(block.kind),
                section: block.origin.section.as_ref().map(ToString::to_string),
                title: block.origin.title.as_deref(),
                first_line,
                last_line,
                file: files.and_then(|files| files.fragment.clone()),
                modules,
            }
        })
        .collect();
    let listing = Listing {
        schema_version: LISTING_SCHEMA,
        glint_version: env!("CARGO_PKG_VERSION"),
        blocks: entries,
    };
    let json = serde_json::to_string_pretty(&listing)
        .map_err(|error| format!("cannot write the block listing: {error}"))?;
    writeln!(std::io::stdout().lock(), "{json}")
        .map_err(|error| format!("cannot write the block listing: {error}"))
}

/// Write each block's modules (and, with `fragments`, each fragment) to `dir`; returns the files written, block by
/// block. A module is written to `<Name>.asn`. When the document defines several modules of one name (RFC 4998
/// defines `ERS` for 1988 and for 1997 syntax), each is written to `<Name>-<oid>.asn`, as overrides name them, so
/// the file says which module it holds whatever the order in the document.
fn write_modules(dir: &Path, blocks: &[Block], fragments: bool) -> Result<Vec<Written>, String> {
    std::fs::create_dir_all(dir)
        .map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
    let mut defined: HashMap<String, usize> = HashMap::new();
    for module in blocks.iter().flat_map(modules_of) {
        let count = defined.entry(module_name(&module)).or_default();
        *count = count.saturating_add(1);
    }
    let mut used: Vec<String> = Vec::new();
    let mut fragment_count = 0_u32;
    let mut written = Vec::with_capacity(blocks.len());
    for block in blocks {
        let modules = modules_of(block);
        let mut files = Written::default();
        if modules.is_empty() {
            if fragments {
                fragment_count = fragment_count.saturating_add(1);
                let name = format!("fragment-{fragment_count}");
                files.fragment = Some(write_file(dir, &name, &block.text, &mut used)?);
            }
        } else {
            for module in modules {
                let name = module_name(&module);
                let stem = match module_oid(module.text) {
                    Some(oid) if defined.get(&name).is_some_and(|&n| n > 1) => {
                        format!("{name}-{oid}")
                    }
                    _ => name,
                };
                files
                    .modules
                    .push(write_file(dir, &stem, module.text, &mut used)?);
            }
        }
        written.push(files);
    }
    Ok(written)
}

/// The name a module's file is named after.
fn module_name(module: &Module<'_>) -> String {
    module.name.clone().unwrap_or_else(|| "module".to_owned())
}

/// Write `body` without trailing whitespace, plus a newline, to `<dir>/<name>.asn`, or `<name>-<n>.asn` when that
/// name is taken (two modules of one name without a numeric OID, or with the same one); returns the file name.
fn write_file(
    dir: &Path,
    name: &str,
    body: &str,
    used: &mut Vec<String>,
) -> Result<String, String> {
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
    used.push(file_name.clone());
    Ok(file_name)
}
