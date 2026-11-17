//! The `glint` command line.

mod artifacts_cmd;
mod check_cmd;
mod closure_cmd;
mod codegen_cmd;
mod common;
mod corpus_cmd;
mod decode_cmd;
mod errata_cmd;
mod explain_cmd;
mod extract_cmd;
mod ir_cmd;
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
    /// List and download the PEM examples of the RFCs and active drafts, each identified as a type
    ///
    /// corpus-build extracts every PEM block of every RFC and active Internet-Draft and identifies its type: a root its
    /// label suggests (`label`), else a type of the document's own modules or a common corpus root that decodes it
    /// (`decoded`), else none (`unknown`). `glint decode --artifact ID` decodes one as its type.
    Artifacts(artifacts_cmd::Args),
    /// Check the ASN.1 modules in a document
    Check(check_cmd::Args),
    /// Write a module set and every module it imports, one module per file, for a compiler
    ///
    /// Imports are resolved as `glint check` resolves them, and every module is written as it is, unless an option asks
    /// for a rewrite. A set that needs two modules of one name is an error without `--rename`, since one file per name
    /// cannot hold them. Whatever else a compiler that reads one module per file and finds imports by name may reject
    /// is a warning, on stderr and in the header of combined.asn, naming the option that rewrites it: `--rename` for an
    /// import resolved by its OID to a module of another name, `--renumber` for an OID arc with no number (`TBD`) in
    /// an Internet-Draft or an input file, and `--builtin-strings` for 1988-syntax definitions of `UniversalString`,
    /// `BMPString` and `UTF8String`, which current ASN.1 has built in. An OID arc with no number in an RFC needs an
    /// erratum: it is a warning that no option fixes. Every rewrite is reported too.
    ///
    /// The output directory gets `modules/<Name>.asn` per module, `modules.tsv` (name, input or dependency,
    /// source; dependencies first) and `combined.asn` (every module in one file, dependencies first).
    Closure(closure_cmd::Args),
    /// Generate code for a type and everything it reaches, for a target library
    ///
    /// Takes a corpus module (`rfc5912#PKIX1Explicit-2009`, or a name only one document defines) or, with FILES, a
    /// module of user ASN.1, and writes one file per ASN.1 module plus the vendored runtime (`glint_rt`, MIT-0). The
    /// generated types decode and encode, resolve open types through their object sets, and check every constraint;
    /// what the target cannot express is reported on stderr (`glint explain W420`).
    Codegen(codegen_cmd::Args),
    /// Browse the module corpus (RFCs, active Internet-Drafts and standalone modules) and update its data
    Corpus(corpus_cmd::Args),
    /// Decode a BER or DER encoding (PEM, base64 or binary) as a type, like `openssl asn1parse` with names
    ///
    /// Every TLV is shown with the component and type it encodes and its value: OIDs named from the corpus, the
    /// contents of `CONTAINING` octet strings and of open types decoded through their object sets, and for 1988-syntax
    /// modules, which have no object sets, through the corpus's OID hint table (marked `~`, heuristic). Findings:
    /// malformed encodings, invalid contents, DER violations, tag mismatches, missing or unknown components,
    /// constraint violations, unresolved open types and trailing data (`glint explain E400` ... `E409`), and types the
    /// IR could not lower, such as an import nothing answers (`E416`). A TLV that does not match its type is decoded
    /// without a schema, and decoding carries on.
    ///
    /// Exits 1 when the encoding has errors (and with --strict-der, DER violations).
    Decode(decode_cmd::Args),
    /// Triage the RFC Editor's Verified errata of RFCs with ASN.1, and keep the corpus's errata patches
    ///
    /// A Verified erratum that corrects ASN.1, in a module or inline, is recorded as a patch to the RFC's source
    /// (`rfcN.xml` from RFC 8650, `rfcN.txt` before) in `corpus/errata`, which corpus-build applies before extracting.
    /// Typical session: `queue`, then per erratum `show`, `edit` (and edit the work file), `prove`, `record`; finally
    /// `check`.
    Errata(errata_cmd::Args),
    /// Describe a diagnostic code, with examples
    Explain(explain_cmd::Args),
    /// Extract ASN.1 modules from an RFCXML, Markdown or plain-text document
    Extract(extract_cmd::Args),
    /// Print the IR of a type and everything it reaches, as the decoder and code generators see it (development
    /// command)
    ///
    /// Takes a corpus module (`rfc5912#PKIX1Explicit-2009`, or a name only one document defines) or, with FILES, a
    /// module of user ASN.1. The text form lists each type definition, then each object set table, then the OID
    /// constants and anything the lowering could not represent; `--json` prints the IR itself.
    Ir(ir_cmd::Args),
    /// Parse ASN.1 modules and report syntax errors (development command)
    Parse(parse_cmd::Args),
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Artifacts(args) => artifacts_cmd::run(&args),
        Command::Check(args) => check_cmd::run(&args),
        Command::Closure(args) => closure_cmd::run(&args),
        Command::Codegen(args) => codegen_cmd::run(&args),
        Command::Corpus(args) => corpus_cmd::run(&args),
        Command::Decode(args) => decode_cmd::run(&args),
        Command::Errata(args) => errata_cmd::run(&args),
        Command::Explain(args) => explain_cmd::run(&args),
        Command::Extract(args) => extract_cmd::run(&args),
        Command::Ir(args) => ir_cmd::run(&args),
        Command::Parse(args) => parse_cmd::run(&args),
    }
}
