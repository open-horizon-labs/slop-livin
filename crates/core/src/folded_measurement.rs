//! The one place a *unit* (an external storage location, a tool home) is
//! turned into bytes.
//!
//! The 2026-09-21 review's P1 on non-incremental traversal: `external.rs`
//! recursively re-sized every external root on every call, and the agent
//! layer added a second recursive traversal of its own. Both are now
//! funnelled through here, so there is exactly one implementation to
//! make incremental and exactly one place the work counters live
//! (`.oh/guardrails/no-second-traversal-on-report-path.md` allow-lists
//! this module and forbids the callers).
//!
//! What this module does today, honestly:
//!
//! * it owns the readability probe and the folded measurement, so
//!   `external.rs` and `agents/**` contain no directory traversal at
//!   all; and
//! * it records the work each measurement actually did
//!   (`crate::work_counters`), which is what makes "unchanged work is
//!   cheap" a testable claim rather than an assertion.
//!
//! Since 2026-09-22 it also **reuses** the folded rows a previous pass
//! persisted, which is the other half of the incrementality repair. See
//! [`reuse_folded_measurement`] for what the reuse is allowed to
//! conclude and, just as importantly, what it cannot see.

use crate::report::ArtifactKind;
use std::path::{Path, PathBuf};

/// Whether a unit can be measured this pass at all, distinguishing the
/// three outcomes that must never be collapsed: gone, present but
/// unreadable, and measurable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnitAccess {
    /// Nothing at this path. A real disappearance, which the owning
    /// observation's sweep may tombstone.
    Absent,
    /// The path exists but this pass could not read it (permission on
    /// the path or a parent). Coverage is incomplete -- never a
    /// disappearance, never a zero.
    Unreadable(String),
    Measurable,
}

/// One `symlink_metadata` plus, for a directory, one listing attempt.
/// The listing is the only directory read in the external/agent
/// measurement path, and it exists to tell "unreadable" from "empty".
pub fn access(path: &Path) -> UnitAccess {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => UnitAccess::Absent,
        Err(e) => UnitAccess::Unreadable(e.to_string()),
        Ok(meta) if meta.is_dir() => {
            crate::work_counters::record_dir_listed();
            match std::fs::read_dir(path) {
                Ok(_) => UnitAccess::Measurable,
                // `stat` succeeds (reaching the path itself needs only
                // the *parent's* execute bit) while the directory's own
                // contents cannot be listed, e.g. `chmod 000`.
                Err(e) => UnitAccess::Unreadable(e.to_string()),
            }
        }
        Ok(_) => UnitAccess::Measurable,
    }
}

/// A unit's folded byte total, excluding any nested locations that are
/// separately measured as their own units (so the same bytes are never
/// counted twice).
pub struct FoldedUnit {
    pub bytes: u64,
    pub hardlinked: bool,
    /// Newest recorded modification among the measured children, from
    /// the same folded walk. Carried so an external unit can have an
    /// Activity fact without a second pass: `docs/usage.md` claimed
    /// modification age for external locations while `ExternalUnit` had
    /// no `mtime_max` field at all (the 2026-09-22 re-review).
    pub mtime_max: u64,
}

/// Measures `path`, reusing the previous pass's folded rows when this
/// pass can show they still describe the tree.
///
/// `store` is the swamp store directory; `None` means "no history to
/// reuse and nowhere to record this measurement", which is what a
/// store-less caller (a one-shot `--root` report, most unit tests) gets.
pub fn measure(
    store: Option<&Path>,
    path: &Path,
    exclusions: &[PathBuf],
    observed_at: u64,
) -> FoldedUnit {
    if let Some(folded) = reuse_folded_measurement(store, path, exclusions) {
        crate::work_counters::record_cache_hit();
        return folded;
    }
    let (row, _dirs, stamps) = crate::walk::resize_artifact_stamped(
        path,
        ArtifactKind::Unknown,
        observed_at,
        None,
        exclusions,
        store.is_some(),
    );
    crate::work_counters::record_cache_miss();
    let folded = FoldedUnit {
        bytes: row.bytes,
        hardlinked: row.hardlinked,
        mtime_max: row.mtime_max,
    };
    if let Some(dir) = store {
        record_folded_measurement(dir, path, exclusions, observed_at, &folded, &stamps);
    }
    folded
}

