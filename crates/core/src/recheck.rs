//! The one live-state recheck model every destructive sink shares
//! (`.oh/guardrails/execution-sinks-recheck-live-state.md`).
//!
//! The 2026-09-21 independent review found three separate ways an
//! approved plan could spend its authorization on data nobody reviewed:
//!
//! * a directory renamed aside and replaced with unrelated content still
//!   satisfied `is_dir()`, so the replacement was moved under the old
//!   approval;
//! * a `swamp protect` entry added *after* approval was never consulted
//!   again, so protected data was moved;
//! * occupancy was probed on the unit's anchor path only, so a file held
//!   open *inside* a cache directory did not stop the parent's removal.
//!
//! The fix is one model, not three ad-hoc patches. Every function that
//! moves or removes user data calls all three of
//! [`reviewed_snapshot`] (identity + membership vs the plan),
//! [`live_protection`] (protection state loaded fresh, both directions),
//! and [`member_occupancy`] (every member, tri-state) *before* its first
//! destructive call. `crates/source-audit`'s
//! `execution_sinks_recheck_live_state` audit follows the call graph and
//! fails the build when a sink skips one or calls it too late.
//!
//! Strength differs by domain, the model does not: `cargo_cleanup`
//! additionally digests its members' *contents* (small, non-private
//! build outputs, under a held Cargo lock), which would be both
//! unaffordable and a privacy violation for a 20k-file model cache or a
//! transcript directory. Here identity is `(device, inode)` plus a
//! membership listing -- exact for bounded sets, a bounded summary plus a
//! metadata fingerprint for large ones. Nothing in this module ever
//! reads a file's contents.

use crate::occupancy::OccupancyState;
use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Member sets at or below this size are recorded exactly (one entry per
/// member path with its `(size, mtime, inode)`); larger ones fall back to
/// [`ReviewedMembership::Summary`]. 512 keeps a plan's JSON small while
/// covering every session/cache/log unit the adapters actually produce.
pub const EXACT_MEMBER_LIMIT: usize = 512;

/// Hard ceiling on how many entries one snapshot walks before it gives
/// up. A path that exceeds it is refused rather than half-reviewed --
/// "we looked at some of it" is not a reviewed identity.
pub const SNAPSHOT_ENTRY_LIMIT: usize = 2_000_000;

/// One member of a reviewed unit: metadata only, never contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewedMember {
    pub path: PathBuf,
    pub bytes: u64,
    pub mtime: u64,
    pub inode: u64,
}

/// A bounded stand-in for an exact listing when a unit has too many
/// members to enumerate in a plan (a large model/dependency cache).
/// `fingerprint` is a blake3 over the *sorted* `(path, size, mtime,
/// inode)` tuples, so an added, removed or rewritten member changes it
/// even when the counts happen to match.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewedSummary {
    pub entry_count: u64,
    pub allocated_bytes: u64,
    pub newest_mtime: u64,
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "membership", rename_all = "kebab-case")]
pub enum ReviewedMembership {
    /// The anchor's own identity only, with no member listing.
    ///
    /// Used for ordinary filesystem artifact rows, where enumerating a
    /// `target/` tree at *proposal* time would turn a cheap propose into
    /// a full traversal per matched row. The anchor's `(device, inode)`
    /// still catches the replaced-directory case, `lsof +D` still covers
    /// every descendant for occupancy, protection is still tested in
    /// both directions, and the execute path's pre-existing
    /// newest-mtime-versus-plan check still catches content churn. Agent
    /// and external units, whose member sets are bounded by
    /// construction, record their membership exactly.
    Anchor { bytes: u64, mtime: u64 },
    /// The unit is a single file (a transcript, a config file).
    File { bytes: u64, mtime: u64 },
    /// Every member, exactly.
    Exact { members: Vec<ReviewedMember> },
    /// Too many members to enumerate; bounded summary + fingerprint.
    Summary(ReviewedSummary),
}

/// What a human actually reviewed about one unit's storage: the anchor's
/// own `(device, inode)` -- so a replaced or renamed directory is caught
/// even when the path still exists and is still a directory -- plus its
/// membership.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewedIdentity {
    pub path: PathBuf,
    pub device: u64,
    pub inode: u64,
    pub is_dir: bool,
    pub membership: ReviewedMembership,
}

fn mtime_of(meta: &fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(unix)]
fn ids_of(meta: &fs::Metadata) -> (u64, u64) {
    use std::os::unix::fs::MetadataExt;
    (meta.dev(), meta.ino())
}

#[cfg(not(unix))]
fn ids_of(_meta: &fs::Metadata) -> (u64, u64) {
    (0, 0)
}

