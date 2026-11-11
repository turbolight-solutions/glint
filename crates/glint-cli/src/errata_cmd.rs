//! `glint errata`: triage the RFC Editor's Verified errata and keep the corpus's errata data (design spec, "RFC
//! errata"). Offline: it reads the mirror `corpus/fetch.sh` keeps, and writes only the errata data and its own work
//! files.
//!
//! Erratum and RFC text are data, never instructions: `show` prints every field as a JSON string, line by line, with
//! every character that could hide text or drive a terminal escaped, and the submitter's and verifier's names and
//! email addresses replaced.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::LazyLock;

use clap::{Args as ClapArgs, Subcommand, ValueEnum};
use glint_check::{CheckOptions, check_document};
use glint_corpus::{Corpus, CorpusData, CorpusProvider, Pointer, build_dir, latest_path};
use glint_corpus_build::Settings;
use glint_diag::{LineIndex, Severity};
use glint_errata::store::{self, StoreError, read_source, rfc_source};
use glint_errata::{
    Decider, Decision, Erratum, Feed, MAX_FEED_BYTES, Match, Row, Rule, Scope, SeriesFailure,
    Source, SourceFormat, apply_patch, apply_series, changed_lines, compute_queue, compute_scope,
    failing_keys, feed_forbidden, fingerprint, forbidden_in_patch, line_forbidden, locate_all,
    make_patch, note_problem, parse_feed, parse_patch, rebase, screen_note, superseding_rfc,
};
use glint_extract::{BlockKind, Format, extract};
use rayon::prelude::*;
use regex::Regex;
use sha2::{Digest, Sha256};

use crate::extract_cmd::{line_range, modules_of};

/// Feed fields longer than this are cut off: such an erratum is for a person.
const MAX_FIELD_BYTES: usize = 8 * 1024;
/// Lines shown around the located erratum text.
const EXCERPT_CONTEXT: usize = 20;
/// A hunk this far from every place the erratum's text was found is flagged.
const FAR_LINES: usize = 100;
/// A longer `section` may hold the instruction, and the feed cuts it at about 250 characters.
const SECTION_LONG: usize = 200;
/// The fingerprint of a row whose erratum is no longer in the feed.
const MISSING_FINGERPRINT: &str = "0000000000000000";
/// The diagnostics of `glint check` that compare inline ASN.1 with the modules.
const SNIPPET_CODES: [&str; 4] = ["E300", "W301", "W302", "W310"];

#[derive(ClapArgs)]
pub struct Args {
    /// The corpus directory: `errata/` (the ledger and patches this command keeps), `overrides/`, `sources.toml`
    #[arg(long, value_name = "DIR", default_value = "corpus")]
    corpus_dir: PathBuf,
    /// The mirror `corpus/fetch.sh` keeps: `rfc/`, `drafts/`, `errata.json`, `smi-numbers.xml`; Glint writes its
    /// downloads to `files/` and the triage work files to `errata-work/` there
    #[arg(long, value_name = "DIR", default_value = "corpus/cache")]
    cache_dir: PathBuf,
    /// Corpus data whose RFC modules put an RFC in triage scope (default: $GLINT_CORPUS, then the installed data)
    #[arg(long, value_name = "FILE")]
    corpus: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List the errata awaiting triage, and the `needs-human` ones
    Queue {
        /// Only this RFC
        #[arg(long, value_parser = ascii_number)]
        rfc: Option<u32>,
        /// Show at most this many
        #[arg(long)]
        limit: Option<usize>,
    },
    /// List every erratum of an RFC in the feed, whatever its status, with its ledger decision
    ///
    /// Errata that are not Verified never reach the queue; this is how a newly filed one is seen.
    List {
        #[arg(value_parser = ascii_number)]
        rfc: u32,
    },
    /// Show one erratum, where its text is in the RFC, the RFC's ASN.1 and its other errata
    Show {
        #[arg(value_parser = ascii_number)]
        rfc: u32,
        #[arg(value_parser = ascii_number)]
        errata_id: u32,
        /// Lines shown around the located text
        #[arg(long, default_value_t = EXCERPT_CONTEXT)]
        context: usize,
        /// Print no RFC lines, only where the erratum's text was found
        #[arg(long)]
        no_excerpt: bool,
    },
    /// Write the work file of an erratum: the RFC's source with its lower-numbered patched errata applied
    Edit {
        #[arg(value_parser = ascii_number)]
        rfc: u32,
        #[arg(value_parser = ascii_number)]
        errata_id: u32,
        /// Start again from the base text, replacing a work file in progress
        #[arg(long)]
        force: bool,
    },
    /// Build the corpus before and after the edit in a work file, and compare the findings
    ///
    /// Both sides get the RFC's higher-numbered patched errata. A new error or warning, in the RFC's modules and inline
    /// ASN.1 or in any corpus module, is a loss: the exit status is 1.
    Prove {
        /// The work file `edit` wrote, after the edit: `<cache>/errata-work/rfcN-ID.txt` or `.xml`
        work_file: PathBuf,
    },
    /// Record a decision; for `patched`, the patch is the work file's change. Higher patches of the RFC are rebased
    Record {
        #[arg(value_parser = ascii_number)]
        rfc: u32,
        #[arg(value_parser = ascii_number)]
        errata_id: u32,
        #[arg(long, value_enum)]
        decision: DecisionArg,
        /// Why: `superseded` names the RFC ("RFC NNNN"), `needs-human` gives the reason
        #[arg(long, default_value = "")]
        note: String,
        #[arg(long, value_enum, default_value_t = ByArg::Claude)]
        by: ByArg,
    },
    /// Check the errata data: the ledger, the patches and that every RFC's series applies to the mirror
    Check,
}

#[derive(Clone, Copy, ValueEnum)]
enum DecisionArg {
    Patched,
    NotApplicable,
    Superseded,
    NeedsHuman,
}

impl From<DecisionArg> for Decision {
    fn from(d: DecisionArg) -> Self {
        match d {
            DecisionArg::Patched => Decision::Patched,
            DecisionArg::NotApplicable => Decision::NotApplicable,
            DecisionArg::Superseded => Decision::Superseded,
            DecisionArg::NeedsHuman => Decision::NeedsHuman,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum ByArg {
    Claude,
    Human,
}

fn ascii_number(text: &str) -> Result<u32, String> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err("expected a number in ASCII digits".to_owned());
    }
    text.parse().map_err(|e| format!("{e}"))
}

struct Paths {
    errata: PathBuf,
    overrides: PathBuf,
    sources: PathBuf,
    rfcs: PathBuf,
    drafts: PathBuf,
    feed: PathBuf,
    iana: PathBuf,
    files: PathBuf,
    work: PathBuf,
}

impl Paths {
    fn new(args: &Args) -> Self {
        let (corpus, cache) = (&args.corpus_dir, &args.cache_dir);
        Self {
            errata: corpus.join("errata"),
            overrides: corpus.join("overrides"),
            sources: corpus.join("sources.toml"),
            rfcs: cache.join("rfc"),
            drafts: cache.join("drafts"),
            feed: cache.join("errata.json"),
            iana: cache.join("smi-numbers.xml"),
            files: cache.join("files"),
            work: cache.join("errata-work"),
        }
    }

