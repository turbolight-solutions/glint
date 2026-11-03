//! Helpers shared by the subcommands.

use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};

use clap::ValueEnum;
use glint_corpus::{Corpus, read_bundle};

pub const EXIT_OK: u8 = 0;
pub const EXIT_ERRORS: u8 = 1;
pub const EXIT_FAILURE: u8 = 2;

#[derive(Clone, Copy, ValueEnum)]
pub enum Color {
    Auto,
    Always,
    Never,
}

impl Color {
    pub fn enabled_for_stderr(self) -> bool {
        match self {
            Self::Always => true,
            Self::Never => false,
            Self::Auto => std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none(),
        }
    }
}

/// Reads a document. Bytes that are not UTF-8 (old RFCs use ISO-8859-1) are replaced rather than rejected.
pub fn read_input(path: &Path) -> std::io::Result<String> {
    let mut bytes = Vec::new();
    if path.as_os_str() == "-" {
        std::io::stdin().read_to_end(&mut bytes)?;
    } else {
        bytes = std::fs::read(path)?;
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

pub fn display_name(path: &Path) -> String {
    if path.as_os_str() == "-" {
        "<stdin>".to_owned()
    } else {
        path.display().to_string()
    }
}

#[cfg(embedded_corpus)]
static EMBEDDED: Option<&[u8]> = Some(include_bytes!(env!("GLINT_EMBEDDED_CORPUS_PATH")));
#[cfg(not(embedded_corpus))]
static EMBEDDED: Option<&[u8]> = None;

/// Where `glint corpus update` stores the downloaded bundle.
pub fn cached_bundle_path() -> Option<PathBuf> {
    let base = std::env::var_os("GLINT_CACHE_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            use etcetera::BaseStrategy;
            etcetera::choose_base_strategy()
                .ok()
                .map(|strategy| strategy.cache_dir().join("glint"))
        })?;
    Some(base.join("corpus.bundle"))
}

/// Loads the corpus: `--corpus`, then `$GLINT_CORPUS`, then the bundle `glint corpus update` downloaded, then the
/// bundle embedded at build time. A downloaded bundle that cannot be read (a newer format, say) is skipped with a note.
pub fn load_corpus(explicit: Option<&Path>) -> Result<Corpus, String> {
    let from_env = std::env::var_os("GLINT_CORPUS").map(PathBuf::from);
    if let Some(path) = explicit.map(Path::to_path_buf).or(from_env) {
        let mut file = std::fs::File::open(&path)
            .map_err(|e| format!("cannot open corpus {}: {e}", path.display()))?;
        let bundle = read_bundle(&mut file).map_err(|e| format!("{}: {e}", path.display()))?;
        return Ok(Corpus::from_bundle(bundle));
    }
    if let Some(path) = cached_bundle_path().filter(|p| p.is_file()) {
        let loaded = std::fs::File::open(&path)
            .map_err(|e| e.to_string())
            .and_then(|mut file| read_bundle(&mut file).map_err(|e| e.to_string()));
        match loaded {
            Ok(bundle) => return Ok(Corpus::from_bundle(bundle)),
            Err(message) => eprintln!(
                "glint: ignoring the downloaded corpus {} ({message}); run `glint corpus update`",
                path.display()
            ),
        }
    }
    match EMBEDDED {
        Some(mut bytes) => {
            let bundle = read_bundle(&mut bytes).map_err(|e| format!("embedded corpus: {e}"))?;
            Ok(Corpus::from_bundle(bundle))
        }
        None => Ok(Corpus::default()),
    }
}

/// The nearest `glint.toml` in `start` or one of its ancestors.
pub fn find_config(start: &Path) -> Option<PathBuf> {
    let absolute = if start.as_os_str() == "-" {
        std::env::current_dir().ok()?
    } else {
        start.canonicalize().ok()?
    };
    let mut dir = if absolute.is_dir() {
        Some(absolute.as_path())
    } else {
        absolute.parent()
    };
    while let Some(current) = dir {
        let candidate = current.join("glint.toml");
        if candidate.is_file() {
            return Some(candidate);
        }
        dir = current.parent();
    }
    None
}
