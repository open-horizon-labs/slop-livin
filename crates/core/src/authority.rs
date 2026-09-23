//! Authorization as values (`.oh/guardrails/human-only-authorization.md`).
//!
//! Two tokens:
//!
//! * [`HumanConfirmed`] -- a human confirmed **one specific thing** at a
//!   reviewed confirmation site: this plan (its id *and* the digest of
//!   the content that was shown), these standing-grant terms, this
//!   protect-list change, or this one unit of the TUI's confirm dialog.
//!   Each constructor names its site and its subject, and the gate audit
//!   pins each to the one function that shows the human that subject
//!   (`cmd_approve`, `cmd_grant_add`, `cmd_protect`, the TUI's
//!   `start_delete`). A confirmation is **spent** by whatever it
//!   authorizes (it is taken by value and is not `Clone`), and every
//!   consumer refuses a confirmation whose subject is not exactly what it
//!   is about to do.
//! * [`Authorized`] -- *this* execution may act on *this* unit, with the
//!   identity, members, store and (for a Docker object) removal the plan
//!   or the confirmation recorded. Minted only by [`authorize`] (a
//!   stored grant, verified against the stored plan's content digest) or
//!   [`authorize_confirmed`] (one TUI confirmation). The live recheck
//!   (`recheck::run_all`) takes every input it checks from this token, so
//!   no caller chooses what "reviewed" means at execution time.
//!
//! Neither token is `Clone`, `Default`, `Serialize` or `Deserialize`, and
//! neither has a public field.

use crate::fs_gate::StoreDir;
use crate::recheck::ReviewedIdentity;
use anyhow::{Result, bail};
use std::path::{Path, PathBuf};

/// Which reviewed confirmation site minted a [`HumanConfirmed`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmationSite {
    /// A human ran `swamp approve <plan>`.
    CliApprove,
    /// A human ran `swamp grant add …`.
    CliGrantAdd,
    /// A human ran `swamp protect add|remove <path>`.
    CliProtect,
    /// A human accepted the TUI's confirm dialog.
    TuiDialog,
}

impl ConfirmationSite {
    pub fn label(self) -> &'static str {
        match self {
            ConfirmationSite::CliApprove => "cli-approve",
            ConfirmationSite::CliGrantAdd => "cli-grant-add",
            ConfirmationSite::CliProtect => "cli-protect",
            ConfirmationSite::TuiDialog => "tui-dialog",
        }
    }
}

/// A standing grant's terms, exactly as the human typed them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StandingTerms {
    pub predicate: String,
    pub budget_bytes: u64,
    pub max_units: Option<u32>,
    pub expires_in_secs: u64,
}

/// One change to the human keep list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtectChange {
    Add(PathBuf),
    Remove(PathBuf),
}

/// One unit the TUI's confirm dialog listed, with what the TUI recorded
/// about it when the human marked it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedUnit {
    /// The path (or, for a Docker object, its id or name).
    pub path: PathBuf,
    /// What was at `path` when it was marked (`None` refuses at the
    /// recheck, exactly as a plan with no reviewed identity does).
    pub reviewed: Option<ReviewedIdentity>,
    /// Set for a Docker object: the removal the dialog showed.
    pub docker: Option<crate::docker::Removal>,
    /// A linked worktree: after the move, `git worktree prune` runs on
    /// its common dir.
    pub linked_worktree: bool,
    /// The worktree whose `bin/` receives preserved executables
    /// (`--keep-executables`), when that was on.
    pub preserve_into: Option<PathBuf>,
}