/// The exclusion set a stored measurement was taken under, as one
/// string. A measurement taken while a nested location was excluded
/// describes different bytes than one taken without it, so a changed
/// exclusion set is a cache miss rather than a silent wrong answer
/// (`.oh/guardrails/coverage-changes-are-not-storage-changes.md` is the
/// other half of this: the miss re-measures, it never invents a delta).
fn exclusions_digest(exclusions: &[PathBuf]) -> String {
    let mut parts: Vec<String> = exclusions.iter().map(|p| p.display().to_string()).collect();
    parts.sort();
    parts.join("\u{1}")
}

/// The previous pass's folded measurement of `path`, if every directory
/// it listed still carries the same `mtime`/`ctime` it did then.
///
/// **Cost.** One `stat` per directory the unit contains and *no*
/// directory listing at all -- the whole point. A 20,000-file Cargo
/// registry cache costs four stats on an unchanged pass instead of
/// 20,000 stats and four listings.
///
/// **What this detects.** Creating, deleting, renaming or replacing any
/// entry in a directory moves that directory's own `mtime` and `ctime`,
/// so any change to the *shape* of the tree -- including a new
/// subdirectory, whose parent is stamped -- is a miss. A directory that
/// has been removed fails its `stat` and is a miss. A changed exclusion
/// set is a miss.
///
/// **What this cannot see**, stated plainly because a cache that
/// oversells itself is worse than none: a file rewritten *in place*
/// (same name, same directory) does not move its directory's stamp. If
/// such a rewrite also changes the file's allocation -- a truncation, a
/// hole punched, a sparse file grown -- the reused byte total is stale
/// until something else in that directory changes or the store is
/// cleared. The alternative is stat'ing every file on every pass, which
/// is the cost this exists to remove, and is exactly the "scales with
/// all files" shape the handoff forbids.
pub fn reuse_folded_measurement(
    store: Option<&Path>,
    path: &Path,
    exclusions: &[PathBuf],
) -> Option<FoldedUnit> {
    let dir = store?;
    let unit_path = path.display().to_string();
    let rows = crate::growth::folded_rows_for(dir, &unit_path);
    let root = rows.iter().find(|r| r.rel_dir.is_empty())?;
    if root.exclusions != exclusions_digest(exclusions) {
        return None;
    }
    for row in &rows {
        let dir_path = if row.rel_dir.is_empty() {
            path.to_path_buf()
        } else {
            path.join(&row.rel_dir)
        };
        crate::work_counters::record_files_statted(1);
        let meta = std::fs::symlink_metadata(&dir_path).ok()?;
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return None;
        }
        if stamp_ns(&meta) != (row.mtime_ns, row.ctime_ns) {
            return None;
        }
    }
    Some(FoldedUnit {
        bytes: root.bytes,
        hardlinked: root.hardlinked,
        mtime_max: root.mtime_max,
    })
}

fn stamp_ns(meta: &std::fs::Metadata) -> (i64, i64) {
    use std::os::unix::fs::MetadataExt;
    (
        meta.mtime() * 1_000_000_000 + meta.mtime_nsec(),
        meta.ctime() * 1_000_000_000 + meta.ctime_nsec(),
    )
}

fn record_folded_measurement(
    store: &Path,
    path: &Path,
    exclusions: &[PathBuf],
    observed_at: u64,
    folded: &FoldedUnit,
    stamps: &[crate::walk::DirStamp],
) {
    let unit_path = path.display().to_string();
    let digest = exclusions_digest(exclusions);
    let mut rows: Vec<crate::growth::FoldedRow> = Vec::with_capacity(stamps.len());
    let mut saw_root = false;
    for stamp in stamps {
        let rel = match stamp.path.strip_prefix(path) {
            Ok(r) => r.display().to_string(),
            // A stamp from outside the unit cannot be validated against
            // the unit root later, so it is not stored -- and, since a
            // directory the measurement listed would then go unwatched,
            // the whole measurement is not stored either.
            Err(_) => return,
        };
        saw_root |= rel.is_empty();
        rows.push(crate::growth::FoldedRow {
            unit_path: unit_path.clone(),
            rel_dir: rel,
            mtime_ns: stamp.mtime_ns,
            ctime_ns: stamp.ctime_ns,
            bytes: 0,
            hardlinked: false,
            mtime_max: 0,
            observed_at,
            exclusions: digest.clone(),
        });
    }
    // No root stamp means the root itself was never listed (an excluded
    // or unreadable root): there is nothing to anchor a later reuse on.
    if !saw_root {
        return;
    }
    for row in rows.iter_mut().filter(|r| r.rel_dir.is_empty()) {
        row.bytes = folded.bytes;
        row.hardlinked = folded.hardlinked;
        row.mtime_max = folded.mtime_max;
    }
    // A cache write that fails is a cache that will miss next time,
    // which is the correct outcome and not worth failing a report over.
    let _ = crate::growth::store_folded_rows(store, &unit_path, &rows);
}

