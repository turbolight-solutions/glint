use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args as ClapArgs, ValueEnum};
use glint_check::{CorpusOracle, lower_corpus_root, lower_corpus_type, lower_user_type};
use glint_corpus::{Corpus, CorpusProvider, EmptyCorpus};
use glint_decode::{
    Decoded, Encoding, Input, NoOracle, Oracle, decode, label_roots, read_as, render_text, sniff,
};
use glint_ir::IrSet;
use serde::Serialize;

use crate::common::{EXIT_ERRORS, EXIT_FAILURE, EXIT_OK, display_name, load_corpus, read_input};
use crate::ir_cmd::find_module;

#[derive(Clone, Copy, ValueEnum)]
enum InputFormat {
    Auto,
    Pem,
    Base64,
    Der,
}

#[derive(ClapArgs)]
pub struct Args {
    /// `[MODULE TYPE] [INPUT]`: the module (`rfc5912#PKIX1Explicit-2009`, a corpus module name defined by one document,
    /// or with --file the name of a module of user ASN.1), the root type, and the file to decode (`-` or nothing for
    /// stdin). Without a module and type, a PEM label chooses the type, or the input is decoded without a schema
    #[arg(value_name = "ARGS", num_args = 0..=3)]
    args: Vec<String>,
    /// A file of user ASN.1 to take the module from; repeat for several. Its imports resolve against the corpus
    #[arg(long = "file", value_name = "ASN1")]
    files: Vec<PathBuf>,
    /// How the input is written; `auto` reads PEM, then base64, then DER
    #[arg(long, value_enum, default_value = "auto")]
    input: InputFormat,
    /// Print the TLV trees as JSON instead of text
    #[arg(long)]
    json: bool,
    /// Fail on DER violations (W402) as well as on errors
    #[arg(long)]
    strict_der: bool,
    /// Decode without a schema even when a PEM label suggests one
    #[arg(long, conflicts_with = "files")]
    no_schema: bool,
    /// Corpus data (`corpus.json`) to use instead of the downloaded one
    #[arg(long)]
    corpus: Option<PathBuf>,
}

fn failure(message: &str) -> ExitCode {
    eprintln!("glint: {message}");
    ExitCode::from(EXIT_FAILURE)
}

fn read_bytes(path: &Path) -> std::io::Result<Vec<u8>> {
    if path.as_os_str() == "-" {
        let mut bytes = Vec::new();
        std::io::stdin().read_to_end(&mut bytes)?;
        Ok(bytes)
    } else {
        std::fs::read(path)
    }
}

/// One decoded input, as `--json` prints it.
#[derive(Serialize)]
struct Block<'a> {
    #[serde(flatten)]
    input: &'a Input,
    /// The root type, `doc#Module.Type` or `Module.Type`; absent for a decode without a schema.
    #[serde(skip_serializing_if = "Option::is_none")]
    root: Option<String>,
    #[serde(flatten)]
    decoded: Decoded,
}

/// The schema of a decode, and how to name it.
struct Schema {
    set: IrSet,
    name: String,
}

fn schema_for(
    args: &Args,
    module: &str,
    ty: &str,
    corpus: Option<&Corpus>,
) -> Result<Schema, String> {
    let empty = EmptyCorpus::default();
    let provider: &dyn CorpusProvider = match corpus {
        Some(corpus) => corpus,
        None => &empty,
    };
    if !args.files.is_empty() {
        let mut texts = Vec::new();
        for path in &args.files {
            texts.push(
                read_input(path).map_err(|e| format!("cannot read {}: {e}", display_name(path)))?,
            );
        }
        let set = lower_user_type(&texts, provider, module, ty)?;
        return Ok(Schema {
            set,
            name: format!("{module}.{ty}"),
        });
    }
    let corpus = corpus.ok_or("no corpus data")?;
    let index = find_module(corpus, module)?;
    let set = lower_corpus_type(corpus, index, ty)?;
    let record = corpus.modules().get(index).ok_or("no such module")?;
    Ok(Schema {
        set,
        name: format!("{}#{}.{ty}", record.source.document, record.name),
    })
}