/// What one TUI dialog item is.
pub enum Confirmable<'a> {
    /// A stored plan the dialog built for a Cargo or agent-storage unit.
    Plan(&'a crate::actions::Plan),
    /// A path, a worktree or a Docker object acted on directly.
    Unit(SelectedUnit),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Subject {
    Plan {
        plan_id: String,
        content_digest: String,
    },
    StandingGrant(StandingTerms),
    Protect(ProtectChange),
    Selection {
        store: StoreDir,
        unit: SelectedUnit,
    },
}

/// Proof that a human confirmed one subject at a reviewed site. Spent by
/// what it authorizes.
#[must_use = "a confirmation only means something when it is handed to what it authorizes"]
#[derive(Debug)]
pub struct HumanConfirmed {
    id: String,
    site: ConfirmationSite,
    actor: String,
    subject: Subject,
}

impl HumanConfirmed {
    fn mint(site: ConfirmationSite, actor: &str, subject: Subject) -> HumanConfirmed {
        HumanConfirmed {
            id: crate::entities::new_id(),
            site,
            actor: actor.to_string(),
            subject,
        }
    }

    /// `swamp approve <plan>`: the human was shown `plan`. Binds its id
    /// and the digest of its content, so an approval covers exactly the
    /// units that were printed. The gate audit allows this only in the
    /// CLI's `cmd_approve`.
    pub fn cli_approve(actor: &str, plan: &crate::actions::Plan) -> HumanConfirmed {
        Self::mint(
            ConfirmationSite::CliApprove,
            actor,
            Subject::Plan {
                plan_id: plan.id.clone(),
                content_digest: plan.content_digest(),
            },
        )
    }

    /// `swamp grant add …`: the human typed `terms`. Only in the CLI's
    /// `cmd_grant_add`.
    pub fn cli_grant(actor: &str, terms: StandingTerms) -> HumanConfirmed {
        Self::mint(
            ConfirmationSite::CliGrantAdd,
            actor,
            Subject::StandingGrant(terms),
        )
    }

    /// `swamp protect add|remove <path>`. Only in the CLI's `cmd_protect`.
    pub fn cli_protect(actor: &str, change: ProtectChange) -> HumanConfirmed {
        Self::mint(
            ConfirmationSite::CliProtect,
            actor,
            Subject::Protect(change),
        )
    }

    /// The TUI's confirm dialog accepted: one confirmation per listed
    /// item, each bound to exactly that item (a plan by id and content
    /// digest; a unit by path, recorded identity and removal). Only in
    /// the TUI's `start_delete`.
    pub fn tui_dialog(
        actor: &str,
        store: &StoreDir,
        items: Vec<Confirmable<'_>>,
    ) -> Vec<HumanConfirmed> {
        items
            .into_iter()
            .map(|item| {
                let subject = match item {
                    Confirmable::Plan(plan) => Subject::Plan {
                        plan_id: plan.id.clone(),
                        content_digest: plan.content_digest(),
                    },
                    Confirmable::Unit(unit) => Subject::Selection {
                        store: store.clone(),
                        unit,
                    },
                };
                Self::mint(ConfirmationSite::TuiDialog, actor, subject)
            })
            .collect()
    }

    pub fn actor(&self) -> &str {
        &self.actor
    }

    /// This confirmation's own id: what a grant records as its
    /// `confirmation`, and what the ledger records for a direct TUI
    /// action.
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn site(&self) -> ConfirmationSite {
        self.site
    }

    /// Whether this confirms approving plan `plan_id` whose content
    /// digest is `digest`, at a site that may approve plans.
    pub(crate) fn confirms_plan(&self, plan_id: &str, digest: &str) -> bool {
        matches!(
            self.site,
            ConfirmationSite::CliApprove | ConfirmationSite::TuiDialog
        ) && self.subject
            == Subject::Plan {
                plan_id: plan_id.to_string(),
                content_digest: digest.to_string(),
            }
    }

    /// Whether this confirms exactly these standing-grant terms.
    pub(crate) fn confirms_terms(&self, terms: &StandingTerms) -> bool {
        self.site == ConfirmationSite::CliGrantAdd
            && self.subject == Subject::StandingGrant(terms.clone())
    }

    /// Whether this confirms exactly this keep-list change.
    pub(crate) fn confirms_protect(&self, change: &ProtectChange) -> bool {
        self.site == ConfirmationSite::CliProtect
            && self.subject == Subject::Protect(change.clone())
    }
}

/// What an [`Authorized`] execution acts on, copied from the stored plan
/// unit or the TUI confirmation -- never supplied by the sink.
#[derive(Debug)]
pub(crate) struct Target {
    pub(crate) anchor: PathBuf,
    pub(crate) reviewed: Option<ReviewedIdentity>,
    /// Sidecar members recorded at proposal (a session's files outside
    /// its anchor, a Cargo group's companions), each with its identity.
    pub(crate) members: Vec<ReviewedIdentity>,
    pub(crate) docker: Option<crate::docker::Removal>,
    pub(crate) linked_worktree: bool,
    pub(crate) preserve_into: Option<PathBuf>,
}

/// This execution may act on this unit. Borrowed by every destructive
/// operation; the live recheck reads every input from it.
#[derive(Debug)]
pub struct Authorized {
    grant_id: String,
    store: StoreDir,
    target: Target,
}

impl Authorized {
    /// Whether `path` is the anchor this authorization names. Exact
    /// equality: an authorization for a directory is not an
    /// authorization for its parent or a sibling.
    pub fn covers(&self, path: &Path) -> bool {
        self.target.anchor == path
    }

    pub fn grant_id(&self) -> &str {
        &self.grant_id
    }

    pub(crate) fn store(&self) -> &StoreDir {
        &self.store
    }

    pub(crate) fn target(&self) -> &Target {
        &self.target
    }
}

/// An authorization for unit `index` of `plan` under `grant`, or `None`
/// when the grant is not live at `at`, does not cover the unit, was
/// approved for different plan content than `plan` now holds, or has no
/// budget left. `plan` and `grant` exist only as the store loader
/// verified them (neither type is `Deserialize`, and both carry swamp's
/// keyed binding on disk); `store` is the one they were loaded from.
#[allow(clippy::too_many_arguments)]
pub(crate) fn authorize(
    plan: &crate::actions::Plan,
    plan_digest: &str,
    index: usize,
    grant: &crate::actions::Grant,
    store: &StoreDir,
    spent_bytes: u64,
    used_units: u32,
    at: u64,
) -> Option<Authorized> {
    let unit = plan.units().get(index)?;
    if plan.content_digest() != plan_digest {
        return None;
    }
    // (`content_digest` is computed once per plan and cached.)
    if !crate::actions::grant_is_live_and_covers(grant, plan, plan_digest, unit, at) {
        return None;
    }
    if let Some(b) = grant.budget_bytes()
        && spent_bytes + unit.bytes() > b
    {
        return None;
    }
    if let Some(mu) = grant.max_units()
        && used_units + 1 > mu
    {
        return None;
    }
    Some(Authorized {
        grant_id: grant.id().to_string(),
        store: store.clone(),
        target: crate::actions::target_of(unit),
    })
}

/// An authorization for exactly the unit one TUI confirmation named,
/// spending the confirmation. Refused when the confirmation is not the
/// TUI dialog's, or names a plan rather than a unit, or names a
/// different path than `path`.
pub fn authorize_confirmed(confirmed: HumanConfirmed, path: &Path) -> Result<Authorized> {
    let HumanConfirmed {
        id, site, subject, ..
    } = confirmed;
    if site != ConfirmationSite::TuiDialog {
        bail!("refused: only the TUI's confirm dialog authorizes a unit directly");
    }
    let Subject::Selection { store, unit } = subject else {
        bail!("refused: this confirmation names a plan, not a unit; approve the plan instead");
    };
    if unit.path != path {
        bail!(
            "refused: the confirmation names {}, not {}",
            unit.path.display(),
            path.display()
        );
    }
    // The dialog's store is where protection is read from at the sink;
    // make sure it is a store swamp keeps authority state in.
    crate::fs_gate::key::authority_key(&store)?;
    Ok(Authorized {
        grant_id: id,
        store,
        target: Target {
            anchor: unit.path,
            reviewed: unit.reviewed,
            members: Vec::new(),
            docker: unit.docker,
            linked_worktree: unit.linked_worktree,
            preserve_into: unit.preserve_into,
        },
    })
}

/// An authorization for `anchor`, for this crate's own unit tests that
/// drive a sink function directly.
#[cfg(test)]
pub(crate) fn for_tests(anchor: &Path, store: &Path, members: &[PathBuf]) -> Authorized {
    crate::fs_gate::key::authority_key(&StoreDir::at(store).expect("test store"))
        .expect("test store key");
    Authorized {
        grant_id: "test".to_string(),
        store: StoreDir::at(store).expect("test store"),
        target: Target {
            anchor: anchor.to_path_buf(),
            reviewed: crate::recheck::capture(anchor).ok(),
            members: members
                .iter()
                .filter(|m| m.as_path() != anchor)
                .map(|m| crate::recheck::capture(m).expect("test member"))
                .collect(),
            docker: None,
            linked_worktree: false,
            preserve_into: None,
        },
    }
}