    fn work_file(&self, source: Source, errata_id: u32) -> PathBuf {
        let extension = match source.format {
            SourceFormat::Xml => "xml",
            SourceFormat::Text => "txt",
        };
        self.work
            .join(format!("rfc{}-{errata_id}.{extension}", source.rfc))
    }

    fn base_file(&self, rfc: u32, errata_id: u32) -> PathBuf {
        self.work.join(format!("rfc{rfc}-{errata_id}.base"))
    }
}

pub fn run(args: &Args) -> ExitCode {
    let paths = Paths::new(args);
    let result = match &args.command {
        Command::Queue { rfc, limit } => queue(&paths, args.corpus.as_deref(), *rfc, *limit),
        Command::List { rfc } => list(&paths, *rfc),
        Command::Show {
            rfc,
            errata_id,
            context,
            no_excerpt,
        } => show(&paths, *rfc, *errata_id, *context, *no_excerpt),
        Command::Edit {
            rfc,
            errata_id,
            force,
        } => edit(&paths, *rfc, *errata_id, *force),
        Command::Prove { work_file } => prove(&paths, work_file),
        Command::Record {
            rfc,
            errata_id,
            decision,
            note,
            by,
        } => {
            let by = match by {
                ByArg::Claude => Decider::Claude,
                ByArg::Human => Decider::Human,
            };
            record(&paths, *rfc, *errata_id, (*decision).into(), note, by)
        }
        Command::Check => check(&paths),
    };
    match result {
        Ok(code) => code,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::from(1)
        }
    }
}

// -- escaping and redaction ------------------------------------------------------------------------------------------

/// `text` as a JSON string, with every character that is not plainly printable escaped as `\u{...}`: controls, bidi
/// and invisible characters, line separators, and any whitespace but the space.
fn escape_line(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            ' ' => out.push(' '),
            c if c.is_control() || c.is_whitespace() || feed_forbidden(c) || line_forbidden(c) => {
                let _ = write!(out, "\\u{{{:04x}}}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

static EMAIL: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}").ok()
});

/// `text` without the erratum's submitter and verifier names or any email address.
fn redact(text: &str, e: &Erratum) -> String {
    let mut out = text.to_owned();
    for name in e.names.iter().filter(|n| n.chars().count() >= 3) {
        if let Ok(re) = Regex::new(&format!("(?i){}", regex::escape(name))) {
            out = re.replace_all(&out, "[name]").into_owned();
        }
    }
    match EMAIL.as_ref() {
        Some(re) => re.replace_all(&out, "[email]").into_owned(),
        None => out,
    }
}

/// Prints a field line by line, escaped and capped; returns whether it was cut off.
fn print_field(label: &str, text: &str, erratum: Option<&Erratum>) -> bool {
    let unified = text.replace("\r\n", "\n").replace('\r', "\n");
    let text = erratum.map_or(unified.clone(), |e| redact(&unified, e));
    let truncated = text.len() > MAX_FIELD_BYTES;
    let mut cut = MAX_FIELD_BYTES.min(text.len());
    while !text.is_char_boundary(cut) {
        cut = cut.saturating_sub(1);
    }
    let shown = text.get(..cut).unwrap_or_default();
    let lines: Vec<&str> = if shown.is_empty() {
        Vec::new()
    } else {
        shown.split('\n').collect()
    };
    println!(
        "{label} ({} lines{}):",
        lines.len(),
        if truncated { ", TRUNCATED" } else { "" }
    );
    for line in lines {
        println!("  {}", escape_line(line));
    }
    if truncated {
        println!("  ... truncated at {MAX_FIELD_BYTES} bytes: triage as needs-human");
    }
    truncated
}

// -- state -----------------------------------------------------------------------------------------------------------

fn store_error(e: StoreError) -> String {
    e.to_string()
}

fn load_feed(paths: &Paths) -> Result<Feed, String> {
    let raw = store::read_regular_file(&paths.feed, MAX_FEED_BYTES)
        .map_err(|e| format!("no usable errata feed: {e}; run corpus/fetch.sh"))?;
    parse_feed(&raw).map_err(|e| format!("{}: {e}", paths.feed.display()))
}

fn load_rows(paths: &Paths) -> Result<Vec<Row>, String> {
    store::read_ledger(&paths.errata).map_err(store_error)
}

/// RFC `rfc`'s published source as corpus-build reads it.
fn published(paths: &Paths, rfc: u32) -> Result<(String, Source), String> {
    let (path, format) = rfc_source(&paths.rfcs, rfc).ok_or_else(|| {
        format!(
            "rfc{rfc} is not in {}; run corpus/fetch.sh",
            paths.rfcs.display()
        )
    })?;
    Ok((
        read_source(&path).map_err(store_error)?,
        Source { rfc, format },
    ))
}

/// The patches of RFC `rfc`'s `patched` rows, `(errata_id, patch)` in id order.
fn series_of(paths: &Paths, rows: &[Row], rfc: u32) -> Result<Vec<(u32, String)>, SeriesFailure> {
    let (files, _) = store::scan_patches(&paths.errata);
    let rfc_rows: Vec<Row> = rows.iter().filter(|r| r.rfc == rfc).cloned().collect();
    store::patched_series(&paths.errata, &rfc_rows, &files)
        .remove(&rfc)
        .unwrap_or_else(|| Ok(Vec::new()))
}

fn below(series: &[(u32, String)], errata_id: u32) -> Vec<(u32, String)> {
    series
        .iter()
        .filter(|(id, _)| *id < errata_id)
        .cloned()
        .collect()
}

/// The text erratum `errata_id`'s patch applies to: the published source plus every lower patched erratum.
fn base_for(
    paths: &Paths,
    rows: &[Row],
    rfc: u32,
    errata_id: u32,
) -> Result<(String, Source), String> {
    let (text, source) = published(paths, rfc)?;
    let series = series_of(paths, rows, rfc)
        .map_err(|f| format!("rfc{rfc} erratum {}: {}", f.errata_id, f.reason))?;
    let base = apply_series(&text, &below(&series, errata_id), source).map_err(|f| {
        format!(
            "rfc{rfc}: the patch of erratum {} does not apply ({}); triage it first",
            f.errata_id, f.reason
        )
    })?;
    Ok((base, source))
}

