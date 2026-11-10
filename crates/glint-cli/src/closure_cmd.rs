use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Args as ClapArgs;
use glint_check::{
    ArcOrigin, Closure, ClosureInput, ClosureOptions, Problem, Role, Warning, closure,
};
use glint_corpus::{Corpus, CorpusProvider, Oid};

use crate::common::{EXIT_ERRORS, EXIT_FAILURE, EXIT_OK, load_corpus};

#[derive(ClapArgs)]
pub struct Args {
    /// Module files to start from; each holds one or more module definitions
    files: Vec<PathBuf>,
    /// A corpus module to start from: `rfc9881#X509-ML-DSA-2025`, or `rfc4998#ERS@1.3.6.1.5.5.11.0.2.1` when the
    /// document defines the name twice
    #[arg(long = "module", value_name = "DOC#NAME[@OID]")]
    modules: Vec<String>,
    /// Directory to write `modules/<Name>.asn` per module, `modules.tsv` and `combined.asn` to; it must be empty
    /// or absent
    #[arg(short, long, value_name = "DIR")]
    out: PathBuf,
    /// Corpus data (`corpus.json`) to use instead of the downloaded one
    #[arg(long)]
    corpus: Option<PathBuf>,
    /// Resolve imports of `Name` to the module published in `rfcNNNN`, or to the module with that OID
    #[arg(long = "pin", value_name = "NAME=rfcNNNN|OID")]
    pins: Vec<String>,
    /// Give modules of one name names of their own (`PKIX1Explicit88-rfc2459`), and rename an import resolved by
    /// its OID to a module of another name to that module
    #[arg(long)]
    rename: bool,
    /// Give an OID arc with no number (`TBD`, `TBDMOD`) in an Internet-Draft or an input file the number the set or
    /// the corpus gives it elsewhere, or one from 10000 up; an RFC's is left as written, with a warning
    #[arg(long)]
    renumber: bool,
}

/// The module texts the closure starts from, or every reason one cannot be read.
fn gather(args: &Args, corpus: &Corpus) -> Result<Vec<ClosureInput>, Vec<String>> {
    let mut inputs = Vec::new();
    let mut problems = Vec::new();
    for path in &args.files {
        match std::fs::read(path) {
            Ok(bytes) => inputs.push(ClosureInput::Text {
                label: path.display().to_string(),
                text: String::from_utf8_lossy(&bytes).into_owned(),
            }),
            Err(error) => problems.push(format!("{}: cannot read: {error}", path.display())),
        }
    }
    for reference in &args.modules {
        match corpus_module(corpus, reference) {
            Ok(record) => inputs.push(ClosureInput::Corpus { record }),
            Err(problem) => problems.push(format!("{reference}: {problem}")),
        }
    }
    if problems.is_empty() {
        Ok(inputs)
    } else {
        Err(problems)
    }
}

/// The index of the corpus module `DOC#NAME[@OID]`.
fn corpus_module(corpus: &Corpus, reference: &str) -> Result<usize, String> {
    let (document, rest) = reference
        .split_once('#')
        .ok_or("expected DOC#NAME or DOC#NAME@OID")?;
    let (name, oid) = match rest.split_once('@') {
        Some((name, oid)) => (
            name,
            Some(
                oid.parse::<Oid>()
                    .map_err(|_| format!("`{oid}` is not a dotted OID"))?,
            ),
        ),
        None => (rest, None),
    };
    let matching: Vec<(usize, &glint_corpus::ModuleRecord)> = corpus
        .by_name(name)
        .iter()
        .filter_map(|&index| corpus.modules().get(index).map(|m| (index, m)))
        .filter(|(_, m)| m.source.document.eq_ignore_ascii_case(document))
        .filter(|(_, m)| oid.as_ref().is_none_or(|oid| m.oid.as_ref() == Some(oid)))
        .collect();
    match matching.as_slice() {
        [(index, _)] => Ok(*index),
        [] => Err("not in the corpus".to_owned()),
        several => {
            let oids: Vec<String> = several
                .iter()
                .map(|(_, m)| {
                    m.oid
                        .as_ref()
                        .map_or_else(|| "none".to_owned(), ToString::to_string)
                })
                .collect();
            Err(format!(
                "the document defines it more than once; add @OID ({})",
                oids.join(", ")
            ))
        }
    }
}

