//! The `glint` command line.

mod check_cmd;
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
    /// Browse the RFC module corpus
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
        Command::Corpus(args) => corpus_cmd::run(&args),
        Command::Explain(args) => explain_cmd::run(&args),
        Command::Extract(args) => extract_cmd::run(&args),
        Command::Parse(args) => parse_cmd::run(&args),
    }
}