fn digest(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .fold(String::new(), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
}

fn get_erratum(feed: &Feed, rfc: u32, errata_id: u32) -> Result<Option<&Erratum>, String> {
    match feed.errata.get(&errata_id) {
        Some(e) if e.rfc != rfc => Err(format!(
            "erratum {errata_id} is of rfc{}, not rfc{rfc}",
            e.rfc
        )),
        found => Ok(found),
    }
}

/// The RFC numbers of the mirror, from its `rfcN.txt` and `rfcN.xml` names.
fn mirror_rfcs(paths: &Paths) -> BTreeSet<u32> {
    let Ok(entries) = std::fs::read_dir(&paths.rfcs) else {
        return BTreeSet::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let stem = name
                .strip_suffix(".txt")
                .or_else(|| name.strip_suffix(".xml"))?;
            ascii_number(stem.strip_prefix("rfc")?).ok()
        })
        .collect()
}

/// The patched errata whose patches do not apply: for each RFC, the first that is missing, unreadable or fails, and
/// every higher one.
fn failing(paths: &Paths, rows: &[Row]) -> BTreeSet<(u32, u32)> {
    let mut found = BTreeSet::new();
    let rfcs: BTreeSet<u32> = rows
        .iter()
        .filter(|r| r.decision == Decision::Patched)
        .map(|r| r.rfc)
        .collect();
    for rfc in rfcs {
        let Ok((text, source)) = published(paths, rfc) else {
            continue;
        };
        let failure = series_of(paths, rows, rfc)
            .and_then(|series| apply_series(&text, &series, source).map(|_| ()));
        if let Err(f) = failure {
            found.extend(failing_keys(rows, rfc, f.errata_id));
        }
    }
    found
}

// -- queue -----------------------------------------------------------------------------------------------------------

/// The RFCs with modules in the corpus, and those with inline ASN.1 in the mirror (the extractor's rule:
/// [`glint_extract::Extraction::has_inline_asn1`]).
fn asn1_rfcs(
    paths: &Paths,
    corpus: Option<&Path>,
) -> Result<(BTreeSet<u32>, BTreeSet<u32>), String> {
    let corpus = crate::common::load_corpus(corpus)?;
    let modules: BTreeSet<u32> = corpus
        .modules()
        .iter()
        .filter_map(|m| m.source.rfc_number())
        .collect();
    let rfcs: Vec<u32> = mirror_rfcs(paths).into_iter().collect();
    let inline: BTreeSet<u32> = rfcs
        .into_par_iter()
        .filter(|&rfc| {
            published(paths, rfc).is_ok_and(|(text, source)| {
                extract(&text, extract_format(source.format)).has_inline_asn1()
            })
        })
        .collect();
    Ok((modules, inline))
}

fn extract_format(format: SourceFormat) -> Format {
    match format {
        SourceFormat::Xml => Format::Xml,
        SourceFormat::Text => Format::Text,
    }
}

fn scope(paths: &Paths, corpus: Option<&Path>, rows: &[Row], feed: &Feed) -> Result<Scope, String> {
    let (modules, inline) = asn1_rfcs(paths, corpus)?;
    let (files, _) = store::scan_patches(&paths.errata);
    let patch_keys: BTreeSet<(u32, u32)> = files.keys().copied().collect();
    Ok(compute_scope(
        &modules,
        &inline,
        rows,
        &patch_keys,
        Some(feed),
    ))
}

fn queue(
    paths: &Paths,
    corpus: Option<&Path>,
    rfc: Option<u32>,
    limit: Option<usize>,
) -> Result<ExitCode, String> {
    let feed = load_feed(paths)?;
    let rows = load_rows(paths)?;
    let scope = scope(paths, corpus, &rows, &feed)?;
    let queue = compute_queue(&feed, &rows, &scope, &failing(paths, &rows));
    let wanted = |n: u32| rfc.is_none_or(|r| r == n);
    let items: Vec<_> = queue.items.iter().filter(|i| wanted(i.rfc)).collect();
    let for_rfc = rfc.map(|r| format!(" for rfc{r}")).unwrap_or_default();
    println!(
        "{} errata awaiting triage{for_rfc} ({} RFCs in scope)",
        items.len(),
        scope.rfcs.len()
    );
    let shown = limit.map_or(items.len(), |l| l.min(items.len()));
    for item in items.iter().take(shown) {
        let section: String = feed.errata.get(&item.errata_id).map_or(String::new(), |e| {
            redact(&e.section, e).chars().take(60).collect()
        });
        println!(
            "  rfc{} {} {} section {}",
            item.rfc,
            item.errata_id,
            item.reason,
            escape_line(&section)
        );
    }
    if shown < items.len() {
        println!("  ... {} more", items.len().saturating_sub(shown));
    }
    let needs_human: Vec<_> = queue.needs_human.iter().filter(|r| wanted(r.rfc)).collect();
    println!(
        "{} needs-human (not triaged again unless the erratum changes)",
        needs_human.len()
    );
    for row in needs_human {
        println!(
            "  rfc{} {}: {}",
            row.rfc,
            row.errata_id,
            escape_line(&row.note)
        );
    }
    let skipped: Vec<&String> = feed
        .malformed
        .iter()
        .filter(|m| match malformed_rfc(m) {
            Some(n) => scope.rfcs.contains(&n) && wanted(n),
            None => rfc.is_none(),
        })
        .collect();
    if !skipped.is_empty() {
        println!(
            "{} feed entries skipped as malformed, in scope or of an unknown RFC (decide them by hand at \
             https://www.rfc-editor.org/errata/eidID):",
            skipped.len()
        );
        for m in skipped {
            println!("  {}", escape_line(m));
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn malformed_rfc(reason: &str) -> Option<u32> {
    let start = reason.find("(rfc")?.saturating_add(4);
    let rest = reason.get(start..)?;
    ascii_number(rest.get(..rest.find(')')?)?).ok()
}

// -- show ------------------------------------------------------------------------------------------------------------

/// A part of the RFC's source the erratum may fall in: a module or inline ASN.1, with 1-based lines.
struct Region {
    first: usize,
    last: usize,
    label: String,
}

fn regions_of(text: &str, format: SourceFormat) -> Vec<Region> {
    let index = LineIndex::for_document(text);
    let lines = |range| {
        let (first, last) = line_range(&index, range);
        (
            usize::try_from(first).unwrap_or_default(),
            usize::try_from(last).unwrap_or_default(),
        )
    };
    let mut regions = Vec::new();
    for block in extract(text, extract_format(format)).blocks {
        if block.kind == BlockKind::Fragment {
            let (first, last) = lines(block.origin.span.clone());
            let section = block
                .origin
                .section
                .as_ref()
                .map(|s| format!(" ({s})"))
                .unwrap_or_default();
            regions.push(Region {
                first,
                last,
                label: format!("inline ASN.1{section}"),
            });
            continue;
        }
        for module in modules_of(&block) {
            let range = module.start..module.start.saturating_add(module.text.len());
            let Some(original) = block.original_range(range) else {
                continue;
            };
            let (first, last) = lines(original);
            let name = module
                .name
                .clone()
                .unwrap_or_else(|| "(unnamed)".to_owned());
            regions.push(Region {
                first,
                last,
                label: format!("module {name}"),
            });
        }
    }
    regions.sort_by_key(|r| (r.first, r.last));
    regions
}

fn region_at<'a>(regions: &'a [Region], m: &Match) -> Option<&'a Region> {
    regions
        .iter()
        .find(|r| r.first <= m.first && m.first <= r.last)
}

/// Where the erratum's text is: its original text, else its corrected text (the RFC may already say it).
fn located(text: &str, e: &Erratum) -> (Vec<Match>, &'static str) {
    for (field, quote) in [
        ("orig_text", &e.orig_text),
        ("correct_text", &e.correct_text),
    ] {
        let found = locate_all(text, quote);
        if !found.is_empty() {
            return (found, field);
        }
    }
    (Vec::new(), "")
}

static SECTION_NUMBER: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"(?:Appendix\s+)?\b([A-Z]|[0-9]+)((?:\.[0-9]+)*)\b").ok());