/// Access and measurement in one call, so the ordinary report path never
/// pays for the readability probe on a unit whose stored measurement is
/// still good: [`reuse_folded_measurement`] answers from stats alone,
/// and only a miss reaches [`access`]'s listing.
pub enum UnitObservation {
    Absent,
    Unreadable(String),
    Unit(FoldedUnit),
}

pub fn observe_unit(
    store: Option<&Path>,
    path: &Path,
    exclusions: &[PathBuf],
    observed_at: u64,
) -> UnitObservation {
    if let Some(folded) = reuse_folded_measurement(store, path, exclusions) {
        crate::work_counters::record_cache_hit();
        return UnitObservation::Unit(folded);
    }
    match access(path) {
        UnitAccess::Absent => UnitObservation::Absent,
        UnitAccess::Unreadable(why) => UnitObservation::Unreadable(why),
        UnitAccess::Measurable => {
            UnitObservation::Unit(measure(store, path, exclusions, observed_at))
        }
    }
}

/// Bounded, stat-only folded byte total for `path` (file or directory),
/// returning `(bytes, mtime_max, truncated)`.
///
/// This is deliberately *not* [`measure`]'s parallel-pool machinery:
/// that is tuned for a handful of potentially huge artifact roots, not
/// hundreds of small per-session directories, and spinning up its thread
/// pool that many times would itself be the "unacceptable scanning cost"
/// the agent epic guards against. Reads directory names and `stat` calls
/// only -- never file contents. Bounded by `max_entries`; a directory
/// that hits the bound is reported truncated rather than silently
/// under-measured.
///
/// It lives here rather than in `agents/mod.rs` because this module is
/// the one place allowed to traverse on the ordinary report path
/// (`.oh/guardrails/no-second-traversal-on-report-path.md`); adapters
/// reach it through `agents::IdentifyCtx::folded_bytes`, never directly.
pub fn folded_bytes_bounded(path: &Path, max_entries: usize) -> (u64, u64, bool) {
    let (bytes, mtime_max, truncated, _stamps) = folded_bytes_bounded_stamped(path, max_entries);
    (bytes, mtime_max, truncated)
}

/// [`folded_bytes_bounded`] plus one [`crate::walk::DirStamp`] per
/// directory it listed, taken from the `stat` that listing already did.
///
/// This is the agent family's half of the bargain
/// [`crate::walk::resize_artifact_stamped`] strikes for the external
/// family: the pass that pays for a fold hands back exactly what a later
/// pass needs in order to decide, from `stat`s alone, whether that fold
/// still describes the tree. A *truncated* fold returns no stamps at
/// all -- a measurement that stopped at the bound does not describe the
/// whole subtree, so it must never anchor a reuse.
pub fn folded_bytes_bounded_stamped(
    path: &Path,
    max_entries: usize,
) -> (u64, u64, bool, Vec<crate::walk::DirStamp>) {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return (0, 0, false, Vec::new());
    };
    if meta.is_file() {
        return (meta.len(), mtime_secs(&meta), false, Vec::new());
    }
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return (0, 0, false, Vec::new());
    }
    let mut total = 0u64;
    let mut mtime_max = mtime_secs(&meta);
    let mut stack = vec![(path.to_path_buf(), meta)];
    let mut seen = 0usize;
    let mut truncated = false;
    let mut stamps: Vec<crate::walk::DirStamp> = Vec::new();
    while let Some((dir, dir_meta)) = stack.pop() {
        crate::work_counters::record_dir_listed();
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        let (mtime_ns, ctime_ns) = stamp_ns(&dir_meta);
        stamps.push(crate::walk::DirStamp {
            path: dir.clone(),
            mtime_ns,
            ctime_ns,
        });
        let mut here = 0u64;
        for entry in rd.flatten() {
            seen += 1;
            if seen > max_entries {
                truncated = true;
                break;
            }
            let Ok(m) = entry.metadata() else { continue };
            here += 1;
            mtime_max = mtime_max.max(mtime_secs(&m));
            if m.is_dir() && !m.file_type().is_symlink() {
                stack.push((entry.path(), m));
            } else if m.is_file() {
                total += m.len();
            }
        }
        crate::work_counters::record_files_statted(here);
        if truncated {
            break;
        }
    }
    if truncated {
        stamps.clear();
    }
    (total, mtime_max, truncated, stamps)
}

