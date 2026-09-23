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

use crate::fs_gate::{self as fs, Metadata};
use crate::occupancy::OccupancyState;
use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How long a [`RecheckProof`] stays spendable. A proof is evidence about
/// the filesystem *now*; one carried across a long pause (a preserved
/// executables copy, a slow sibling unit) is evidence about the past.
pub const MAX_PROOF_AGE: Duration = Duration::from_secs(120);

/// Evidence that all three live rechecks passed for one unit, just now:
/// identity + membership vs the plan ([`reviewed_snapshot`]), protection
/// loaded fresh in both directions ([`live_protection`]), and every
/// member's occupancy `Free` ([`member_occupancy`]).
///
/// Constructible only by [`run_all`] (the fields are private to this
/// module; there is no `Default`, `Clone`, `Deserialize` or other
/// constructor), and consumed by the destructive operation it licenses
/// ([`crate::fs_gate::destroy::trash_move`],
/// [`crate::fs_gate::destroy::Envelope::open`]). A sink that skips a
/// recheck therefore does not compile: it has no proof to hand over.
/// That replaces the `execution_sinks_recheck_live_state`,
/// `occupancy_is_tristate_at_sinks` and `protection_fails_closed` call
/// graph audits (`crates/core/tests/compile_fail/`).
#[must_use = "a recheck proof licenses exactly one destructive operation; hand it to fs_gate::destroy"]
#[derive(Debug)]
pub struct RecheckProof {
    anchor: PathBuf,
    kind: ProofKind,
    covered: Vec<PathBuf>,
    taken_at: Instant,
    /// For a linked worktree: its common git dir, read from the anchor's
    /// `.git` pointer during the recheck (`git worktree prune` runs there
    /// after the move, and nowhere a caller names).
    linked_common: Option<PathBuf>,
}

#[derive(Debug)]
enum ProofKind {
    /// A filesystem unit: its identity as the recheck observed it.
    Path(ReviewedIdentity),
    /// A Docker object the daemon still has.
    Docker(crate::docker::Removal),
}

impl RecheckProof {
    /// The unit the proof is about.
    pub fn anchor(&self) -> &Path {
        &self.anchor
    }

    /// The identity the recheck observed (equal to the reviewed one);
    /// `None` for a Docker object.
    pub fn identity(&self) -> Option<&ReviewedIdentity> {
        match &self.kind {
            ProofKind::Path(identity) => Some(identity),
            ProofKind::Docker(_) => None,
        }
    }

    /// The Docker removal the daemon was just asked about.
    pub(crate) fn docker(&self) -> Option<&crate::docker::Removal> {
        match &self.kind {
            ProofKind::Docker(r) => Some(r),
            ProofKind::Path(_) => None,
        }
    }

    pub(crate) fn linked_common(&self) -> Option<&Path> {
        self.linked_common.as_deref()
    }

    /// Every path the proof covers: the anchor, each exactly-recorded
    /// member, and each reviewed sidecar member with its own members.
    pub fn covers(&self, path: &Path) -> bool {
        self.covered.iter().any(|p| p == path)
    }

    /// Whether the proof is still spendable ([`MAX_PROOF_AGE`]).
    pub fn is_fresh(&self) -> bool {
        self.taken_at.elapsed() <= MAX_PROOF_AGE
    }
}