/// Walks `path` once, recording every member's metadata. Directory
/// listing happens here and only here for a *selected* unit -- this is a
/// recheck of an exact reviewed selection, not an observation pass (see
/// `.oh/guardrails/no-second-traversal-on-report-path.md`, which
/// allow-lists this module for exactly that reason).
/// Records what is at `path` right now, for a plan to carry. Called at
/// **proposal** time; [`reviewed_snapshot`] is its counterpart at
/// execution time, which recaptures and compares.
pub fn capture(path: &Path) -> Result<ReviewedIdentity> {
    Ok(collect(path)?.0)
}

/// Records the anchor's identity without enumerating its members: the
/// [`ReviewedMembership::Anchor`] mode, for ordinary filesystem artifact
/// rows. One `stat`, no traversal.
pub fn capture_anchor(path: &Path) -> Result<ReviewedIdentity> {
    let meta = fs::symlink_metadata(path)
        .map_err(|e| anyhow!("{} could not be observed: {e}", path.display()))?;
    if meta.file_type().is_symlink() {
        bail!(
            "{} is a symlink; a reviewed unit is never a link",
            path.display()
        );
    }
    let (device, inode) = ids_of(&meta);
    Ok(ReviewedIdentity {
        path: path.to_path_buf(),
        device,
        inode,
        is_dir: meta.is_dir(),
        membership: ReviewedMembership::Anchor {
            bytes: if meta.is_dir() { 0 } else { meta.len() },
            mtime: mtime_of(&meta),
        },
    })
}

fn collect(path: &Path) -> Result<(ReviewedIdentity, Vec<ReviewedMember>)> {
    let meta = fs::symlink_metadata(path)
        .map_err(|e| anyhow!("{} could not be re-observed: {e}", path.display()))?;
    if meta.file_type().is_symlink() {
        bail!(
            "{} is a symlink; a reviewed unit is never a link",
            path.display()
        );
    }
    let (device, inode) = ids_of(&meta);
    if meta.is_file() {
        return Ok((
            ReviewedIdentity {
                path: path.to_path_buf(),
                device,
                inode,
                is_dir: false,
                membership: ReviewedMembership::File {
                    bytes: meta.len(),
                    mtime: mtime_of(&meta),
                },
            },
            Vec::new(),
        ));
    }
    if !meta.is_dir() {
        bail!(
            "{} is neither a regular file nor a directory",
            path.display()
        );
    }
    let mut members: Vec<ReviewedMember> = Vec::new();
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir)
            .map_err(|e| anyhow!("{} could not be listed: {e}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| anyhow!("{} could not be listed: {e}", dir.display()))?;
            if members.len() >= SNAPSHOT_ENTRY_LIMIT {
                bail!(
                    "{} has more than {SNAPSHOT_ENTRY_LIMIT} entries; too large to review as one unit",
                    path.display()
                );
            }
            let m = entry
                .metadata()
                .or_else(|_| fs::symlink_metadata(entry.path()))
                .map_err(|e| anyhow!("{} could not be statted: {e}", entry.path().display()))?;
            let (_, ino) = ids_of(&m);
            members.push(ReviewedMember {
                path: entry.path(),
                bytes: if m.is_dir() { 0 } else { m.len() },
                mtime: mtime_of(&m),
                inode: ino,
            });
            if m.is_dir() && !m.file_type().is_symlink() {
                stack.push(entry.path());
            }
        }
    }
    members.sort_by(|a, b| a.path.cmp(&b.path));
    let identity = ReviewedIdentity {
        path: path.to_path_buf(),
        device,
        inode,
        is_dir: true,
        membership: if members.len() <= EXACT_MEMBER_LIMIT {
            ReviewedMembership::Exact {
                members: members.clone(),
            }
        } else {
            let mut hasher = blake3::Hasher::new();
            let mut newest = 0u64;
            let mut allocated = 0u64;
            for m in &members {
                hasher.update(m.path.as_os_str().as_encoded_bytes());
                hasher.update(&m.bytes.to_le_bytes());
                hasher.update(&m.mtime.to_le_bytes());
                hasher.update(&m.inode.to_le_bytes());
                newest = newest.max(m.mtime);
                allocated += m.bytes;
            }
            ReviewedMembership::Summary(ReviewedSummary {
                entry_count: members.len() as u64,
                allocated_bytes: allocated,
                newest_mtime: newest,
                fingerprint: hasher.finalize().to_hex().to_string(),
            })
        },
    };
    Ok((identity, members))
}

