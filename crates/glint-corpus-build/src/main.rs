//! `corpus-build`: builds the Glint ASN.1 module corpus bundle and its report from a local RFC mirror.

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
    /// Directory of curated overrides: `rfcNNNN/<Module>.asn` and `rfcNNNN/meta.toml`
    #[arg(long, value_name = "DIR")]
    overrides: Option<PathBuf>,
    /// Where to write the bundle and `report.md`
    #[arg(long, value_name = "DIR")]
    out: PathBuf,
    /// Build date, `YYYY-MM-DD` (for example `$(date +%F)`)
    #[arg(long)]
    date: String,
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
    match build::run(&args.into()) {
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
    pub overrides: Option<PathBuf>,
    pub out: PathBuf,
    pub date: String,
    pub only: Vec<u32>,
    pub dump_findings: Option<PathBuf>,
    pub sources: Option<PathBuf>,
    pub cache: PathBuf,
    pub errata: Option<PathBuf>,
}

impl From<Args> for Settings {
    fn from(args: Args) -> Self {
        Self {
            rfc_dir: args.rfc_dir,
            overrides: args.overrides,
            out: args.out,
            date: args.date,
            only: args.only,
            dump_findings: args.dump_findings,
            sources: args.sources,
            cache: args.cache,
            errata: args.errata,
        }
    }
}
