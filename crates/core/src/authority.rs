//! Authorization as values (`.oh/guardrails/human-only-authorization.md`).
//!
//! Two tokens, each constructible in exactly one place:
//!
//! * [`HumanConfirmed`] -- a human confirmed something at a reviewed
//!   confirmation site. Its constructors are public (the CLI and the TUI
//!   are other crates), and the `gate_paths_only_inside_gates` audit
//!   allows naming them only in `crates/cli/src/main.rs` (the `approve`
//!   and `grant add` handlers) and `crates/tui/src/app.rs` (the confirm
//!   dialog). Every function that mints lasting authorization --
//!   [`crate::actions::approve`], [`crate::actions::add_standing_grant`]
//!   -- borrows one, so a keystroke handler, an agent path or a
//!   convenience wrapper that does not hold one does not compile.
//! * [`Authorized`] -- *this* execution may act on *these* anchors.
//!   Minted only by [`authorize`] (a live grant that covers the unit,
//!   within budget) or [`authorize_confirmed`] (the TUI's direct confirm
//!   path, from a [`HumanConfirmed`]). Every destructive operation in
//!   [`crate::fs_gate::destroy`] borrows one and refuses an anchor it
//!   does not cover.
//!
//! Neither is `Clone`, `Default`, `Serialize` or `Deserialize`; neither
//! has a public field. That is the whole mechanism.

use std::path::{Path, PathBuf};

/// Which reviewed confirmation site minted a [`HumanConfirmed`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmationSite {
    /// A human ran `swamp approve <plan>` or `swamp grant add …`.
    CliCommand,
    /// A human accepted the TUI's confirm dialog.
    TuiDialog,
}

/// Proof that a human confirmed at a reviewed site, held for the duration
/// of the confirmed operation.
#[must_use = "a confirmation only means something when it is handed to what it authorizes"]
#[derive(Debug)]
pub struct HumanConfirmed {
    site: ConfirmationSite,
    actor: String,
}

impl HumanConfirmed {
    /// The CLI's `approve` / `grant add` handlers: the human typed the
    /// command. Naming this anywhere but `crates/cli/src/main.rs` fails
    /// the gate audit.
    pub fn cli_command(actor: &str) -> HumanConfirmed {
        HumanConfirmed {
            site: ConfirmationSite::CliCommand,
            actor: actor.to_string(),
        }
    }

    /// The TUI's confirm dialog accepted. Naming this anywhere but
    /// `crates/tui/src/app.rs` fails the gate audit.
    pub fn tui_dialog(actor: &str) -> HumanConfirmed {
        HumanConfirmed {
            site: ConfirmationSite::TuiDialog,
            actor: actor.to_string(),
        }
    }

    pub fn site(&self) -> ConfirmationSite {
        self.site
    }

    pub fn actor(&self) -> &str {
        &self.actor
    }
}

/// This execution may act on these anchors. Borrowed by every
/// destructive operation; see the module docs.
#[derive(Debug)]
pub struct Authorized {
    anchors: Vec<PathBuf>,
    grant_id: String,
}

impl Authorized {
    /// Whether `path` is one of the anchors this authorization names.
    /// Exact equality: an authorization for a directory is not an
    /// authorization for its parent or a sibling.
    pub fn covers(&self, path: &Path) -> bool {
        self.anchors.iter().any(|a| a == path)
    }

    pub fn grant_id(&self) -> &str {
        &self.grant_id
    }
}

/// An authorization for `unit` under `grant`, or `None` when the grant is
/// not live at `at`, does not cover the unit, or has no budget left for
/// it. The one constructor the plan executor uses.
pub fn authorize(
    plan: &crate::actions::Plan,
    unit: &crate::actions::PlanUnit,
    grant: &crate::actions::Grant,
    spent_bytes: u64,
    used_units: u32,
    at: u64,
) -> Option<Authorized> {
    if !crate::actions::grant_is_live_and_covers(grant, plan, unit, at) {
        return None;
    }
    if let Some(b) = grant.budget_bytes
        && spent_bytes + unit.bytes > b
    {
        return None;
    }
    if let Some(mu) = grant.max_units
        && used_units + 1 > mu
    {
        return None;
    }
    Some(Authorized {
        anchors: vec![unit.path.clone()],
        grant_id: grant.id.clone(),
    })
}

/// An authorization for exactly the anchors a human confirmed in the TUI
/// dialog, under the one-shot grant the dialog recorded.
pub fn authorize_confirmed(
    confirmed: &HumanConfirmed,
    grant_id: &str,
    anchors: &[PathBuf],
) -> Option<Authorized> {
    if confirmed.site != ConfirmationSite::TuiDialog || anchors.is_empty() {
        return None;
    }
    Some(Authorized {
        anchors: anchors.to_vec(),
        grant_id: grant_id.to_string(),
    })
}

/// An authorization for `anchors`, for this crate's own unit tests that
/// drive a sink function directly.
#[cfg(test)]
pub(crate) fn for_tests(anchors: &[PathBuf]) -> Authorized {
    Authorized {
        anchors: anchors.to_vec(),
        grant_id: "test".to_string(),
    }
}
