use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args as ClapArgs, Subcommand};
use glint_corpus::{
    CORPUS_FILE, Corpus, CorpusData, CorpusProvider, ModuleRecord, Oid, ParseStatus, Pointer,
    build_dir, latest_path, valid_build_id,
};
use glint_diag::Versions;

use crate::common::{
    EXIT_ERRORS, EXIT_FAILURE, EXIT_OK, cache_dir, cached_corpus_path, load_corpus,
};
use crate::net::fetch_data;

/// The Glint site, which publishes the data under `data/v<schema>/`.
const DEFAULT_SITE: &str = "https://glint.turbolightsolutions.com";

#[derive(ClapArgs)]
pub struct Args {
    /// Corpus data (`corpus.json`) to use instead of the downloaded one
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
    /// Download the corpus data to the user cache
    Update {
        /// The site to download from; the data is read from `<URL>/data/v<schema>/`
        #[arg(long, default_value = DEFAULT_SITE)]
        url: String,
        /// Download this build (`YYYY-MM-DD.<hash>`) instead of the latest
        #[arg(long, value_name = "ID", conflicts_with = "from")]
        build: Option<String>,
        /// Install a `corpus.json` file instead of downloading one
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
    if let Sub::Update { url, build, from } = &args.command {
        return match update(url, build.as_deref(), from.as_deref()) {
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
    let mut out = std::io::stdout().lock();
    let code = match &args.command {
        Sub::List { filter } => list(&corpus, filter.as_deref(), &mut out),
        Sub::Show { key } => show(&corpus, key, &mut out),
        Sub::Oid { oid } => oid_info(&corpus, oid, &mut out),
        Sub::Update { .. } => EXIT_OK,
    };
    let versions = Versions {
        glint: env!("CARGO_PKG_VERSION"),
        data: Some(&corpus.info().build),
    };
    eprintln!("{}", versions.line());
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
        for evidence in corpus.oid_labels().labels_of(&current) {
            let documents: Vec<String> = evidence
                .documents
                .iter()
                .take(3)
                .map(|d| glint_corpus::document_title(d))
                .collect();
            let more = evidence.documents.len().saturating_sub(3);
            let more = if more > 0 {
                format!(" and {more} more")
            } else {
                String::new()
            };
            let _ = writeln!(
                out,
                "  {current}  label {}({})  ({}{more})",
                evidence.label,
                current.last().unwrap_or_default(),
                documents.join(", ")
            );
        }
        if let Some(allowed) = corpus.oid_labels().allowed(&current)
            && allowed.oid == current
        {
            let _ = writeln!(out, "  {current}  allowlisted: {}", allowed.reason);
        }
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

fn update(site: &str, build: Option<&str>, from: Option<&Path>) -> Result<String, String> {
    let destination = cached_corpus_path()
        .ok_or_else(|| "cannot tell where the user cache is; set GLINT_CACHE_DIR".to_owned())?;
    if let Some(path) = from {
        let bytes =
            std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        return install(&bytes, None, None, &destination);
    }
    let site = site.trim_end_matches('/');
    let build = match build {
        Some(build) if valid_build_id(build) => build.to_owned(),
        Some(build) => {
            return Err(format!(
                "`{build}` is not a build id; it looks like `2026-11-05.0123abcd`"
            ));
        }
        None => {
            let url = format!("{site}/{}", latest_path());
            let bytes = fetch_data(&url)?.ok_or_else(|| {
                format!("{url}: not found; the site publishes no data for this version of Glint")
            })?;
            Pointer::from_json(&bytes)
                .map_err(|e| format!("{url}: {e}"))?
                .build
        }
    };
    if installed_build(&destination).as_deref() == Some(build.as_str()) {
        return Ok(format!(
            "the corpus data is up to date (build {build}) at {}",
            destination.display()
        ));
    }
    let url = format!("{site}/{}/{CORPUS_FILE}", build_dir(&build));
    let bytes = fetch_data(&url)?.ok_or_else(|| format!("{url}: not found"))?;
    let digest = fetch_data(&format!("{url}.sha256"))?
        .ok_or_else(|| format!("{url}.sha256: not found; the download cannot be verified"))?;
    install(
        &bytes,
        Some(&String::from_utf8_lossy(&digest)),
        Some(&build),
        &destination,
    )
}

/// The build of the data at `path`, if it is readable data of this schema.
fn installed_build(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    CorpusData::from_json(&bytes)
        .ok()
        .map(|data| data.info.build)
}

/// Checks the digest (when one is given), that the bytes are data of this schema and of the expected build, then
/// replaces `destination`.
fn install(
    bytes: &[u8],
    expected_sha256: Option<&str>,
    expected_build: Option<&str>,
    destination: &Path,
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
    let data = CorpusData::from_json(bytes)
        .map_err(|e| format!("the download is not usable corpus data: {e}"))?;
    if let Some(build) = expected_build
        && data.info.build != build
    {
        return Err(format!(
            "the download is build {}, not the build {build} that was asked for",
            data.info.build
        ));
    }
    let parent = destination
        .parent()
        .ok_or_else(|| "the cache path has no directory".to_owned())?;
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    let temporary = destination.with_extension("json.part");
    std::fs::write(&temporary, bytes)
        .and_then(|()| std::fs::rename(&temporary, destination))
        .map_err(|e| format!("cannot write {}: {e}", destination.display()))?;
    // The cache file of Glint versions that embedded a corpus.
    if let Some(old) = cache_dir().map(|dir| dir.join("corpus.bundle")) {
        let _ = std::fs::remove_file(old);
    }
    Ok(format!(
        "installed corpus data build {} ({} modules) at {}",
        data.info.build,
        data.modules.len(),
        destination.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data_bytes(build: &str) -> Vec<u8> {
        let mut data = CorpusData::default();
        data.info.build = build.to_owned();
        data.to_json()
    }

    #[test]
    fn a_matching_digest_installs_the_data() {
        use sha2::{Digest, Sha256};
        let bytes = data_bytes("2026-11-05.0123abcd");
        let digest: String = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let dir = std::env::temp_dir().join(format!("glint-install-ok-{}", std::process::id()));
        let destination = dir.join("corpus.json");
        let message = install(
            &bytes,
            Some(&format!("{digest}  corpus.json\n")),
            Some("2026-11-05.0123abcd"),
            &destination,
        )
        .unwrap();
        assert!(message.contains("0 modules"), "{message}");
        assert_eq!(std::fs::read(&destination).unwrap(), bytes);
        assert_eq!(
            installed_build(&destination).as_deref(),
            Some("2026-11-05.0123abcd")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_wrong_digest_build_or_format_is_refused_and_installs_nothing() {
        let dir = std::env::temp_dir().join(format!("glint-install-bad-{}", std::process::id()));
        let destination = dir.join("corpus.json");
        let bytes = data_bytes("2026-11-05.0123abcd");
        assert!(install(&bytes, Some("00ff"), None, &destination).is_err());
        assert!(install(&bytes, None, Some("2026-11-06.0123abcd"), &destination).is_err());
        assert!(install(b"not data", None, None, &destination).is_err());
        let other_schema =
            String::from_utf8(bytes)
                .unwrap()
                .replacen("\"schema\":1", "\"schema\":99", 1);
        assert!(install(other_schema.as_bytes(), None, None, &destination).is_err());
        assert!(!destination.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