/// The 1-based line of the heading of `section` ("Appendix A", "4.1"), skipping table-of-contents lines.
fn section_line(lines: &[&str], section: &str) -> Option<usize> {
    let captures = SECTION_NUMBER.as_ref()?.captures(section)?;
    let lead = captures.get(1)?.as_str();
    let number = format!("{lead}{}", captures.get(2).map_or("", |m| m.as_str()));
    let prefix = if lead.chars().all(|c| c.is_ascii_alphabetic()) {
        r"(?:Appendix\s+)?"
    } else {
        ""
    };
    let heading = Regex::new(&format!(r"^\s*{prefix}{}\.?\s+\S", regex::escape(&number))).ok()?;
    lines
        .iter()
        .position(|l| heading.is_match(l) && !l.contains("....") && !l.contains(". . ."))
        .map(|i| i.saturating_add(1))
}

/// The match `show` excerpts: the most complete, then one in a module or inline ASN.1, then the nearest the
/// erratum's section heading.
fn best_match<'a>(
    matches: &'a [Match],
    regions: &[Region],
    heading: Option<usize>,
) -> Option<&'a Match> {
    matches.iter().min_by_key(|m| {
        let complete = if m.total == 0 {
            1000
        } else {
            m.matched
                .saturating_mul(1000)
                .checked_div(m.total)
                .unwrap_or(0)
        };
        (
            !m.exact,
            std::cmp::Reverse(complete),
            region_at(regions, m).is_none(),
            heading.map_or(0, |h| h.abs_diff(m.first)),
            m.first,
        )
    })
}

fn describe(m: &Match, regions: &[Region]) -> String {
    let place = region_at(regions, m).map_or("in prose".to_owned(), |r| {
        format!("in {} (lines {}-{})", r.label, r.first, r.last)
    });
    let how = if m.exact {
        String::new()
    } else {
        format!(", {} of {} quoted pieces", m.matched, m.total)
    };
    format!("lines {}-{}{how}: {place}", m.first, m.last)
}

fn field_warnings(e: &Erratum) -> Vec<String> {
    let mut warnings = Vec::new();
    if e.correct_text.trim().is_empty() && !e.orig_text.trim().is_empty() {
        warnings.push("correct_text is empty: the correction may be in orig_text or notes; apply it only if its meaning is certain".to_owned());
    }
    let length = e.section.chars().count();
    if length > SECTION_LONG {
        warnings.push(format!("section is {length} characters: it may hold the instruction, and the feed cuts it off at about 250"));
    }
    warnings
}

fn show(
    paths: &Paths,
    rfc: u32,
    errata_id: u32,
    context: usize,
    no_excerpt: bool,
) -> Result<ExitCode, String> {
    let feed = load_feed(paths)?;
    let rows = load_rows(paths)?;
    let erratum = get_erratum(&feed, rfc, errata_id)?;
    match erratum {
        None => println!(
            "erratum {errata_id} is not in the feed (withdrawn); only its ledger row and patch are shown"
        ),
        Some(e) => {
            println!(
                "rfc{rfc} erratum {errata_id}: status {}, fingerprint {}",
                escape_line(&e.status),
                fingerprint(e)
            );
            let mut truncated = false;
            for (label, value) in [
                ("section", &e.section),
                ("orig_text", &e.orig_text),
                ("correct_text", &e.correct_text),
                ("notes", &e.notes),
            ] {
                truncated |= print_field(label, value, Some(e));
            }
            if truncated {
                println!("A field was truncated: record this erratum as needs-human.");
            }
            for warning in field_warnings(e) {
                println!("WARNING: {warning}");
            }
        }
    }
    println!();
    match rows.iter().find(|r| r.key() == (rfc, errata_id)) {
        Some(row) => {
            let changed = erratum.is_some_and(|e| fingerprint(e) != row.fingerprint);
            println!(
                "ledger: {} by {} under rule {}, fingerprint {}{}, note {}",
                row.decision,
                row.by,
                row.rule,
                row.fingerprint,
                if changed {
                    " (erratum changed since)"
                } else {
                    ""
                },
                escape_line(&row.note)
            );
        }
        None => println!("ledger: no decision yet"),
    }
    let patch_file = store::patch_path(&paths.errata, rfc, errata_id);
    if let Ok(patch) = store::read_patch(&patch_file) {
        print_field("patch", &patch, None);
    }
    println!();
    match base_for(paths, &rows, rfc, errata_id) {
        Err(message) => println!("{message}"),
        Ok((text, source)) => show_text(&text, source, erratum, errata_id, context, no_excerpt),
    }
    println!();
    let others: Vec<&Erratum> = feed
        .for_rfc(rfc)
        .filter(|x| x.errata_id != errata_id)
        .collect();
    println!(
        "other errata of rfc{rfc}:{}",
        if others.is_empty() { " none" } else { "" }
    );
    print_errata(&others, &rows, false);
    Ok(ExitCode::SUCCESS)
}

fn list(paths: &Paths, rfc: u32) -> Result<ExitCode, String> {
    let feed = load_feed(paths)?;
    let rows = load_rows(paths)?;
    let errata: Vec<&Erratum> = feed.for_rfc(rfc).collect();
    println!("rfc{rfc}: {} errata in the feed", errata.len());
    print_errata(&errata, &rows, true);
    Ok(ExitCode::SUCCESS)
}

/// How much of a `section` field `list` prints: enough to name the section, never a whole quote.
const SECTION_SHOWN: usize = 80;

