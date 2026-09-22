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
//! What it does **not** do yet is reuse the folded directory rows the
//! walk already persisted for an unchanged root; that is the remaining
//! half of the incrementality repair and is recorded as such in
//! `.oh/sessions/2026-09-21-foundation-repairs.md` rather than implied
//! by this module's existence.

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
}

pub fn measure(path: &Path, exclusions: &[PathBuf], observed_at: u64) -> FoldedUnit {
    let row = crate::walk::resize_artifact_excluding(
        path,
        ArtifactKind::Unknown,
        observed_at,
        exclusions,
    );
    crate::work_counters::record_cache_miss();
    FoldedUnit {
        bytes: row.bytes,
        hardlinked: row.hardlinked,
    }
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
        let before = crate::work_counters::snapshot();
        let folded = measure(tmp.path(), &[], 1_000);
        assert!(folded.bytes >= 5);
        assert!(crate::work_counters::since(before).identification_cache_misses >= 1);
    }
}