/// The first corpus root of a PEM label that the corpus can lower.
fn schema_for_label(label: &str, corpus: &Corpus) -> Option<Schema> {
    label_roots(label).iter().find_map(|root| {
        lower_corpus_root(corpus, root).ok().map(|set| Schema {
            set,
            name: (*root).to_owned(),
        })
    })
}

pub fn run(args: &Args) -> ExitCode {
    let (schema_args, path) = match args.args.as_slice() {
        [] => (None, PathBuf::from("-")),
        [file] => (None, PathBuf::from(file)),
        [module, ty] => (Some((module.as_str(), ty.as_str())), PathBuf::from("-")),
        [module, ty, file, ..] => (Some((module.as_str(), ty.as_str())), PathBuf::from(file)),
    };
    if schema_args.is_none() && !args.files.is_empty() {
        return failure("--file needs a module and a type");
    }
    let needs_corpus = schema_args.is_some() && args.files.is_empty();
    let corpus = match load_corpus(args.corpus.as_deref()) {
        Ok(corpus) => Some(corpus),
        Err(message) if needs_corpus || args.corpus.is_some() => return failure(&message),
        Err(message) => {
            eprintln!(
                "glint: warning: {message}; OIDs are not named and PEM labels choose no type"
            );
            None
        }
    };
    let bytes = match read_bytes(&path) {
        Ok(bytes) => bytes,
        Err(error) => return failure(&format!("cannot read {}: {error}", display_name(&path))),
    };
    let inputs = match args.input {
        InputFormat::Auto => Ok(sniff(&bytes)),
        InputFormat::Pem => read_as(&bytes, Encoding::Pem),
        InputFormat::Base64 => read_as(&bytes, Encoding::Base64),
        InputFormat::Der => read_as(&bytes, Encoding::Der),
    };
    let inputs = match inputs {
        Ok(inputs) => inputs,
        Err(message) => return failure(&format!("{}: {message}", display_name(&path))),
    };
    let fixed = match schema_args {
        Some((module, ty)) => match schema_for(args, module, ty, corpus.as_ref()) {
            Ok(schema) => Some(schema),
            Err(message) => return failure(&message),
        },
        None => None,
    };
    let oracle: Box<dyn Oracle + '_> = match &corpus {
        Some(corpus) => Box::new(CorpusOracle::new(corpus)),
        None => Box::new(NoOracle),
    };
    let mut blocks = Vec::new();
    for input in &inputs {
        let from_label = match (&fixed, &input.label, &corpus) {
            (None, Some(label), Some(corpus)) if !args.no_schema => schema_for_label(label, corpus),
            _ => None,
        };
        let schema = fixed.as_ref().or(from_label.as_ref());
        let decoded = decode(
            &input.bytes,
            schema.map(|s| (&s.set, s.set.root)),
            oracle.as_ref(),
        );
        blocks.push(Block {
            input,
            root: schema.map(|s| s.name.clone()),
            decoded,
        });
    }
    let failed = blocks.iter().any(|b| b.decoded.failed(args.strict_der));
    let text = if args.json {
        match serde_json::to_string_pretty(&blocks) {
            Ok(json) => json + "\n",
            Err(error) => return failure(&format!("cannot write JSON: {error}")),
        }
    } else {
        let mut out = String::new();
        let several = blocks.len() > 1;
        for (index, block) in blocks.iter().enumerate() {
            if several || block.input.label.is_some() || block.root.is_some() {
                let mut header = Vec::new();
                if several {
                    header.push(format!(
                        "block {} of {}",
                        index.saturating_add(1),
                        blocks.len()
                    ));
                }
                if let Some(label) = &block.input.label {
                    header.push(label.clone());
                }
                header.push(match &block.root {
                    Some(root) => format!("as {root}"),
                    None => "without a schema".to_owned(),
                });
                out.push_str(&format!("# {}\n", header.join(", ")));
            }
            out.push_str(&render_text(&block.decoded));
            if several && index.saturating_add(1) < blocks.len() {
                out.push('\n');
            }
        }
        out
    };
    let mut stdout = std::io::stdout().lock();
    if stdout.write_all(text.as_bytes()).is_err() {
        return ExitCode::from(EXIT_FAILURE);
    }
    ExitCode::from(if failed { EXIT_ERRORS } else { EXIT_OK })
}