/// One line per erratum: id, status, its ledger decision (or `untriaged`) and, with `section`, the section it names.
fn print_errata(errata: &[&Erratum], rows: &[Row], section: bool) {
    let by_key: BTreeMap<(u32, u32), &Row> = rows.iter().map(|r| (r.key(), r)).collect();
    for x in errata {
        let decision = by_key.get(&x.key()).map_or("untriaged".to_owned(), |r| {
            format!("{} ({})", r.decision, escape_line(&r.note))
        });
        let section = if section {
            let text: String = redact(&x.section, x).chars().take(SECTION_SHOWN).collect();
            format!(" section {}", escape_line(&text))
        } else {
            String::new()
        };
        println!(
            "  {} {} {decision}{section}",
            x.errata_id,
            escape_line(&x.status)
        );
    }
}

fn show_text(
    text: &str,
    source: Source,
    erratum: Option<&Erratum>,
    errata_id: u32,
    context: usize,
    no_excerpt: bool,
) {
    let name = source.file_name();
    let lines: Vec<&str> = text.split('\n').collect();
    let regions = regions_of(text, source.format);
    let listing: Vec<String> = regions
        .iter()
        .map(|r| format!("{} (lines {}-{})", r.label, r.first, r.last))
        .collect();
    println!(
        "ASN.1 in {name} (published, with the patched errata below {errata_id}): {}",
        if listing.is_empty() {
            "none".to_owned()
        } else {
            listing.join(", ")
        }
    );
    let (matches, field) = erratum.map_or((Vec::new(), ""), |e| located(text, e));
    let heading = erratum.and_then(|e| section_line(&lines, &e.section));
    let quoted = erratum.map_or(1, |e| e.orig_text.split('\n').count());
    let (at, end, how) = match best_match(&matches, &regions, heading) {
        Some(best) => {
            let exactly = if best.exact {
                "ignoring whitespace"
            } else {
                "partly: some quoted pieces"
            };
            println!(
                "{field} located ({exactly}), {} place{}:",
                matches.len(),
                if matches.len() > 1 { "s" } else { "" }
            );
            for m in &matches {
                println!(
                    "  {}{}",
                    describe(m, &regions),
                    if m == best { "  <- shown" } else { "" }
                );
            }
            if field == "correct_text" {
                println!("  the text may already be corrected");
            }
            let end = best
                .last
                .min(best.first.saturating_add(quoted).saturating_add(context));
            (best.first, end, format!("{field} located"))
        }
        None => match heading {
            Some(h) => (
                h,
                h.saturating_add(quoted),
                "orig_text not found; showing the section heading".to_owned(),
            ),
            None => (
                1,
                1usize.saturating_add(quoted),
                "not located; showing the start of the RFC".to_owned(),
            ),
        },
    };
    let first = at.saturating_sub(context).max(1);
    let last = end.saturating_add(context).min(lines.len());
    if no_excerpt {
        println!(
            "{name}: excerpt not shown (--no-excerpt); it would be lines {first}-{last}: {how}"
        );
        return;
    }
    println!("{name} lines {first}-{last}: {how}");
    for (number, line) in lines
        .iter()
        .enumerate()
        .map(|(i, l)| (i.saturating_add(1), l))
    {
        if (first..=last).contains(&number) {
            println!("  {number:6} {}", escape_line(line));
        }
    }
}

// -- edit ------------------------------------------------------------------------------------------------------------

/// Creates the work directory, private to the user, and refuses it if it is not a real directory.
fn work_dir(paths: &Paths) -> Result<(), String> {
    match std::fs::symlink_metadata(&paths.work) {
        Ok(m) if m.file_type().is_dir() => Ok(()),
        Ok(_) => Err(format!(
            "{} is not a directory (symbolic links are refused)",
            paths.work.display()
        )),
        Err(_) => {
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
            builder
                .create(&paths.work)
                .map_err(|e| format!("{}: {e}", paths.work.display()))
        }
    }
}

fn edit(paths: &Paths, rfc: u32, errata_id: u32, force: bool) -> Result<ExitCode, String> {
    let rows = load_rows(paths)?;
    let (base, source) = base_for(paths, &rows, rfc, errata_id)?;
    work_dir(paths)?;
    let path = paths.work_file(source, errata_id);
    if std::fs::symlink_metadata(&path).is_ok() && !force {
        return Err(format!(
            "{} exists (an edit in progress?); pass --force to start again from the base text",
            path.display()
        ));
    }
    // The work file first, then the digest of its base: after a crash in between, `record` sees the old digest and
    // refuses rather than accepting a work file made from another base.
    store::atomic_write(&path, base.as_bytes()).map_err(store_error)?;
    store::atomic_write(
        &paths.base_file(rfc, errata_id),
        format!("{}\n", digest(&base)).as_bytes(),
    )
    .map_err(store_error)?;
    let lower = rows
        .iter()
        .any(|r| r.rfc == rfc && r.decision == Decision::Patched && r.errata_id < errata_id);
    println!(
        "wrote {}: the published {}{}. Edit it, run `glint errata prove`, then `record`.",
        path.display(),
        source.file_name(),
        if lower {
            format!(" plus its patched errata below {errata_id}")
        } else {
            String::new()
        }
    );
    Ok(ExitCode::SUCCESS)
}

/// The work file of an edit, checked against the base `edit` wrote it from.
fn read_work(
    paths: &Paths,
    rfc: u32,
    errata_id: u32,
    base: &str,
    path: &Path,
) -> Result<String, String> {
    let bytes = store::read_regular_file(path, store::MAX_SOURCE_BYTES)
        .map_err(|e| format!("cannot read the work file: {e}; run `edit` first"))?;
    let edited =
        String::from_utf8(bytes).map_err(|_| format!("{} is not UTF-8", path.display()))?;
    let base_file = paths.base_file(rfc, errata_id);
    let started_from = store::read_regular_file(&base_file, 128)
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .ok_or_else(|| format!("cannot read {}: the work file was not made by `edit`; run `edit {rfc} {errata_id} --force` and make the edit again", base_file.display()))?;
    if started_from.trim() != digest(base) {
        return Err(format!(
            "the base text of rfc{rfc} erratum {errata_id} changed since `edit` (a lower erratum was recorded or backed out, or the RFC changed upstream); a patch from this work file would undo that change. Run `edit {rfc} {errata_id} --force` and make the edit again"
        ));
    }
    Ok(edited)
}

// -- prove -----------------------------------------------------------------------------------------------------------

static WORK_NAME: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"^rfc([0-9]+)-([0-9]+)\.(txt|xml)$").ok());

