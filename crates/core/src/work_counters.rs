//! Cheap process-global counters for the work an observation actually
//! did: directories listed, files statted, session-header bytes read.
//!
//! These exist because "incremental" is otherwise unfalsifiable. The
//! handoff requires unchanged work to scale with roots and changed
//! containers rather than with all files; the only way to *test* that is
//! to count the syscalls an unchanged pass makes and assert it is zero.
//! `crates/core/tests/incremental_external_and_agent_measurement.rs`
//! does exactly that.
//!
//! The counters are **per thread**. They were process-global
//! `AtomicU64`s, which made every assertion in this crate's own tests a
//! race: `cargo test` runs the lib tests in parallel threads, so one
//! adapter's `header_bytes_read == 0` could be falsified by a different
//! adapter's fixture reading a header at the same moment. An
//! intermittently wrong measurement is worse than no measurement.
//!
//! Per-thread is also the *right* scope for what these measure:
//! identification, folded measurement and the bounded listings all run
//! on the caller's own thread, and a caller asking "what did my pass
//! cost" means its own pass. Incrementing a `Cell<u64>` is cheaper than
//! an atomic, and nothing reads these except tests and `swamp report
//! --json`'s optional work block.

use std::cell::Cell;

thread_local! {
    static DIRS_LISTED: Cell<u64> = const { Cell::new(0) };
    static FILES_STATTED: Cell<u64> = const { Cell::new(0) };
    static HEADER_BYTES: Cell<u64> = const { Cell::new(0) };
    static CACHE_HITS: Cell<u64> = const { Cell::new(0) };
    static CACHE_MISSES: Cell<u64> = const { Cell::new(0) };
}

fn add(counter: &'static std::thread::LocalKey<Cell<u64>>, n: u64) {
    counter.with(|c| c.set(c.get().saturating_add(n)));
}

/// A snapshot of the work counters, for a test or a `--json` work block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct WorkCounters {
    pub dirs_listed: u64,
    pub files_statted: u64,
    pub header_bytes_read: u64,
    pub identification_cache_hits: u64,
    pub identification_cache_misses: u64,
}

pub fn record_dir_listed() {
    add(&DIRS_LISTED, 1);
}

pub fn record_files_statted(n: u64) {
    add(&FILES_STATTED, n);
}

pub fn record_header_bytes(n: u64) {
    add(&HEADER_BYTES, n);
}

pub fn record_cache_hit() {
    add(&CACHE_HITS, 1);
}

pub fn record_cache_miss() {
    add(&CACHE_MISSES, 1);
}

pub fn snapshot() -> WorkCounters {
    WorkCounters {
        dirs_listed: DIRS_LISTED.with(Cell::get),
        files_statted: FILES_STATTED.with(Cell::get),
        header_bytes_read: HEADER_BYTES.with(Cell::get),
        identification_cache_hits: CACHE_HITS.with(Cell::get),
        identification_cache_misses: CACHE_MISSES.with(Cell::get),
    }
}

/// Zeroes this thread's counters. Tests call this immediately before the
/// pass they are measuring; nothing in production resets them.
pub fn reset() {
    for c in [
        &DIRS_LISTED,
        &FILES_STATTED,
        &HEADER_BYTES,
        &CACHE_HITS,
        &CACHE_MISSES,
    ] {
        c.with(|c| c.set(0));
    }
}

/// The work done between `before` and now.
pub fn since(before: WorkCounters) -> WorkCounters {
    let now = snapshot();
    WorkCounters {
        dirs_listed: now.dirs_listed.saturating_sub(before.dirs_listed),
        files_statted: now.files_statted.saturating_sub(before.files_statted),
        header_bytes_read: now
            .header_bytes_read
            .saturating_sub(before.header_bytes_read),
        identification_cache_hits: now
            .identification_cache_hits
            .saturating_sub(before.identification_cache_hits),
        identification_cache_misses: now
            .identification_cache_misses
            .saturating_sub(before.identification_cache_misses),
    }
}
