//! `corpus-build`: builds the Glint data files (`corpus.json`, `documents.bin`, `latest.json`) and the report from a
//! local RFC mirror.

mod build;
mod collect;
mod index;
mod overrides;
mod report;
mod sources;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;

#[derive(Parser)]
#[command(
    name = "corpus-build",
    version,
    about = "Builds the Glint ASN.1 module corpus"
)]
struct Args {
    /// Directory with `rfcNNNN.txt`, `rfcNNNN.xml` and `rfc-index.xml` (see `corpus/fetch.sh`)
    #[arg(long, value_name = "DIR")]
    rfc_dir: PathBuf,
    /// Directory with the active Internet-Drafts, `draft-*.xml`, `draft-*.txt` and `all_id2.txt` (see
    /// `corpus/fetch.sh`)
    #[arg(long, value_name = "DIR")]
    drafts_dir: Option<PathBuf>,
    /// Directory of curated overrides: `rfcNNNN/<Module>.asn` and `rfcNNNN/meta.toml`
    #[arg(long, value_name = "DIR")]
    overrides: Option<PathBuf>,
    /// Where to write `data/v<schema>/<build>/`, `data/v<schema>/latest.json` and `report.md`
    #[arg(long, value_name = "DIR")]
    out: PathBuf,
    /// Build date, `YYYY-MM-DD`, for the build id; today in UTC by default
    #[arg(long)]
    date: Option<String>,
    /// Only process these RFC numbers (for quick runs)
    #[arg(long = "only", value_name = "RFC")]
    only: Vec<u32>,
    /// Also write every finding, one per line, to this file (for triage)
    #[arg(long, value_name = "FILE")]
    dump_findings: Option<PathBuf>,
    /// TOML manifest of the standalone modules of other standards bodies (ITU-T, ETSI, ...) to bundle
    #[arg(long, value_name = "FILE")]
    sources: Option<PathBuf>,
    /// Where downloaded standalone modules are cached
    #[arg(long, value_name = "DIR", default_value = "corpus/cache/files")]
    cache: PathBuf,
    /// RFC Editor `errata.json`, used to check `errata-verified` overrides
    #[arg(long, value_name = "FILE")]
    errata: Option<PathBuf>,
    /// Number of worker threads; defaults to the number of CPUs
    #[arg(long)]
    jobs: Option<usize>,
}

fn main() -> ExitCode {
    let args = Args::parse();
    if let Some(jobs) = args.jobs {
        let _ = rayon::ThreadPoolBuilder::new()
            .num_threads(jobs)
            .build_global();
    }
    // The only place that reads the clock; everything downstream reads the build id from the data.
    let date = args.date.clone().unwrap_or_else(utc_today);
    match build::run(&Settings::new(args, date)) {
        Ok(summary) => {
            println!("{summary}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("corpus-build: {error}");
            ExitCode::from(2)
        }
    }
}

pub struct Settings {
    pub rfc_dir: PathBuf,
    pub drafts_dir: Option<PathBuf>,
    pub overrides: Option<PathBuf>,
    pub out: PathBuf,
    pub date: String,
    pub only: Vec<u32>,
    pub dump_findings: Option<PathBuf>,
    pub sources: Option<PathBuf>,
    pub cache: PathBuf,
    pub errata: Option<PathBuf>,
}

impl Settings {
    fn new(args: Args, date: String) -> Self {
        Self {
            rfc_dir: args.rfc_dir,
            drafts_dir: args.drafts_dir,
            overrides: args.overrides,
            out: args.out,
            date,
            only: args.only,
            dump_findings: args.dump_findings,
            sources: args.sources,
            cache: args.cache,
            errata: args.errata,
        }
    }
}

/// Today's date in UTC, `YYYY-MM-DD`.
fn utc_today() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    civil_date(seconds / 86_400)
}

/// The proleptic Gregorian date `days` after 1970-01-01 (H. Hinnant, "chrono-Compatible Low-Level Date Algorithms").
fn civil_date(days: u64) -> String {
    let z = days.saturating_add(719_468);
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = doe
        .saturating_sub(doe / 1_460)
        .saturating_add(doe / 36_524)
        .saturating_sub(doe / 146_096)
        / 365;
    let doy = doe.saturating_sub(
        yoe.saturating_mul(365)
            .saturating_add(yoe / 4)
            .saturating_sub(yoe / 100),
    );
    let mp = doy.saturating_mul(5).saturating_add(2) / 153;
    let day = doy
        .saturating_sub(mp.saturating_mul(153).saturating_add(2) / 5)
        .saturating_add(1);
    let month = if mp < 10 {
        mp.saturating_add(3)
    } else {
        mp.saturating_sub(9)
    };
    let year = yoe
        .saturating_add(era.saturating_mul(400))
        .saturating_add(u64::from(month <= 2));
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::civil_date;

    #[test]
    fn civil_dates() {
        assert_eq!(civil_date(0), "1970-01-01");
        assert_eq!(civil_date(11_016), "2000-02-29");
        assert_eq!(civil_date(20_762), "2026-11-05");
    }
}