/// The RFC's higher patched errata applied to both texts in id order, up to the first that does not apply to either.
/// Returns both texts, the errata applied and a note for each left out.
fn apply_later(
    mut before: String,
    mut after: String,
    later: &[(u32, String)],
    source: Source,
) -> (String, String, Vec<u32>, Vec<String>) {
    let mut applied = Vec::new();
    let mut notes = Vec::new();
    for (position, (id, patch)) in later.iter().enumerate() {
        let new_before = apply_patch(&before, patch, source);
        let new_after = apply_patch(&after, patch, source);
        match (new_before, new_after) {
            (Ok(b), Ok(a)) => {
                before = b;
                after = a;
                applied.push(*id);
            }
            (b, a) => {
                let rest: Vec<String> = later
                    .iter()
                    .skip(position.saturating_add(1))
                    .map(|(i, _)| i.to_string())
                    .collect();
                let left_out = if rest.is_empty() {
                    format!("erratum {id}")
                } else {
                    format!("erratum {id} and the higher {}", rest.join(", "))
                };
                notes.push(match (b, a) {
                    (_, Err(e)) => format!("erratum {id} does not apply after this edit ({}): recording it queues erratum {id} as patch-fails; {left_out} left out of both builds", e.0),
                    (Err(e), _) => format!("erratum {id} does not apply without this edit ({}); {left_out} left out of both builds", e.0),
                    _ => String::new(),
                });
                break;
            }
        }
    }
    (before, after, applied, notes)
}

/// A finding of a corpus build, without its line: (document, module or `(inline)`, code, message).
type Finding = (String, String, String, String);

/// Builds the corpus with RFC `rfc`'s source replaced by `text` (its errata already in it) into `dir`, and returns its
/// errors and warnings: every module's, and those `glint check` reports on RFC `rfc`'s inline ASN.1.
fn build_side(
    paths: &Paths,
    source: Source,
    text: &str,
    rows: &[Row],
    dir: &Path,
) -> Result<(Vec<Finding>, Corpus), String> {
    let _ = std::fs::remove_dir_all(dir);
    let rfcs = dir.join("rfc");
    std::fs::create_dir_all(&rfcs).map_err(|e| format!("{}: {e}", rfcs.display()))?;
    if !std::fs::symlink_metadata(dir).is_ok_and(|m| m.file_type().is_dir()) {
        return Err(format!("{} is not a directory", dir.display()));
    }
    let source_name = source.file_name();
    for entry in std::fs::read_dir(&paths.rfcs)
        .map_err(|e| format!("{}: {e}", paths.rfcs.display()))?
        .filter_map(Result::ok)
    {
        let name = entry.file_name();
        if name.to_string_lossy() == source_name {
            continue;
        }
        link_or_copy(&entry.path(), &rfcs.join(&name))?;
    }
    std::fs::write(rfcs.join(&source_name), text).map_err(|e| format!("{source_name}: {e}"))?;
    // The other RFCs' errata; this RFC's are in `text` already.
    let errata = dir.join("errata");
    let others: Vec<Row> = rows
        .iter()
        .filter(|r| r.rfc != source.rfc)
        .cloned()
        .collect();
    store::write_ledger(&errata, &others).map_err(store_error)?;
    let (files, _) = store::scan_patches(&paths.errata);
    for ((rfc, id), path) in files.iter().filter(|((rfc, _), _)| *rfc != source.rfc) {
        let patch = store::read_patch(path).map_err(store_error)?;
        store::write_patch(&errata, *rfc, *id, &patch).map_err(store_error)?;
    }
    let out = dir.join("out");
    let findings_file = dir.join("findings.tsv");
    let present = |p: &Path| p.exists().then(|| p.to_owned());
    let settings = Settings {
        rfc_dir: rfcs,
        drafts_dir: present(&paths.drafts),
        overrides: present(&paths.overrides),
        out: out.clone(),
        date: "2000-01-01".to_owned(),
        only: Vec::new(),
        dump_findings: Some(findings_file.clone()),
        sources: present(&paths.sources),
        cache: paths.files.clone(),
        errata: None,
        errata_dir: Some(errata),
        iana: present(&paths.iana),
    };
    glint_corpus_build::run(&settings)?;
    let dump = std::fs::read_to_string(&findings_file)
        .map_err(|e| format!("{}: {e}", findings_file.display()))?;
    let mut findings: Vec<Finding> = dump
        .lines()
        .filter_map(|line| {
            let mut fields = line.splitn(5, '\t');
            let (document, module, code, _line, message) = (
                fields.next()?,
                fields.next()?,
                fields.next()?,
                fields.next()?,
                fields.next()?,
            );
            counts(code).then(|| {
                (
                    document.to_owned(),
                    module.to_owned(),
                    code.to_owned(),
                    message.to_owned(),
                )
            })
        })
        .collect();
    let corpus = load_built(&out)?;
    let document = format!("rfc{}", source.rfc);
    let report = check_document(
        &document,
        text,
        extract_format(source.format),
        &corpus,
        &CheckOptions::default(),
    );
    findings.extend(
        report
            .diagnostics
            .iter()
            .filter(|d| SNIPPET_CODES.contains(&d.code) && d.severity != Severity::Info)
            .map(|d| {
                (
                    document.clone(),
                    "(inline)".to_owned(),
                    d.code.to_owned(),
                    d.message.clone(),
                )
            }),
    );
    Ok((findings, corpus))
}

fn counts(code: &str) -> bool {
    code.starts_with('E') || code.starts_with('W')
}

/// Links `to` to `from`, or copies it.  The link's target is made absolute: a relative one would resolve from the
/// link's directory, not from where `from` was named (`--cache-dir corpus/cache`, the default), and dangle.
fn link_or_copy(from: &Path, to: &Path) -> Result<(), String> {
    let from = std::path::absolute(from).map_err(|e| format!("{}: {e}", from.display()))?;
    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(&from, to);
    #[cfg(not(unix))]
    let linked = std::fs::copy(&from, to).map(|_| ());
    linked.map_err(|e| format!("{}: {e}", to.display()))
}

fn load_built(out: &Path) -> Result<Corpus, String> {
    let read = |p: PathBuf| std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()));
    let pointer = Pointer::from_json(&read(out.join(latest_path()))?).map_err(|e| e.to_string())?;
    let data = CorpusData::from_json(&read(
        out.join(build_dir(&pointer.build))
            .join(glint_corpus::CORPUS_FILE),
    )?)
    .map_err(|e| e.to_string())?;
    Ok(Corpus::from_data(data))
}

fn tally(findings: &[Finding]) -> BTreeMap<&Finding, usize> {
    let mut tally = BTreeMap::new();
    for f in findings {
        let n = tally.entry(f).or_insert(0usize);
        *n = n.saturating_add(1);
    }
    tally
}

/// The findings in `a` more often than in `b`.
fn more_in<'a>(
    a: &BTreeMap<&'a Finding, usize>,
    b: &BTreeMap<&'a Finding, usize>,
) -> Vec<&'a Finding> {
    a.iter()
        .filter(|(f, n)| **n > b.get(**f).copied().unwrap_or(0))
        .map(|(f, _)| *f)
        .collect()
}

