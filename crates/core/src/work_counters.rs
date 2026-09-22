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
//! Relaxed atomics behind the existing trace seam: incrementing three
//! `AtomicU64`s per directory listing is far below the cost of the
//! listing itself, and nothing reads them except tests and
//! `swamp report --json`'s optional work block.

use std::sync::atomic::{AtomicU64, Ordering};

static DIRS_LISTED: AtomicU64 = AtomicU64::new(0);
static FILES_STATTED: AtomicU64 = AtomicU64::new(0);
static HEADER_BYTES: AtomicU64 = AtomicU64::new(0);
static CACHE_HITS: AtomicU64 = AtomicU64::new(0);
static CACHE_MISSES: AtomicU64 = AtomicU64::new(0);

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
    DIRS_LISTED.fetch_add(1, Ordering::Relaxed);
}

pub fn record_files_statted(n: u64) {
    FILES_STATTED.fetch_add(n, Ordering::Relaxed);
}

pub fn record_header_bytes(n: u64) {
    HEADER_BYTES.fetch_add(n, Ordering::Relaxed);
}

pub fn record_cache_hit() {
    CACHE_HITS.fetch_add(1, Ordering::Relaxed);
}

pub fn record_cache_miss() {
    CACHE_MISSES.fetch_add(1, Ordering::Relaxed);
}

pub fn snapshot() -> WorkCounters {
    WorkCounters {
        dirs_listed: DIRS_LISTED.load(Ordering::Relaxed),
        files_statted: FILES_STATTED.load(Ordering::Relaxed),
        header_bytes_read: HEADER_BYTES.load(Ordering::Relaxed),
        identification_cache_hits: CACHE_HITS.load(Ordering::Relaxed),
        identification_cache_misses: CACHE_MISSES.load(Ordering::Relaxed),
    }
}

/// Zeroes every counter. Tests call this immediately before the pass
/// they are measuring; nothing in production resets them.
pub fn reset() {
    DIRS_LISTED.store(0, Ordering::Relaxed);
    FILES_STATTED.store(0, Ordering::Relaxed);
    HEADER_BYTES.store(0, Ordering::Relaxed);
    CACHE_HITS.store(0, Ordering::Relaxed);
    CACHE_MISSES.store(0, Ordering::Relaxed);
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