pub fn mtime_secs(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absent_path_is_absent_not_zero_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(access(&tmp.path().join("nope")), UnitAccess::Absent);
    }

    #[test]
    fn an_unlistable_directory_is_unreadable_not_empty() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("locked");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("f"), b"x").unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o000)).unwrap();
        let got = access(&dir);
        // Restore before asserting so the tempdir can always clean up.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        // Running as root defeats the permission bit; only assert the
        // distinction where the platform actually enforces it.
        if !matches!(got, UnitAccess::Measurable) {
            assert!(matches!(got, UnitAccess::Unreadable(_)), "{got:?}");
        }
    }

    #[test]
    fn a_measurable_directory_folds_its_bytes_and_counts_the_work() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a"), b"12345").unwrap();
        assert_eq!(access(tmp.path()), UnitAccess::Measurable);
        let (folded, counted) =
            crate::work_counters::measured(|| measure(None, tmp.path(), &[], 1_000));
        assert!(folded.bytes >= 5);
        assert!(counted.identification_cache_misses >= 1);
    }

    /// The reuse, end to end: a second measurement of an unchanged tree
    /// lists no directories at all, and a file added to a subdirectory
    /// is a miss.
    #[test]
    fn an_unchanged_unit_is_reused_without_listing_anything() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tempfile::tempdir().unwrap();
        let unit = tmp.path().join("cache");
        std::fs::create_dir_all(unit.join("a/b")).unwrap();
        std::fs::write(unit.join("a/b/f"), vec![b'x'; 4096]).unwrap();
        std::fs::write(unit.join("a/g"), vec![b'y'; 4096]).unwrap();

        let first = measure(Some(store.path()), &unit, &[], 1_000);
        assert!(first.bytes >= 8192, "{}", first.bytes);

        let (second, cost) =
            crate::work_counters::measured(|| measure(Some(store.path()), &unit, &[], 2_000));
        assert_eq!(second.bytes, first.bytes);
        assert_eq!(
            cost.dirs_listed, 0,
            "an unchanged unit must be answered without listing a directory"
        );
        assert_eq!(
            cost.identification_cache_hits, 1,
            "the answer must come from the stored rows, not a re-walk"
        );

        std::fs::write(unit.join("a/b/new"), vec![b'z'; 4096]).unwrap();
        let (third, cost) =
            crate::work_counters::measured(|| measure(Some(store.path()), &unit, &[], 3_000));
        assert!(third.bytes > second.bytes, "a new file must be measured");
        assert!(
            cost.dirs_listed > 0,
            "a changed directory must be re-listed"
        );
    }

    /// A different exclusion set describes different bytes, so it must
    /// not be answered from a measurement taken under the old one.
    #[test]
    fn a_changed_exclusion_set_is_a_miss() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tempfile::tempdir().unwrap();
        let unit = tmp.path().join("cache");
        std::fs::create_dir_all(unit.join("nested")).unwrap();
        std::fs::write(unit.join("nested/f"), vec![b'x'; 8192]).unwrap();
        let all = measure(Some(store.path()), &unit, &[], 1_000);
        let excluded = measure(Some(store.path()), &unit, &[unit.join("nested")], 2_000);
        assert!(
            excluded.bytes < all.bytes,
            "excluding the only populated subtree must measure fewer bytes, got {} vs {}",
            excluded.bytes,
            all.bytes
        );
    }
}