fn parse_pins(pins: &[String]) -> Result<Vec<(String, String)>, String> {
    pins.iter()
        .map(|pin| match pin.split_once('=') {
            Some((name, target))
                if !name.is_empty()
                    && (target.starts_with("rfc") || target.parse::<Oid>().is_ok()) =>
            {
                Ok((name.to_owned(), target.to_owned()))
            }
            _ => Err(format!(
                "--pin expects `Name=rfcNNNN` or `Name=<oid>`, got `{pin}`"
            )),
        })
        .collect()
}

pub fn run(args: &Args) -> ExitCode {
    let pins = match parse_pins(&args.pins) {
        Ok(pins) => pins,
        Err(message) => return failure(&message),
    };
    if args.files.is_empty() && args.modules.is_empty() {
        return failure("give module files or --module DOC#NAME");
    }
    if let Err(message) = check_out_dir(&args.out) {
        return failure(&message);
    }
    let corpus = match load_corpus(args.corpus.as_deref()) {
        Ok(corpus) => corpus,
        Err(message) => return failure(&message),
    };
    let inputs = match gather(args, &corpus) {
        Ok(inputs) => inputs,
        Err(problems) => return incomplete(&problems),
    };
    let rewrites = ClosureOptions {
        rename: args.rename,
        renumber: args.renumber,
    };
    let result = match closure(&inputs, &corpus, &pins, rewrites) {
        Ok(result) => result,
        Err(problems) => {
            let mut lines: Vec<String> = problems.iter().map(Problem::to_string).collect();
            if problems
                .iter()
                .any(|p| matches!(p, Problem::SameName { .. }))
            {
                lines.push("--rename gives each module a name of its own".to_owned());
            }
            return incomplete(&lines);
        }
    };
    report(&result);
    let header = combined_header(&result, &corpus.info().build);
    if let Err(message) = write_out(&args.out, &result, &header) {
        return failure(&message);
    }
    ExitCode::from(EXIT_OK)
}

fn failure(message: &str) -> ExitCode {
    eprintln!("glint: {message}");
    ExitCode::from(EXIT_FAILURE)
}

/// The module set is not complete: one line naming every problem.
fn incomplete(problems: &[String]) -> ExitCode {
    eprintln!("glint: error: {}", problems.join("; "));
    ExitCode::from(EXIT_ERRORS)
}

fn check_out_dir(dir: &Path) -> Result<(), String> {
    match std::fs::read_dir(dir) {
        Ok(mut entries) => {
            if entries.next().is_some() {
                Err(format!("{} is not empty", dir.display()))
            } else {
                Ok(())
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("cannot read {}: {error}", dir.display())),
    }
}

fn arc_origin(origin: &ArcOrigin) -> String {
    match origin {
        ArcOrigin::Copied { from } => format!("copied from {from}"),
        ArcOrigin::AutoAssigned => "auto-assigned".to_owned(),
    }
}

/// The option that fixes `warning`, or what does when no option does.
fn fixed_by(warning: &Warning) -> &'static str {
    match warning {
        Warning::ImportName(_) => "--rename renames the import",
        Warning::Placeholder { .. } => "--renumber gives it a number",
        Warning::PublishedPlaceholder { .. } => {
            "only a correction of the document, such as an RFC erratum, can number it; --renumber numbers drafts \
             and input files only"
        }
    }
}

/// Tells on standard error what was left as written that a compiler may reject, every rewrite, and import cycles.
fn report(result: &Closure) {
    for warning in &result.warnings {
        eprintln!("glint: warning: {warning} ({})", fixed_by(warning));
    }
    for rename in &result.module_renames {
        eprintln!(
            "glint: {}: renamed to {}, since the set needs another module named {}",
            rename.source, rename.to, rename.from
        );
    }
    for rename in &result.renames {
        eprintln!(
            "glint: {}: import of {} renamed to {}, the module it resolves to",
            rename.importer, rename.from, rename.to
        );
    }
    for fill in &result.arcs {
        let sites: Vec<String> = fill
            .sites
            .iter()
            .map(|(module, written)| format!("{module} ({written})"))
            .collect();
        eprintln!(
            "glint: OID: {} gets arc {} ({}) in {}",
            fill.key,
            fill.arc,
            arc_origin(&fill.origin),
            sites.join(", ")
        );
    }
    for cycle in &result.cycles {
        eprintln!(
            "glint: import cycle: {}; the order of these modules is approximate",
            cycle.join(" -> ")
        );
    }
}

