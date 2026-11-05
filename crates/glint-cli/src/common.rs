//! Helpers shared by the subcommands.

use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};

use clap::ValueEnum;
use glint_corpus::{CORPUS_FILE, Corpus, CorpusData, SCHEMA, build_date};

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

/// Data older than this many days gets a warning.
const STALE_DAYS: u64 = 30;

/// Where `glint corpus update` keeps its files.
pub fn cache_dir() -> Option<PathBuf> {
    std::env::var_os("GLINT_CACHE_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            use etcetera::BaseStrategy;
            etcetera::choose_base_strategy()
                .ok()
                .map(|strategy| strategy.cache_dir().join("glint"))
        })
}

/// Where `glint corpus update` stores the downloaded `corpus.json`.
pub fn cached_corpus_path() -> Option<PathBuf> {
    cache_dir().map(|dir| dir.join(CORPUS_FILE))
}

/// Loads the corpus data: `--corpus`, then `$GLINT_CORPUS`, then the data `glint corpus update` downloaded. Glint
/// carries no data of its own, so with none of these it fails and says how to get some.
pub fn load_corpus(explicit: Option<&Path>) -> Result<Corpus, String> {
    let from_env = std::env::var_os("GLINT_CORPUS").map(PathBuf::from);
    let (path, downloaded) = match explicit.map(Path::to_path_buf).or(from_env) {
        Some(path) => (path, false),
        None => match cached_corpus_path().filter(|p| p.is_file()) {
            Some(path) => (path, true),
            None => {
                return Err(
                    "no corpus data; run `glint corpus update` to download it, or pass --corpus <corpus.json>"
                        .to_owned(),
                );
            }
        },
    };
    let json =
        std::fs::read(&path).map_err(|e| format!("cannot read corpus {}: {e}", path.display()))?;
    let data = CorpusData::from_json(&json).map_err(|e| {
        let hint = if downloaded {
            "; run `glint corpus update`"
        } else {
            ""
        };
        format!("{}: {e}{hint}", path.display())
    })?;
    warn_if_stale(&data.info.build);
    Ok(Corpus::from_data(data))
}

fn warn_if_stale(build: &str) {
    let Some(built) = days_since_epoch(build_date(build)) else {
        return;
    };
    let today = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() / 86_400);
    let age = today.saturating_sub(built);
    if age > STALE_DAYS {
        eprintln!(
            "glint: warning: the corpus data is {age} days old (build {build}); run `glint corpus update`. \
             If that finds nothing newer, upgrade Glint: this version reads data schema {SCHEMA}."
        );
    }
}

/// Days from 1970-01-01 to the date `YYYY-MM-DD` (H. Hinnant, "chrono-Compatible Low-Level Date Algorithms").
fn days_since_epoch(date: &str) -> Option<u64> {
    let mut parts = date.splitn(3, '-').map(str::parse::<u64>);
    let (year, month, day) = (
        parts.next()?.ok()?,
        parts.next()?.ok()?,
        parts.next()?.ok()?,
    );
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || year < 1970 {
        return None;
    }
    let year = year.saturating_sub(u64::from(month <= 2));
    let era = year / 400;
    let yoe = year % 400;
    let mp = if month > 2 {
        month.saturating_sub(3)
    } else {
        month.saturating_add(9)
    };
    let doy = mp
        .saturating_mul(153)
        .saturating_add(2)
        .checked_div(5)?
        .saturating_add(day)
        .saturating_sub(1);
    let doe = yoe
        .saturating_mul(365)
        .saturating_add(yoe / 4)
        .saturating_sub(yoe / 100)
        .saturating_add(doy);
    era.saturating_mul(146_097)
        .saturating_add(doe)
        .checked_sub(719_468)
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

#[cfg(test)]
mod tests {
    use super::days_since_epoch;

    #[test]
    fn dates_count_from_the_epoch() {
        assert_eq!(days_since_epoch("1970-01-01"), Some(0));
        assert_eq!(days_since_epoch("2000-02-29"), Some(11_016));
        assert_eq!(days_since_epoch("2026-11-05"), Some(20_762));
        assert_eq!(days_since_epoch("2026-13-05"), None);
    }
}
