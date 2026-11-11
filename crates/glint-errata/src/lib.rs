//! RFC errata: the RFC Editor's feed, the triage ledger, the per-erratum patches to an RFC's source, and the triage
//! queue.
//!
//! A Verified erratum that corrects ASN.1 is recorded as a patch to the document Glint extracts from (`rfcN.xml` when
//! the RFC has RFCXML, `rfcN.txt` otherwise). An RFC's patches apply in erratum-id order, each against the published
//! document plus the RFC's lower-numbered patched errata, and the series applies all or nothing. The applier ignores
//! line numbers: each hunk's old text must occur exactly once, so a patch never lands on the wrong copy of a repeated
//! definition. [`make_patch`] widens a patch's context until that holds.
//!
//! Everything but [`store`] is free of I/O.

mod chars;
mod feed;
mod fingerprint;
mod ledger;
mod locate;
mod patch;
mod queue;
pub mod store;

pub use chars::{feed_forbidden, line_forbidden};
pub use feed::{Erratum, Feed, FeedError, MAX_FEED_BYTES, check_plausible, parse_feed};
pub use fingerprint::fingerprint;
pub use ledger::{
    Decider, Decision, LEDGER_HEADER, LedgerError, MAX_NOTE_CHARS, Row, Rule, format_ledger,
    note_problem, parse_ledger, screen_note, superseding_rfc,
};
pub use locate::{Match, locate_all};
pub use patch::{
    Hunk, MAX_HUNKS, MAX_PATCH_BYTES, PatchError, RebaseOutcome, SeriesFailure, Source,
    SourceFormat, apply_patch, apply_series, changed_lines, forbidden_in_patch, make_patch,
    parse_patch, rebase,
};
pub use queue::{
    Queue, QueueItem, QueueReason, Scope, asn1_relevant, compute_queue, compute_scope, failing_keys,
};