/// All three rechecks, in order, failing closed: the only constructor of
/// a [`RecheckProof`].
///
/// Every input comes from `auth` -- the anchor, the identity a human
/// reviewed, the sidecar members recorded at proposal (each with its own
/// identity, rechecked exactly like the anchor's), the store whose
/// protect list applies, and for a Docker object the removal the plan
/// named -- never from the sink that calls this (re-review 5, finding 3:
/// a caller-chosen `reviewed`, store or member list made the recheck say
/// whatever the caller wanted). A store with no authority key is not the
/// store the authorization came from, and refuses; a store with a key
/// and no protect file is simply a store where nothing is protected.
pub fn run_all(auth: &crate::authority::Authorized) -> Result<RecheckProof> {
    let target = auth.target();
    let store = auth.store();
    if !crate::fs_gate::key::has_authority_key(store) {
        bail!(
            "refused: {} holds no swamp authority key, so it is not the store this \
             authorization was issued from; protection cannot be read from it",
            store.path().display()
        );
    }
    if let Some(removal) = &target.docker {
        crate::docker::still_removable(removal).map_err(|why| anyhow!("{why}"))?;
        return Ok(RecheckProof {
            anchor: target.anchor.clone(),
            kind: ProofKind::Docker(removal.clone()),
            covered: vec![target.anchor.clone()],
            taken_at: Instant::now(),
            linked_common: None,
        });
    }
    let identity = reviewed_snapshot(&target.anchor, target.reviewed.as_ref())?;
    let mut covered = covered_paths(&identity);
    for member in &target.members {
        if member.path == target.anchor {
            continue;
        }
        let fresh = reviewed_snapshot(&member.path, Some(member))?;
        for p in covered_paths(&fresh) {
            if !covered.contains(&p) {
                covered.push(p);
            }
        }
    }
    live_protection(store.path(), &covered)?;
    match member_occupancy(&covered) {
        OccupancyState::Free => {}
        other => bail!(
            "{}",
            other
                .refusal()
                .unwrap_or_else(|| "occupancy refused this unit".to_string())
        ),
    }
    let linked_common = if target.linked_worktree {
        crate::git::linked_common_dir(&target.anchor)
    } else {
        None
    };
    Ok(RecheckProof {
        anchor: target.anchor.clone(),
        kind: ProofKind::Path(identity),
        covered,
        taken_at: Instant::now(),
        linked_common,
    })
}

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
///
/// **Not `(size, mtime_secs, inode)`.** That fingerprint is one this
/// codebase already rejects in writing: `agents/mod.rs`'s
/// `file_fingerprint` says, in its own words, that a cache which cannot
/// see a same-second, same-size rewrite "is a cache that silently lies",
/// and so uses `mtime_ns`, `ctime_ns` and the inode. The 2026-09-22
/// re-review found the *identification memo*, where a stale answer
/// costs a wrong project label, carrying the strong fingerprint while
/// the *destructive sink*, where a stale answer moves unreviewed user
/// data, carried the weak one. An in-place `write` of the same byte
/// count in the same wall-clock second preserved all three fields, so
/// `reviewed_snapshot` reported the unit unchanged and the content was
/// moved to the Trash under an approval nobody gave for it.
///
/// So this carries `file_fingerprint`'s shape:
///
/// * `bytes` -- the obvious one;
/// * `mtime_ns` -- nanoseconds, not seconds;
/// * `ctime_ns` -- the inode change time, which moves on a rename-over
///   even when the content's mtime is preserved; and
/// * `inode` -- a replaced file is a different file whatever its
///   timestamps say.
///
/// None of it costs a read: it is all in the `stat` already taken.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewedMember {
    pub path: PathBuf,
    pub bytes: u64,
    /// Modification time in nanoseconds since the epoch.
    pub mtime_ns: u64,
    /// Inode change time in nanoseconds since the epoch (`0` where the
    /// platform has none).
    #[serde(default)]
    pub ctime_ns: u64,
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
    Anchor {
        bytes: u64,
        mtime_ns: u64,
        #[serde(default)]
        ctime_ns: u64,
    },
    /// The unit is a single file (a transcript, a config file).
    File {
        bytes: u64,
        mtime_ns: u64,
        #[serde(default)]
        ctime_ns: u64,
    },
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

