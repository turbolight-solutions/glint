use std::collections::HashMap;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Args as ClapArgs;
use glint_corpus::{Corpus, CorpusProvider, EmptyCorpus};
use glint_sema::{DraftSource, Options, analyze};
use glint_syntax::{ParseOptions, parse_source_with};

use crate::closure_cmd::{corpus_module, parse_pins};
use crate::common::{EXIT_FAILURE, EXIT_OK, display_name, load_corpus, read_input};

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
    /// Print the IR as JSON instead of text
    #[arg(long)]
    json: bool,
    /// Corpus data (`corpus.json`) to use instead of the downloaded one
    #[arg(long)]
    corpus: Option<PathBuf>,
    /// Resolve imports of `Name` to the module published in `rfcNNNN`, or to the module with that OID
    #[arg(long = "pin", value_name = "NAME=rfcNNNN|OID")]
    pins: Vec<String>,
}

fn failure(message: &str) -> ExitCode {
    eprintln!("glint: {message}");
    ExitCode::from(EXIT_FAILURE)
}

/// The corpus module `reference` names: `DOC#NAME[@OID]`, or a bare name only one document defines.
pub(crate) fn find_module(corpus: &Corpus, reference: &str) -> Result<usize, String> {
    if reference.contains('#') {
        return corpus_module(corpus, reference).map_err(|e| format!("{reference}: {e}"));
    }
    let matches = corpus.by_name(reference);
    match matches {
        [index] => Ok(*index),
        [] => Err(format!("{reference}: not in the corpus")),
        several => {
            let documents: Vec<String> = several
                .iter()
                .filter_map(|&i| corpus.modules().get(i))
                .map(|m| format!("{}#{reference}", m.source.document))
                .collect();
            Err(format!(
                "{reference}: several documents define it; name one: {}",
                documents.join(", ")
            ))
        }
    }
}

/// Where a root type comes from: a corpus module, or a module of user ASN.1 files.
pub(crate) struct RootSource<'a> {
    pub module: &'a str,
    pub ty: &'a str,
    pub files: &'a [PathBuf],
    pub corpus: Option<&'a std::path::Path>,
    pub pins: &'a [String],
}

/// The IR of a root type, and the corpus data build its imports resolved against.
pub(crate) fn lower_root(
    source: &RootSource<'_>,
) -> Result<(glint_ir::IrSet, Option<String>), String> {
    let pins = parse_pins(source.pins)?;
    let mut options = Options {
        pins: pins.into_iter().collect::<HashMap<_, _>>(),
        oid_collisions: false,
        oid_labels: false,
        policy_checks: false,
        resolution_notes: false,
        ..Options::default()
    };
    // user ASN.1 can do without a corpus; a corpus module cannot
    let corpus = match load_corpus(source.corpus) {
        Ok(corpus) => Some(corpus),
        Err(message) if !source.files.is_empty() && source.corpus.is_none() => {
            eprintln!("glint: warning: {message}; imports from the corpus stay unresolved");
            None
        }
        Err(message) => return Err(message),
    };
    let mut texts = Vec::new();
    let mut parse_options = ParseOptions::default();
    let module_name = if source.files.is_empty() {
        let corpus = corpus.as_ref().ok_or("no corpus data")?;
        let record = find_module(corpus, source.module).and_then(|i| {
            corpus
                .modules()
                .get(i)
                .ok_or_else(|| "no such module".to_owned())
        })?;
        options.home_document = Some(record.source.document.clone());
        parse_options = ParseOptions::corpus();
        texts.push(record.text.clone());
        record.name.clone()
    } else {
        for path in source.files {
            let text = read_input(path)
                .map_err(|error| format!("cannot read {}: {error}", display_name(path)))?;
            texts.push(text);
        }
        source.module.to_owned()
    };
    let sources = texts
        .iter()
        .map(|text| DraftSource {
            parse: parse_source_with(text, parse_options),
            text: text.clone(),
        })
        .collect();
    let empty = EmptyCorpus::default();
    let provider: &dyn CorpusProvider = match &corpus {
        Some(corpus) => corpus,
        None => &empty,
    };
    let analysis = analyze(sources, provider, &options);
    let module = analysis
        .drafts
        .iter()
        .copied()
        .find(|&id| {
            analysis
                .graph
                .module(id)
                .is_some_and(|m| m.model.name == module_name)
        })
        .ok_or_else(|| format!("no module `{module_name}` in the given files"))?;
    let set = glint_sema::ir::lower(&analysis.graph, module, source.ty)
        .ok_or_else(|| format!("`{module_name}` defines no type `{}`", source.ty))?;
    let build = corpus
        .as_ref()
        .map(|c| c.info().build.clone())
        .filter(|b| !b.is_empty());
    Ok((set, build))
}

pub fn run(args: &Args) -> ExitCode {
    let source = RootSource {
        module: &args.module,
        ty: &args.ty,
        files: &args.files,
        corpus: args.corpus.as_deref(),
        pins: &args.pins,
    };
    let set = match lower_root(&source) {
        Ok((set, _)) => set,
        Err(message) => return failure(&message),
    };
    let text = if args.json {
        match serde_json::to_string_pretty(&set) {
            Ok(json) => json + "\n",
            Err(error) => return failure(&format!("cannot write JSON: {error}")),
        }
    } else {
        glint_ir::dump(&set)
    };
    let mut stdout = std::io::stdout().lock();
    if stdout.write_all(text.as_bytes()).is_err() {
        return ExitCode::from(EXIT_FAILURE);
    }
    ExitCode::from(EXIT_OK)
}
