//! Observation regions (#42): what a multi-root observation of a resolved
//! [`crate::scope::EffectiveScope`] actually covered this pass, and why,
//! so a report/coverage consumer never mistakes "not observed" for
//! "observed empty" or "deleted". See
//! `.oh/guardrails/coverage-changes-are-not-storage-changes.md`.
//!
//! [`RootCoverage`]/[`RegionStatus`] describe *this observation's* outcome
//! per root; they are never persisted as byte history and never feed the
//! reverse-delta store directly. [`crate::scope::EffectiveScope`] (#41)
//! decides which roots are candidates; this module reports what happened
//! when `crate::report::report_scope` tried to observe them.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// One root's observation outcome this pass. Five states, matching #42's
/// acceptance criteria; a `SkippedAsNested` root from
/// [`crate::scope::EffectiveScope`] does not get its own row here at all
/// -- it was folded into its parent root's own walk (the parent's walk
/// recurses into it), so the parent's region already accounts for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum RegionStatus {
    /// The root was walked to completion this pass; growth/tombstone
    /// bookkeeping for it is authoritative.
    Complete,
    /// The root was walked, but part of it could not be read (e.g. a
    /// subdirectory became unreadable, or a previously known worktree's
    /// path could not be reconfirmed). Rows under an unconfirmed region
    /// are *not* tombstoned by this observation -- see
    /// `growth::compute_unconfirmed_worktrees`.
    Partial { reason: String },
    /// A configured exclusion removed this root from scope, or (via
    /// `EffectiveScope::pruned_subtrees`) pruned a subtree inside a kept
    /// root. Not observed; never treated as deleted.
    Excluded,
    /// The root does not currently exist. Not observed; rows previously
    /// stored for it keep whatever history they already have, untouched
    /// -- this is a coverage change, never a storage change (#41/#42).
    Missing,
    /// The root exists but could not be read at all (e.g. `chmod 000`,
    /// or access was lost between scope resolution and the walk). No
    /// walk was attempted; the store for this root is completely
    /// untouched, and no FSEvents cursor was advanced for it.
    Inaccessible { reason: String },
}

impl RegionStatus {
    pub fn label(&self) -> String {
        match self {
            RegionStatus::Complete => "complete".to_string(),
            RegionStatus::Partial { reason } => format!("partial ({reason})"),
            RegionStatus::Excluded => "excluded".to_string(),
            RegionStatus::Missing => "missing".to_string(),
            RegionStatus::Inaccessible { reason } => format!("inaccessible ({reason})"),
        }
    }

    /// Whether this observation actually persisted anything to the
    /// growth store for this root -- `Complete`/`Partial` only.
    pub fn was_observed(&self) -> bool {
        matches!(self, RegionStatus::Complete | RegionStatus::Partial { .. })
    }
}

/// One root's coverage row for a `report_scope` call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RootCoverage {
    pub path: PathBuf,
    /// Flattened so the JSON shape is `{"status": "missing", ...}` (or
    /// `{"status": "partial", "reason": "...", ...}`) rather than a
    /// doubly-nested `{"status": {"status": "missing"}}` -- `RegionStatus`
    /// is itself internally tagged on a field named `status`.
    #[serde(flatten)]
    pub status: RegionStatus,
    /// Populated only when `status.was_observed()`.
    pub walked_total: u64,
    pub projects: usize,
    /// `"full"` / `"incremental"` / `""` when not walked.
    #[serde(default)]
    pub mode: String,
}

impl RootCoverage {
    pub fn excluded(path: PathBuf) -> Self {
        Self {
            path,
            status: RegionStatus::Excluded,
            walked_total: 0,
            projects: 0,
            mode: String::new(),
        }
    }
    pub fn missing(path: PathBuf) -> Self {
        Self {
            path,
            status: RegionStatus::Missing,
            walked_total: 0,
            projects: 0,
            mode: String::new(),
        }
    }
    pub fn inaccessible(path: PathBuf, reason: String) -> Self {
        Self {
            path,
            status: RegionStatus::Inaccessible { reason },
            walked_total: 0,
            projects: 0,
            mode: String::new(),
        }
    }
}