/// Modification time in **nanoseconds**. Second granularity is what let
/// a same-second rewrite spend an approval; see [`ReviewedMember`].
fn mtime_ns_of(meta: &Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

/// Inode change time in nanoseconds, which moves on a rename-over even
/// when the content's own mtime is preserved.
#[cfg(unix)]
fn ctime_ns_of(meta: &Metadata) -> u64 {
    use crate::fs_gate::MetadataExt;
    (meta.ctime() as u64)
        .saturating_mul(1_000_000_000)
        .saturating_add(meta.ctime_nsec() as u64)
}

#[cfg(not(unix))]
fn ctime_ns_of(_meta: &Metadata) -> u64 {
    0
}

#[cfg(unix)]
fn ids_of(meta: &Metadata) -> (u64, u64) {
    use crate::fs_gate::MetadataExt;
    (meta.dev(), meta.ino())
}

#[cfg(not(unix))]
fn ids_of(_meta: &Metadata) -> (u64, u64) {
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
///
/// Private to the crate (the propose path), so no caller outside it can
/// take a "reviewed" identity at execution time and hand it to the
/// recheck; the `testing` feature re-exports it for fixtures.
#[cfg(not(feature = "testing"))]
pub(crate) fn capture(path: &Path) -> Result<ReviewedIdentity> {
    Ok(collect(path)?.0)
}

/// [`capture`], for integration-test fixtures (`testing` only).
#[cfg(feature = "testing")]
pub fn capture(path: &Path) -> Result<ReviewedIdentity> {
    Ok(collect(path)?.0)
}

/// Records the anchor's identity without enumerating its members: the
/// [`ReviewedMembership::Anchor`] mode, for ordinary filesystem artifact
/// rows. One `stat`, no traversal.
///
/// Public for the TUI, whose marking of a row is its propose step; the
/// gate audit allows naming it only there and in `actions`.
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
            mtime_ns: mtime_ns_of(&meta),
            ctime_ns: ctime_ns_of(&meta),
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
                    mtime_ns: mtime_ns_of(&meta),
                    ctime_ns: ctime_ns_of(&meta),
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
                mtime_ns: mtime_ns_of(&m),
                ctime_ns: ctime_ns_of(&m),
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
                // Length-prefixed field framing (the re-review's P3).
                // The fixed-width `u64`s made a collision contrived
                // rather than practical, but the raw path bytes had no
                // boundary at all: two members whose paths and fields
                // concatenate to the same byte string would hash the
                // same. One length prefix removes the question.
                let path_bytes = m.path.as_os_str().as_encoded_bytes();
                hasher.update(&(path_bytes.len() as u64).to_le_bytes());
                hasher.update(path_bytes);
                hasher.update(&m.bytes.to_le_bytes());
                hasher.update(&m.mtime_ns.to_le_bytes());
                hasher.update(&m.ctime_ns.to_le_bytes());
                hasher.update(&m.inode.to_le_bytes());
                newest = newest.max(m.mtime_ns);
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
#[cfg(feature = "testing")]
pub fn reviewed_snapshot(
    path: &Path,
    reviewed: Option<&ReviewedIdentity>,
) -> Result<ReviewedIdentity> {
    reviewed_snapshot_impl(path, reviewed)
}

#[cfg(not(feature = "testing"))]
pub(crate) fn reviewed_snapshot(
    path: &Path,
    reviewed: Option<&ReviewedIdentity>,
) -> Result<ReviewedIdentity> {
    reviewed_snapshot_impl(path, reviewed)
}

fn reviewed_snapshot_impl(
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
                mtime_ns: rm,
                ctime_ns: rc,
            },
            ReviewedMembership::Anchor {
                bytes: fb,
                mtime_ns: fm,
                ctime_ns: fc,
            },
        ) => {
            if rb != fb || rm != fm || rc != fc {
                bail!(
                    "{} changed since it was reviewed ({rb} bytes/mtime {rm}ns/ctime {rc}ns -> \
                     {fb} bytes/mtime {fm}ns/ctime {fc}ns); propose again",
                    path.display()
                );
            }
        }
        (
            ReviewedMembership::File {
                bytes: rb,
                mtime_ns: rm,
                ctime_ns: rc,
            },
            ReviewedMembership::File {
                bytes: fb,
                mtime_ns: fm,
                ctime_ns: fc,
            },
        ) => {
            if rb != fb || rm != fm || rc != fc {
                bail!(
                    "{} changed since it was reviewed ({rb} bytes/mtime {rm}ns/ctime {rc}ns -> \
                     {fb} bytes/mtime {fm}ns/ctime {fc}ns); propose again",
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
pub(crate) fn covered_paths(identity: &ReviewedIdentity) -> Vec<PathBuf> {
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
pub(crate) fn live_protection(store_dir: &Path, paths: &[PathBuf]) -> Result<()> {
    let protected = crate::protection::load_protect(store_dir)?;
    for candidate in paths {
        if let Some(reason) = protected.conflict(candidate) {
            bail!("refused: {reason}");
        }
    }
    Ok(())
}

/// Proposal-time occupancy, as a refusal: `None` only when every member
/// probed `Free`. The same tri-state, descendant-aware probe the sink's
/// [`run_all`] takes; an unanswerable probe is a refusal, never "nothing
/// open" (`.oh/guardrails/occupancy-is-tristate-at-sinks.md`). The
/// `OccupancyState` itself never leaves this module and `occupancy`.
pub(crate) fn occupancy_refusal(paths: &[PathBuf]) -> Option<String> {
    match member_occupancy(paths) {
        OccupancyState::Free => None,
        other => other.refusal(),
    }
}

/// Probes every member of a reviewed unit, not just its anchor. Returns
/// the first non-[`OccupancyState::Free`] answer; `Unknown` is a refusal
/// at every sink, never a silent pass
/// (`.oh/guardrails/occupancy-is-tristate-at-sinks.md`).
///
/// A directory member is probed with `lsof +D`, which covers everything
/// beneath it, so a bounded set of top-level probes still answers for the
/// whole tree.
fn member_occupancy(paths: &[PathBuf]) -> OccupancyState {
    // Probing every member of a 100k-entry cache would be its own denial
    // of service; `lsof +D` on the anchor already covers descendants, so
    // probe the anchor plus any member that is not beneath it.
    let mut probed: Vec<&PathBuf> = Vec::new();
    for p in paths {
        if probed.iter().any(|q| crate::scope::under(p, q)) {
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

/// Newest mtime anywhere under `path` (files and directories), bounded by
/// `max_entries` so a pathological tree cannot stall the sink; returns
/// `None` when the bound is hit (treated as "could not re-observe").
/// A sink-time re-derivation of a selected unit, like [`capture`].
pub(crate) fn newest_mtime(path: &Path, max_entries: usize) -> Option<u64> {
    let mut newest = 0u64;
    let mut stack = vec![path.to_path_buf()];
    let mut seen = 0usize;
    while let Some(p) = stack.pop() {
        let meta = fs::symlink_metadata(&p).ok()?;
        let m = meta
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs();
        newest = newest.max(m);
        if meta.is_dir() && !meta.file_type().is_symlink() {
            for e in fs::read_dir(&p).ok()?.flatten() {
                seen += 1;
                if seen > max_entries {
                    return None;
                }
                stack.push(e.path());
            }
        }
    }
    Some(newest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

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
            !matches!(state, OccupancyState::Free),
            "an open descendant must make the parent unit non-free, got {state:?}"
        );
    }
}
