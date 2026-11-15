use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Args as ClapArgs;
use glint_codegen::{Options, backend, backends, generate};
use glint_diag::Severity;

use crate::common::{EXIT_FAILURE, EXIT_OK};
use crate::ir_cmd::{RootSource, lower_root};

#[derive(ClapArgs)]
pub struct Args {
    /// The module: `rfc5912#PKIX1Explicit-2009`, a corpus module name defined by one document, or the name of a module
    /// in FILES
    module: String,
    /// The root type
    #[arg(value_name = "TYPE")]
    ty: String,
    /// Files of user ASN.1 to take the module from; their imports resolve against the corpus when there is one
    files: Vec<PathBuf>,
    /// The target library
    #[arg(long, value_parser = backend_ids())]
    backend: String,
    /// The directory to write the files to; without it they go to stdout, each after a `// ---- FILE ----` line
    #[arg(long, short)]
    out: Option<PathBuf>,
    /// Corpus data (`corpus.json`) to use instead of the downloaded one
    #[arg(long)]
    corpus: Option<PathBuf>,
    /// Resolve imports of `Name` to the module published in `rfcNNNN`, or to the module with that OID
    #[arg(long = "pin", value_name = "NAME=rfcNNNN|OID")]
    pins: Vec<String>,
}

fn backend_ids() -> clap::builder::PossibleValuesParser {
    clap::builder::PossibleValuesParser::new(backends().iter().map(|b| b.meta().id))
}

fn failure(message: &str) -> ExitCode {
    eprintln!("glint: {message}");
    ExitCode::from(EXIT_FAILURE)
}

pub fn run(args: &Args) -> ExitCode {
    let Some(backend) = backend(&args.backend) else {
        return failure(&format!("no backend `{}`", args.backend));
    };
    let source = RootSource {
        module: &args.module,
        ty: &args.ty,
        files: &args.files,
        corpus: args.corpus.as_deref(),
        pins: &args.pins,
    };
    let (set, data_build) = match lower_root(&source) {
        Ok(lowered) => lowered,
        Err(message) => return failure(&message),
    };
    let options = Options {
        glint_version: env!("CARGO_PKG_VERSION").to_owned(),
        data_build,
    };
    let output = generate(&set, backend, &options);
    for finding in &output.findings {
        let severity = match finding.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
        };
        eprintln!(
            "glint: {} {severity}: {}: {}",
            finding.code, finding.at, finding.message
        );
    }
    match &args.out {
        Some(dir) => {
            if let Err(error) = std::fs::create_dir_all(dir) {
                return failure(&format!("cannot create {}: {error}", dir.display()));
            }
            for file in &output.files {
                let path = dir.join(&file.path);
                if let Err(error) = std::fs::write(&path, &file.contents) {
                    return failure(&format!("cannot write {}: {error}", path.display()));
                }
            }
            eprintln!(
                "glint: wrote {} files for {} {} to {}",
                output.files.len(),
                backend.meta().library,
                backend.meta().library_version,
                dir.display()
            );
        }
        None => {
            let mut stdout = std::io::stdout().lock();
            for file in &output.files {
                if writeln!(stdout, "// ---- {} ----\n{}", file.path, file.contents).is_err() {
                    return ExitCode::from(EXIT_FAILURE);
                }
            }
        }
    }
    ExitCode::from(EXIT_OK)
}