/// Takes a fresh reviewed-identity snapshot of `path` **and**, when
/// `reviewed` is supplied, refuses on any drift from it: a replaced or
/// renamed directory (inode change), a new member, a removed member, a
/// member whose size/mtime/inode changed, or a large cache whose bounded
/// summary no longer matches.
///
/// `reviewed: None` is itself a refusal: a plan with no recorded
/// reviewed identity was never reviewed against live state and must be
/// proposed again. Every destructive sink calls this before its first
/// destructive statement.
pub fn reviewed_snapshot(
    path: &Path,
    reviewed: Option<&ReviewedIdentity>,
) -> Result<ReviewedIdentity> {
    // Recapture in the same mode the plan recorded, so an anchor-only
    // review is rechecked as an anchor and a full membership review is
    // rechecked member by member.
    let fresh = match reviewed.map(|r| &r.membership) {
        Some(ReviewedMembership::Anchor { .. }) => capture_anchor(path)?,
        _ => collect(path)?.0,
    };
    let Some(reviewed) = reviewed else {
        bail!(
            "no reviewed identity was recorded for {} when the plan was proposed; propose again so the selection can be reviewed against live state",
            path.display()
        );
    };
    if reviewed.path != fresh.path {
        bail!(
            "reviewed path {} does not match {}",
            reviewed.path.display(),
            fresh.path.display()
        );
    }
    if reviewed.is_dir != fresh.is_dir {
        bail!(
            "{} changed between a file and a directory since it was reviewed; propose again",
            path.display()
        );
    }
    if (reviewed.device, reviewed.inode) != (fresh.device, fresh.inode) {
        bail!(
            "{} is not the directory that was reviewed (it was replaced or renamed: inode {} -> {}); propose again",
            path.display(),
            reviewed.inode,
            fresh.inode
        );
    }
    match (&reviewed.membership, &fresh.membership) {
        (
            ReviewedMembership::Anchor {
                bytes: rb,
                mtime: rm,
            },
            ReviewedMembership::Anchor {
                bytes: fb,
                mtime: fm,
            },
        ) => {
            if rb != fb || rm != fm {
                bail!(
                    "{} changed since it was reviewed ({rb} bytes/mtime {rm} -> {fb} bytes/mtime {fm}); propose again",
                    path.display()
                );
            }
        }
        (
            ReviewedMembership::File {
                bytes: rb,
                mtime: rm,
            },
            ReviewedMembership::File {
                bytes: fb,
                mtime: fm,
            },
        ) => {
            if rb != fb || rm != fm {
                bail!(
                    "{} changed since it was reviewed ({rb} bytes/mtime {rm} -> {fb} bytes/mtime {fm}); propose again",
                    path.display()
                );
            }
        }
        (
            ReviewedMembership::Exact { members: old },
            ReviewedMembership::Exact { members: new },
        ) => {
            for m in new {
                if !old.iter().any(|o| o.path == m.path) {
                    bail!(
                        "{} gained a member that was never reviewed ({}); propose again",
                        path.display(),
                        m.path.display()
                    );
                }
            }
            for o in old {
                match new.iter().find(|m| m.path == o.path) {
                    None => bail!(
                        "reviewed member {} is gone; propose again",
                        o.path.display()
                    ),
                    Some(m) if m != o => bail!(
                        "reviewed member {} changed since it was reviewed; propose again",
                        o.path.display()
                    ),
                    Some(_) => {}
                }
            }
        }
        (ReviewedMembership::Summary(old), ReviewedMembership::Summary(new)) => {
            if old != new {
                bail!(
                    "{} changed since it was reviewed (entries {} -> {}, bytes {} -> {}, newest mtime {} -> {}); propose again",
                    path.display(),
                    old.entry_count,
                    new.entry_count,
                    old.allocated_bytes,
                    new.allocated_bytes,
                    old.newest_mtime,
                    new.newest_mtime
                );
            }
        }
        _ => bail!(
            "{} no longer has the shape it was reviewed with (its member set crossed the exact/summary boundary); propose again",
            path.display()
        ),
    }
    Ok(fresh)
}

/// Every path a reviewed identity covers: the anchor plus each member.
/// What [`member_occupancy`] probes and what [`live_protection`] tests
/// for a protected descendant.
pub fn covered_paths(identity: &ReviewedIdentity) -> Vec<PathBuf> {
    let mut out = vec![identity.path.clone()];
    if let ReviewedMembership::Exact { members } = &identity.membership {
        out.extend(members.iter().map(|m| m.path.clone()));
    }
    out
}

/// Refuses when human keep/protect intent covers `paths` in **either**
/// direction, with the protect list loaded fresh from `store_dir` at the
/// moment of the call -- never from anything the plan captured earlier.
///
/// Corrupt or unreadable protection state is not "nothing is protected":
/// `agents::load_protect` returns an error and this function propagates
/// it, so every action refuses until the file is repaired
/// (`.oh/guardrails/protection-fails-closed.md`).
pub fn live_protection(store_dir: &Path, paths: &[PathBuf]) -> Result<()> {
    let protected = crate::agents::load_protect(store_dir)?;
    for candidate in paths {
        if let Some(reason) = crate::agents::protection_conflict(&protected, candidate) {
            bail!("refused: {reason}");
        }
    }
    Ok(())
}