fn prove(paths: &Paths, work_file: &Path) -> Result<ExitCode, String> {
    let name = work_file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let captures = WORK_NAME
        .as_ref()
        .and_then(|re| re.captures(&name))
        .ok_or_else(|| {
            format!(
                "{name} is not a work file name, rfcN-ID.txt or rfcN-ID.xml; make it with `edit`"
            )
        })?;
    let number = |i: usize| {
        captures
            .get(i)
            .and_then(|m| ascii_number(m.as_str()).ok())
            .unwrap_or_default()
    };
    let (rfc, errata_id) = (number(1), number(2));
    let rows = load_rows(paths)?;
    let (base, source) = base_for(paths, &rows, rfc, errata_id)?;
    let edited = read_work(paths, rfc, errata_id, &base, work_file)?;
    let later: Vec<(u32, String)> = series_of(paths, &rows, rfc)
        .map_err(|f| format!("rfc{rfc} erratum {}: {}", f.errata_id, f.reason))?
        .into_iter()
        .filter(|(id, _)| *id > errata_id)
        .collect();
    let (before, after, applied, notes) = apply_later(base, edited, &later, source);
    // In the work directory, which is the user's alone, so nobody else can plant files or links in the builds.
    work_dir(paths)?;
    let scratch = paths.work.join(format!("prove-{}", std::process::id()));
    let sides = build_side(paths, source, &before, &rows, &scratch.join("before")).and_then(|b| {
        Ok((
            b,
            build_side(paths, source, &after, &rows, &scratch.join("after"))?,
        ))
    });
    let _ = std::fs::remove_dir_all(&scratch);
    let ((old, old_corpus), (new, new_corpus)) = sides?;
    let (old_tally, new_tally) = (tally(&old), tally(&new));
    let losses = more_in(&new_tally, &old_tally);
    let gains = more_in(&old_tally, &new_tally);
    println!(
        "prove rfc{rfc} erratum {errata_id} ({})",
        source.file_name()
    );
    if !applied.is_empty() {
        let ids: Vec<String> = applied.iter().map(u32::to_string).collect();
        println!(
            "- higher patched errata applied to both sides: {}",
            ids.join(", ")
        );
    }
    for note in &notes {
        println!("- WARNING: {note}");
    }
    let document = format!("rfc{rfc}");
    let modules = |corpus: &Corpus| -> BTreeSet<String> {
        corpus
            .modules()
            .iter()
            .filter(|m| m.source.document == document)
            .map(|m| m.name.clone())
            .collect()
    };
    let (old_modules, new_modules) = (modules(&old_corpus), modules(&new_corpus));
    println!(
        "- rfc{rfc} modules: {}",
        new_modules.iter().cloned().collect::<Vec<_>>().join(", ")
    );
    for gone in old_modules.difference(&new_modules) {
        println!("- module {gone} is gone after the edit");
    }
    for module in new_modules
        .iter()
        .chain(std::iter::once(&"(inline)".to_owned()))
    {
        let count = |findings: &[Finding]| {
            findings
                .iter()
                .filter(|f| f.0 == document && &f.1 == module)
                .count()
        };
        let (b, a) = (count(&old), count(&new));
        if b > 0 || a > 0 || module != "(inline)" {
            println!("  {module}: {b} errors and warnings before, {a} after");
        }
    }
    for (label, plural, list) in [("loss", "losses", &losses), ("gain", "gains", &gains)] {
        println!(
            "{} {}:",
            list.len(),
            if list.len() == 1 { label } else { plural }
        );
        for (document, module, code, message) in list.iter() {
            println!(
                "  {label} {document} {module} {code}: {}",
                escape_line(message)
            );
        }
    }
    if losses.is_empty() {
        println!("no new errors or warnings");
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::from(1))
    }
}

// -- record ----------------------------------------------------------------------------------------------------------

/// Prints `FLAG:` when a hunk is far from every place the erratum's text was found, or when it was not found.
fn flag_far(base: &str, patch: &str, source: Source, e: &Erratum) {
    let (matches, _) = located(base, e);
    if matches.is_empty() {
        println!(
            "FLAG: the erratum text was not located in the RFC; check the patch is in the right place"
        );
        return;
    }
    let distance = |line: usize, m: &Match| {
        if (m.first..=m.last).contains(&line) {
            0
        } else {
            line.abs_diff(m.first).min(line.abs_diff(m.last))
        }
    };
    let starts = changed_lines(patch, source, base).unwrap_or_default();
    let far: Vec<String> = starts
        .into_iter()
        .filter(|&s| matches.iter().map(|m| distance(s, m)).min().unwrap_or(0) > FAR_LINES)
        .map(|s| s.to_string())
        .collect();
    if !far.is_empty() {
        let places: Vec<String> = matches
            .iter()
            .map(|m| format!("{}-{}", m.first, m.last))
            .collect();
        println!(
            "FLAG: hunks at lines {} are more than {FAR_LINES} lines from the located erratum text (lines {}); name this in the summary for the reviewer",
            far.join(", "),
            places.join(", ")
        );
    }
}

/// Writes the patches of `rebased` that differ from `series`, and reports a rebase failure.
fn write_rebased(
    paths: &Paths,
    rfc: u32,
    skip: u32,
    series: &[(u32, String)],
    rebased: &glint_errata::RebaseOutcome,
) -> Result<(), String> {
    let old: BTreeMap<u32, &String> = series.iter().map(|(id, p)| (*id, p)).collect();
    for (id, patch) in rebased.patches.iter().filter(|(id, _)| *id != skip) {
        if old.get(id).is_none_or(|p| *p != patch) {
            store::write_patch(&paths.errata, rfc, *id, patch).map_err(store_error)?;
            println!("rebased the patch of rfc{rfc} erratum {id}");
        }
    }
    if let Some(failure) = &rebased.failure {
        println!(
            "WARNING: the patch of rfc{rfc} erratum {} no longer applies ({}); it and every higher patched erratum of rfc{rfc} are queued as patch-fails",
            failure.errata_id, failure.reason
        );
    }
    Ok(())
}

