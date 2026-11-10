//! The `glint` command line.

mod check_cmd;
mod closure_cmd;
mod common;
mod corpus_cmd;
mod explain_cmd;
mod extract_cmd;
mod net;
mod parse_cmd;

use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "glint",
    version,
    about = "An ASN.1 checker for Internet-Draft and RFC authors"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Check the ASN.1 modules in a document
    Check(check_cmd::Args),
    /// Write a module set and every module it imports, one module per file, for a compiler
    ///
    /// Imports are resolved as `glint check` resolves them, and every module is written as it is, unless an option asks
    /// for a rewrite. A set that needs two modules of one name is an error without `--rename`, since one file per name
    /// cannot hold them. Whatever else a compiler that reads one module per file and finds imports by name may reject
    /// is a warning, on stderr and in the header of combined.asn, naming the option that rewrites it: `--rename` for an
    /// import resolved by its OID to a module of another name, and `--renumber` for an OID arc with no number (`TBD`)
    /// in an Internet-Draft or an input file. Such an arc in an RFC needs an erratum: it is a warning that no option
    /// fixes. Every rewrite is reported too.
    ///
    /// The output directory gets `modules/<Name>.asn` per module, `modules.tsv` (name, input or dependency,
    /// source; dependencies first) and `combined.asn` (every module in one file, dependencies first).
    Closure(closure_cmd::Args),
    /// Browse the module corpus (RFCs, active Internet-Drafts and standalone modules) and update its data
    Corpus(corpus_cmd::Args),
    /// Describe a diagnostic code, with examples
    Explain(explain_cmd::Args),
    /// Extract ASN.1 modules from an RFCXML, Markdown or plain-text document
    Extract(extract_cmd::Args),
    /// Parse ASN.1 modules and report syntax errors (development command)
    Parse(parse_cmd::Args),
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Check(args) => check_cmd::run(&args),
        Command::Closure(args) => closure_cmd::run(&args),
        Command::Corpus(args) => corpus_cmd::run(&args),
        Command::Explain(args) => explain_cmd::run(&args),
        Command::Extract(args) => extract_cmd::run(&args),
        Command::Parse(args) => parse_cmd::run(&args),
    }
}