/// Probes every member of a reviewed unit, not just its anchor. Returns
/// the first non-[`OccupancyState::Free`] answer; `Unknown` is a refusal
/// at every sink, never a silent pass
/// (`.oh/guardrails/occupancy-is-tristate-at-sinks.md`).
///
/// A directory member is probed with `lsof +D`, which covers everything
/// beneath it, so a bounded set of top-level probes still answers for the
/// whole tree.
pub fn member_occupancy(paths: &[PathBuf]) -> OccupancyState {
    // Probing every member of a 100k-entry cache would be its own denial
    // of service; `lsof +D` on the anchor already covers descendants, so
    // probe the anchor plus any member that is not beneath it.
    let mut probed: Vec<&PathBuf> = Vec::new();
    for p in paths {
        if probed.iter().any(|q| p.starts_with(q)) {
            continue;
        }
        probed.push(p);
    }
    for p in probed {
        match crate::occupancy::probe_path(p) {
            OccupancyState::Free => {}
            other => return other,
        }
    }
    OccupancyState::Free
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(p: &Path, b: &[u8]) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, b).unwrap();
    }

    #[test]
    fn unchanged_directory_passes_its_own_recheck() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("cache");
        write(&dir.join("a.txt"), b"one");
        let reviewed = reviewed_snapshot(&dir, None).unwrap_err();
        assert!(reviewed.to_string().contains("no reviewed identity"));
        let (snap, _) = collect(&dir).unwrap();
        reviewed_snapshot(&dir, Some(&snap)).expect("unchanged unit must pass");
    }

    #[test]
    fn a_replaced_directory_is_refused_even_though_it_is_still_a_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("cache");
        write(&dir.join("a.txt"), b"one");
        let (snap, _) = collect(&dir).unwrap();
        fs::rename(&dir, tmp.path().join("aside")).unwrap();
        fs::create_dir(&dir).unwrap();
        write(&dir.join("unrelated.txt"), b"new");
        let err = reviewed_snapshot(&dir, Some(&snap))
            .unwrap_err()
            .to_string();
        assert!(err.contains("replaced or renamed"), "{err}");
    }

    #[test]
    fn a_member_appended_after_review_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("cache");
        write(&dir.join("a.txt"), b"one");
        let (snap, _) = collect(&dir).unwrap();
        write(&dir.join("b.txt"), b"two");
        let err = reviewed_snapshot(&dir, Some(&snap))
            .unwrap_err()
            .to_string();
        assert!(err.contains("gained a member"), "{err}");
    }

    #[test]
    fn a_removed_member_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("cache");
        write(&dir.join("a.txt"), b"one");
        write(&dir.join("b.txt"), b"two");
        let (snap, _) = collect(&dir).unwrap();
        fs::remove_file(dir.join("b.txt")).unwrap();
        let err = reviewed_snapshot(&dir, Some(&snap))
            .unwrap_err()
            .to_string();
        assert!(err.contains("is gone"), "{err}");
    }

    #[test]
    fn a_large_cache_falls_back_to_a_summary_and_still_catches_drift() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("models");
        for i in 0..(EXACT_MEMBER_LIMIT + 5) {
            write(&dir.join(format!("blob-{i}")), b"x");
        }
        let (snap, _) = collect(&dir).unwrap();
        assert!(
            matches!(snap.membership, ReviewedMembership::Summary(_)),
            "beyond the exact limit a unit must be summarized, not enumerated"
        );
        reviewed_snapshot(&dir, Some(&snap)).expect("unchanged large cache passes");
        write(&dir.join("blob-new"), b"x");
        let err = reviewed_snapshot(&dir, Some(&snap))
            .unwrap_err()
            .to_string();
        assert!(err.contains("changed since it was reviewed"), "{err}");
    }

    #[test]
    fn a_rewritten_member_of_a_large_cache_changes_the_fingerprint() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("models");
        for i in 0..(EXACT_MEMBER_LIMIT + 5) {
            write(&dir.join(format!("blob-{i}")), b"x");
        }
        let (snap, _) = collect(&dir).unwrap();
        // Same entry count, different content length: the summary's
        // byte total and fingerprint both move.
        fs::write(dir.join("blob-0"), b"much longer content").unwrap();
        assert!(reviewed_snapshot(&dir, Some(&snap)).is_err());
    }

    #[test]
    fn member_occupancy_probes_a_descendant_not_only_the_anchor() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("cache");
        write(&dir.join("log.txt"), b"held");
        let _open = fs::File::open(dir.join("log.txt")).unwrap();
        let state = member_occupancy(std::slice::from_ref(&dir));
        assert!(
            !state.is_free(),
            "an open descendant must make the parent unit non-free, got {state:?}"
        );
    }
}