fn record(
    paths: &Paths,
    rfc: u32,
    errata_id: u32,
    decision: Decision,
    note: &str,
    by: Decider,
) -> Result<ExitCode, String> {
    let feed = load_feed(paths)?;
    let rows = load_rows(paths)?;
    let erratum = get_erratum(&feed, rfc, errata_id)?;
    if decision == Decision::Patched && !erratum.is_some_and(Erratum::is_verified) {
        return Err(format!(
            "erratum {errata_id} is not Verified; only Verified errata are patched"
        ));
    }
    if let Some(problem) =
        note_problem(note, decision).or_else(|| erratum.and_then(|e| screen_note(note, e)))
    {
        return Err(problem);
    }
    if decision == Decision::Superseded
        && let Some(later) = superseding_rfc(note)
        && rfc_source(&paths.rfcs, later).is_none()
    {
        println!(
            "WARNING: rfc{later} is not in {}; check the RFC number",
            paths.rfcs.display()
        );
    }
    let row = Row {
        rfc,
        errata_id,
        decision,
        by,
        fingerprint: erratum.map_or(MISSING_FINGERPRINT.to_owned(), fingerprint),
        rule: Rule::Asn1,
        note: note.to_owned(),
    };
    let mut new_rows: Vec<Row> = rows
        .iter()
        .filter(|r| r.key() != row.key())
        .cloned()
        .collect();
    new_rows.push(row);
    let was_patched = rows
        .iter()
        .any(|r| r.key() == (rfc, errata_id) && r.decision == Decision::Patched);
    let series = series_of(paths, &rows, rfc)
        .map_err(|f| format!("rfc{rfc} erratum {}: {}", f.errata_id, f.reason))?;
    if decision == Decision::Patched {
        let (base, source) = base_for(paths, &rows, rfc, errata_id)?;
        let edited = read_work(
            paths,
            rfc,
            errata_id,
            &base,
            &paths.work_file(source, errata_id),
        )?;
        let patch =
            make_patch(source, &base, &edited).map_err(|e| format!("the work file: {e}"))?;
        if let Some(bad) = forbidden_in_patch(&patch) {
            return Err(format!("the patch {bad}"));
        }
        let (published, _) = published(paths, rfc)?;
        let rebased = rebase(&published, &series, errata_id, Some(&patch), source)
            .map_err(|f| format!("rfc{rfc} erratum {}: {}", f.errata_id, f.reason))?;
        // The patches first, then the row: the row is the commit point.
        store::write_patch(&paths.errata, rfc, errata_id, &patch).map_err(store_error)?;
        write_rebased(paths, rfc, errata_id, &series, &rebased)?;
        store::write_ledger(&paths.errata, &new_rows).map_err(store_error)?;
        let hunks = parse_patch(&patch, source)
            .map(|h| h.len())
            .unwrap_or_default();
        println!(
            "recorded rfc{rfc} {errata_id} patched: {} ({hunks} hunk{})",
            store::patch_path(&paths.errata, rfc, errata_id).display(),
            if hunks == 1 { "" } else { "s" }
        );
        if let Some(e) = erratum {
            flag_far(&base, &patch, source, e);
        }
    } else {
        // The row first, then the patches: an interrupted back-out never leaves a patch that is applied.
        store::write_ledger(&paths.errata, &new_rows).map_err(store_error)?;
        if was_patched {
            match published(paths, rfc) {
                Ok((text, source)) => {
                    let rebased = rebase(&text, &series, errata_id, None, source)
                        .map_err(|f| format!("rfc{rfc} erratum {}: {}", f.errata_id, f.reason))?;
                    write_rebased(paths, rfc, errata_id, &series, &rebased)?;
                }
                Err(message) => println!("WARNING: higher patches not rebased: {message}"),
            }
            store::remove_patch(&paths.errata, rfc, errata_id).map_err(store_error)?;
            println!("removed the patch of rfc{rfc} erratum {errata_id}");
        }
        println!("recorded rfc{rfc} {errata_id} {decision}");
    }
    Ok(ExitCode::SUCCESS)
}

// -- check -----------------------------------------------------------------------------------------------------------

fn check(paths: &Paths) -> Result<ExitCode, String> {
    let mut problems: Vec<String> = Vec::new();
    let rows = match load_rows(paths) {
        Ok(rows) => rows,
        Err(message) => {
            println!("ERROR: {message}");
            return Ok(ExitCode::from(1));
        }
    };
    let (files, scan_problems) = store::scan_patches(&paths.errata);
    problems.extend(scan_problems);
    let patched: BTreeSet<(u32, u32)> = rows
        .iter()
        .filter(|r| r.decision == Decision::Patched)
        .map(Row::key)
        .collect();
    for key in patched.iter().filter(|k| !files.contains_key(k)) {
        problems.push(format!(
            "rfc{} {} is patched but has no patch file",
            key.0, key.1
        ));
    }
    for path in files
        .iter()
        .filter(|(k, _)| !patched.contains(k))
        .map(|(_, p)| p)
    {
        problems.push(format!(
            "{} has no `patched` ledger row (record the erratum again, or delete it)",
            path.display()
        ));
    }
    let feed = load_feed(paths).ok();
    for rfc in patched
        .iter()
        .map(|(rfc, _)| *rfc)
        .collect::<BTreeSet<u32>>()
    {
        let Ok((text, source)) = published(paths, rfc) else {
            problems.push(format!(
                "rfc{rfc} has patched errata but is not in {}",
                paths.rfcs.display()
            ));
            continue;
        };
        let series = match series_of(paths, &rows, rfc) {
            Ok(series) => series,
            Err(f) => {
                problems.push(format!("rfc{rfc} erratum {}: {}", f.errata_id, f.reason));
                continue;
            }
        };
        for (id, patch) in &series {
            if store::patch_format(patch) != Some(source.format) {
                problems.push(format!(
                    "rfc{rfc} erratum {id}: the patch is not against {}, which Glint reads",
                    source.file_name()
                ));
            }
            if let Err(e) = parse_patch(patch, source) {
                problems.push(format!("rfc{rfc} erratum {id}: {e}"));
            }
            if let Some(bad) = forbidden_in_patch(patch) {
                problems.push(format!("rfc{rfc} erratum {id}: {bad}"));
            }
        }
        if let Err(f) = apply_series(&text, &series, source) {
            problems.push(format!(
                "rfc{rfc}: the patch of erratum {} does not apply ({})",
                f.errata_id, f.reason
            ));
        }
    }
    for row in &rows {
        if row.decision == Decision::Superseded
            && let Some(later) = superseding_rfc(&row.note)
            && rfc_source(&paths.rfcs, later).is_none()
        {
            problems.push(format!(
                "rfc{} {}: superseded by rfc{later}, which is not in {}",
                row.rfc,
                row.errata_id,
                paths.rfcs.display()
            ));
        }
        let Some(feed) = &feed else { continue };
        let erratum = feed.errata.get(&row.errata_id);
        if let Some(e) = erratum
            && screen_note(&row.note, e).is_some()
        {
            problems.push(format!(
                "rfc{} {}: the note holds a name",
                row.rfc, row.errata_id
            ));
        }
        if row.decision == Decision::Patched
            && !erratum.is_some_and(|e| e.is_verified() && e.rfc == row.rfc)
        {
            problems.push(format!(
                "rfc{} {} is patched but is not a Verified erratum of rfc{} (withdrawn?)",
                row.rfc, row.errata_id, row.rfc
            ));
        }
    }
    for problem in &problems {
        println!("ERROR: {problem}");
    }
    let summary = if problems.is_empty() {
        "OK".to_owned()
    } else {
        format!("{} problem(s)", problems.len())
    };
    println!(
        "check: {} ledger rows, {} patched, {summary}",
        rows.len(),
        patched.len()
    );
    Ok(if problems.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}