fn combined_header(result: &Closure, build: &str) -> String {
    let mut header = String::new();
    let labels: Vec<&str> = result
        .modules
        .iter()
        .filter(|m| m.role == Role::Input)
        .map(|m| m.source.as_str())
        .collect();
    let _ = writeln!(header, "-- Import closure of {}", labels.join(", "));
    let _ = writeln!(
        header,
        "-- glint {}, corpus data {build}",
        env!("CARGO_PKG_VERSION")
    );
    let _ = writeln!(header, "--\n-- Modules, dependencies first:");
    for module in &result.modules {
        let _ = writeln!(
            header,
            "--   {}  {}  {}",
            module.name,
            module.role.as_str(),
            module.source
        );
    }
    if !result.module_renames.is_empty() {
        let _ = writeln!(
            header,
            "--\n-- Modules renamed, since the set needs two modules of their name:"
        );
        for rename in &result.module_renames {
            let _ = writeln!(
                header,
                "--   {}: {} -> {}",
                rename.source, rename.from, rename.to
            );
        }
    }
    if !result.renames.is_empty() {
        let _ = writeln!(
            header,
            "--\n-- Imports renamed to the module they resolve to:"
        );
        for rename in &result.renames {
            let _ = writeln!(
                header,
                "--   {}: {} -> {}",
                rename.importer, rename.from, rename.to
            );
        }
    }
    if !result.arcs.is_empty() {
        let _ = writeln!(header, "--\n-- Object identifier arcs that had no number:");
        for fill in &result.arcs {
            let sites: Vec<String> = fill
                .sites
                .iter()
                .map(|(module, written)| format!("{module} ({written})"))
                .collect();
            let _ = writeln!(
                header,
                "--   {}: {}, {}, in {}",
                fill.key,
                fill.arc,
                arc_origin(&fill.origin),
                sites.join(", ")
            );
        }
    }
    if !result.warnings.is_empty() {
        let _ = writeln!(
            header,
            "--\n-- Left as written, though a compiler may reject it:"
        );
        for warning in &result.warnings {
            let _ = writeln!(header, "--   {warning} ({})", fixed_by(warning));
        }
    }
    if !result.cycles.is_empty() {
        let _ = writeln!(header, "--\n-- Import cycles (order approximate):");
        for cycle in &result.cycles {
            let _ = writeln!(header, "--   {}", cycle.join(" -> "));
        }
    }
    header
}

/// The subdirectory of the output that holds one `<Name>.asn` per module.
const MODULES_DIR: &str = "modules";

fn write_out(dir: &Path, result: &Closure, header: &str) -> Result<(), String> {
    let write = |name: &str, contents: &str| {
        let path = dir.join(name);
        std::fs::write(&path, contents)
            .map_err(|error| format!("cannot write {}: {error}", path.display()))
    };
    // The module files have a directory of their own, so `modules/*.asn` is exactly the set and combined.asn can
    // never be taken for a module file or collide with one (`Combined.asn` on a case-insensitive file system).
    let modules = dir.join(MODULES_DIR);
    std::fs::create_dir_all(&modules)
        .map_err(|error| format!("cannot create {}: {error}", modules.display()))?;
    let mut table = String::new();
    let mut combined = header.to_owned();
    for module in &result.modules {
        write(
            &format!("{MODULES_DIR}/{}.asn", module.name),
            &format!("{}\n", module.text),
        )?;
        let _ = writeln!(
            table,
            "{}\t{}\t{}",
            module.name,
            module.role.as_str(),
            module.source
        );
        let _ = write!(combined, "\n{}\n", module.text);
    }
    write("modules.tsv", &table)?;
    write("combined.asn", &combined)
}
