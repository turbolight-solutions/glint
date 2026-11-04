use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args as ClapArgs, Subcommand};
use glint_corpus::{Corpus, CorpusProvider, ModuleRecord, Oid, ParseStatus};

use crate::common::{EXIT_ERRORS, EXIT_FAILURE, EXIT_OK, cached_bundle_path, load_corpus};
use crate::net::{fetch_bundle, fetch_text};

const DEFAULT_BUNDLE_URL: &str =
    "https://github.com/glint-asn1/glint/releases/latest/download/corpus.bundle";

#[derive(ClapArgs)]
pub struct Args {
    /// Corpus bundle to use instead of the embedded one
    #[arg(long, global = true)]
    corpus: Option<PathBuf>,
    #[command(subcommand)]
    command: Sub,
}

#[derive(Subcommand)]
enum Sub {
    /// List modules with their RFC, OID and parse status
    List {
        /// Only modules whose name, OID or RFC contains this text
        #[arg(long)]
        filter: Option<String>,
    },
    /// Print a module's text and metadata
    Show {
        /// A module name, a module OID or `rfcNNNN`
        key: String,
    },
    /// Show who assigns an OID, and its parent arcs
    Oid {
        /// Dotted object identifier
        oid: String,
    },
    /// Download the latest published bundle to the user cache; it replaces the embedded one
    Update {
        /// Where to download from (the bundle's `.sha256` file is expected next to it)
        #[arg(long, default_value = DEFAULT_BUNDLE_URL)]
        url: String,
        /// Install a bundle file instead of downloading one
        #[arg(long, conflicts_with = "url")]
        from: Option<PathBuf>,
    },
}

fn status_name(status: ParseStatus) -> &'static str {
    match status {
        ParseStatus::Clean => "clean",
        ParseStatus::Warnings => "warnings",
        ParseStatus::Errors => "errors",
        ParseStatus::Unparseable => "unparseable",
    }
}

pub fn run(args: &Args) -> ExitCode {
    if let Sub::Update { url, from } = &args.command {
        return match update(url, from.as_deref()) {
            Ok(message) => {
                println!("{message}");
                ExitCode::from(EXIT_OK)
            }
            Err(message) => {
                eprintln!("glint: {message}");
                ExitCode::from(EXIT_FAILURE)
            }
        };
    }
    let corpus = match load_corpus(args.corpus.as_deref()) {
        Ok(corpus) => corpus,
        Err(message) => {
            eprintln!("glint: {message}");
            return ExitCode::from(EXIT_FAILURE);
        }
    };
    if corpus.modules().is_empty() {
        eprintln!("glint: no corpus is available; pass --corpus <bundle> or set GLINT_CORPUS");
        return ExitCode::from(EXIT_FAILURE);
    }
    let mut out = std::io::stdout().lock();
    let code = match &args.command {
        Sub::List { filter } => list(&corpus, filter.as_deref(), &mut out),
        Sub::Show { key } => show(&corpus, key, &mut out),
        Sub::Oid { oid } => oid_info(&corpus, oid, &mut out),
        Sub::Update { .. } => EXIT_OK,
    };
    ExitCode::from(code)
}

fn list(corpus: &Corpus, filter: Option<&str>, out: &mut impl Write) -> u8 {
    for module in corpus.modules() {
        let line = format!(
            "{:<8} {:<42} {:<34} {}",
            module.source.label(),
            module.name,
            module
                .oid
                .as_ref()
                .map_or_else(String::new, ToString::to_string),
            status_name(module.parse_status)
        );
        if filter.is_none_or(|f| line.to_lowercase().contains(&f.to_lowercase())) {
            let _ = writeln!(out, "{line}");
        }
    }
    EXIT_OK
}

fn find(corpus: &Corpus, key: &str) -> Vec<usize> {
    if let Ok(oid) = key.parse::<Oid>() {
        return corpus.by_oid(&oid).into_iter().collect();
    }
    let by_document: Vec<usize> = corpus
        .modules()
        .iter()
        .enumerate()
        .filter(|(_, m)| m.source.document.eq_ignore_ascii_case(key))
        .map(|(i, _)| i)
        .collect();
    if by_document.is_empty() {
        corpus.by_name(key).to_vec()
    } else {
        by_document
    }
}

fn describe(module: &ModuleRecord) -> String {
    let mut text = format!(
        "-- module:  {}\n-- source:  {}",
        module.name,
        module.source.label()
    );
    if let Some(section) = &module.source.section {
        text.push_str(&format!(" {section}"));
    }
    if let Some(oid) = &module.oid {
        text.push_str(&format!("\n-- oid:     {oid}"));
    }
    if let Some(status) = &module.rfc_status {
        text.push_str(&format!("\n-- status:  {status}"));
    }
    if !module.obsoleted_by.is_empty() {
        let list: Vec<String> = module
            .obsoleted_by
            .iter()
            .map(|n| format!("RFC {n}"))
            .collect();
        text.push_str(&format!("\n-- obsoleted by: {}", list.join(", ")));
    }
    text.push_str(&format!(
        "\n-- parse:   {}\n-- sha256:  {}",
        status_name(module.parse_status),
        module.sha256
    ));
    text
}

fn show(corpus: &Corpus, key: &str, out: &mut impl Write) -> u8 {
    let found = find(corpus, key);
    if found.is_empty() {
        eprintln!("glint: nothing in the corpus matches `{key}`");
        return EXIT_ERRORS;
    }
    if found.len() > 1 && !key.starts_with("rfc") {
        eprintln!(
            "glint: `{key}` matches {} modules; choose one with its OID or `rfcNNNN`:",
            found.len()
        );
        for index in &found {
            if let Some(m) = corpus.modules().get(*index) {
                eprintln!(
                    "  {} {}",
                    m.source.label(),
                    m.oid.as_ref().map_or_else(String::new, ToString::to_string)
                );
            }
        }
    }
    for index in found {
        if let Some(module) = corpus.modules().get(index) {
            let _ = writeln!(out, "{}\n\n{}\n", describe(module), module.text);
        }
    }
    EXIT_OK
}

fn oid_info(corpus: &Corpus, text: &str, out: &mut impl Write) -> u8 {
    let Ok(oid) = text.parse::<Oid>() else {
        eprintln!("glint: `{text}` is not a dotted object identifier");
        return EXIT_FAILURE;
    };
    let _ = writeln!(out, "{oid}");
    let mut walk = Some(oid.clone());
    while let Some(current) = walk {
        for a in corpus.assignments_of(&current) {
            let owner = glint_corpus::document_title(&a.document);
            let _ = writeln!(
                out,
                "  {current}  {}  ({owner}, module {})",
                a.name, a.module
            );
        }
        if let Some(index) = corpus.by_oid(&current)
            && let Some(module) = corpus.modules().get(index)
        {
            let _ = writeln!(
                out,
                "  {current}  module {} ({})",
                module.name,
                module.source.label()
            );
        }
        walk = current.parent();
    }
    let below = corpus.assignments_under(&oid);
    let children: Vec<_> = below.iter().filter(|a| a.oid != oid).collect();
    if !children.is_empty() {
        let _ = writeln!(out, "assigned below:");
        for a in children.iter().take(50) {
            let _ = writeln!(out, "  {}  {}  ({})", a.oid, a.name, a.module);
        }
        if children.len() > 50 {
            let _ = writeln!(out, "  ... and {} more", children.len().saturating_sub(50));
        }
    }
    EXIT_OK
}

fn update(url: &str, from: Option<&std::path::Path>) -> Result<String, String> {
    let destination = cached_bundle_path()
        .ok_or_else(|| "cannot tell where the user cache is; set GLINT_CACHE_DIR".to_owned())?;
    let (bytes, expected) = match from {
        Some(path) => (
            std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?,
            None,
        ),
        None => {
            let bytes = fetch_bundle(url)?;
            let expected = fetch_text(&format!("{url}.sha256"))?;
            (bytes, expected)
        }
    };
    install_bundle(&bytes, expected.as_deref(), &destination)
}

/// Checks the digest (when one is given) and that the bytes are a readable bundle, then replaces `destination`.
fn install_bundle(
    bytes: &[u8],
    expected_sha256: Option<&str>,
    destination: &std::path::Path,
) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    if let Some(expected) = expected_sha256 {
        let wanted = expected.split_whitespace().next().unwrap_or_default();
        let actual: String = Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if !wanted.eq_ignore_ascii_case(&actual) {
            return Err(format!(
                "the download does not match its published digest (expected {wanted}, got {actual})"
            ));
        }
    }
    let mut reader = bytes;
    let bundle = glint_corpus::read_bundle(&mut reader)
        .map_err(|e| format!("the download is not a usable corpus bundle: {e}"))?;
    let parent = destination
        .parent()
        .ok_or_else(|| "the cache path has no directory".to_owned())?;
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    let temporary = destination.with_extension("bundle.part");
    std::fs::write(&temporary, bytes)
        .and_then(|()| std::fs::rename(&temporary, destination))
        .map_err(|e| format!("cannot write {}: {e}", destination.display()))?;
    Ok(format!(
        "installed corpus {} ({} modules) at {}",
        bundle.info.version,
        bundle.modules.len(),
        destination.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle_bytes() -> Vec<u8> {
        let mut bytes = Vec::new();
        glint_corpus::write_bundle(&glint_corpus::Bundle::default(), &mut bytes).unwrap();
        bytes
    }

    #[test]
    fn a_matching_digest_installs_the_bundle() {
        use sha2::{Digest, Sha256};
        let bytes = bundle_bytes();
        let digest: String = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let dir = std::env::temp_dir().join(format!("glint-install-ok-{}", std::process::id()));
        let destination = dir.join("corpus.bundle");
        let message = install_bundle(
            &bytes,
            Some(&format!("{digest}  corpus.bundle\n")),
            &destination,
        )
        .unwrap();
        assert!(message.contains("0 modules"), "{message}");
        assert_eq!(std::fs::read(&destination).unwrap(), bytes);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_wrong_digest_or_a_non_bundle_is_refused_and_installs_nothing() {
        let dir = std::env::temp_dir().join(format!("glint-install-bad-{}", std::process::id()));
        let destination = dir.join("corpus.bundle");
        assert!(install_bundle(&bundle_bytes(), Some("00ff"), &destination).is_err());
        assert!(install_bundle(b"not a bundle", None, &destination).is_err());
        assert!(!destination.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
