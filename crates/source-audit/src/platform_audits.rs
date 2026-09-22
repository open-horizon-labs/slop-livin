//! Platform-capability audits (#79).
//!
//! The behavioural invariants this file guards used to be written in
//! macOS's vocabulary -- "FSEvents before a full walk", "scheduling
//! drives launchctl" -- which made them unenforceable on a second
//! platform and, worse, made *removing* them on that platform look like
//! the correct thing to do. They are re-stated here in terms of the
//! contract both platforms implement, with the backend-specific checks
//! kept alongside rather than in place of them.
//!
//! Two rules, and both are about the same failure: a capability a build
//! does not have must be *refused*, not approximated.
//!
//! 1. A command that installs unattended state consults the platform's
//!    scheduling capability, **honours the answer** (§17.2 -- a
//!    discarded result is a fail), and does so **before** it writes
//!    anything. Writing a LaunchAgent plist into a directory no daemon
//!    reads, and reporting success, is worse than having no scheduler.
//!
//! 2. The two refusals that mean "this platform cannot replay history"
//!    are produced in exactly one place, which derives them from the
//!    platform's continuity source. Hardcoding either one anywhere else
//!    is how "no backend written yet" and "the kernel keeps no history"
//!    become the same message -- and they are not the same message, so
//!    they must not become the same code.

use crate::ast;
use crate::resolve::{self, Honoured};
use std::path::Path;

/// The refusal variants that mean "this platform cannot replay". Only
/// [`PLATFORM_REFUSAL_FN`] may name them.
const PLATFORM_REFUSALS: &[&str] = &["UnsupportedPlatform", "NoPersistedChangeHistory"];

/// The one function allowed to decide which of the two applies.
const PLATFORM_REFUSAL_FN: &str = "platform_refusal";

/// Calls that write unattended state to disk. A scheduling command that
/// reaches any of these without having honoured the capability answer
/// first has installed something on a platform that cannot run it.
const INSTALL_WRITES: &[&str] = &[
    "std::fs::write",
    "std::fs::create_dir_all",
    "std::fs::File::create",
];

pub fn platform_capabilities_gate_their_backends(root: &Path) -> Result<(), String> {
    capability_table_exists(root)?;
    scheduling_is_consulted_before_anything_is_installed(root)?;
    platform_refusals_come_from_the_contract(root)?;
    Ok(())
}

/// The contract itself has to exist, and `Os::current` has to be
/// `cfg`-gated: a `current()` that decided at runtime would put both
/// platforms' answers in both binaries, which is the thing target gating
/// is for.
fn capability_table_exists(root: &Path) -> Result<(), String> {
    // Parsed as well as read: a file that no longer parses is a
    // failure here rather than a rule that quietly matches nothing.
    ast::parse(root, "crates/core/src/platform/mod.rs")?;
    let text = std::fs::read_to_string(root.join("crates/core/src/platform/mod.rs"))
        .map_err(|e| format!("read platform/mod.rs: {e}"))?;
    if !text.contains("CAPABILITIES") {
        return Err(
            "platform/mod.rs declares no CAPABILITIES table: there is nothing for \
             docs/platform.md to be checked against"
                .into(),
        );
    }
    let cfg_gated = text.matches("#[cfg(target_os = \"macos\")]").count() > 0
        && text.matches("#[cfg(target_os = \"linux\")]").count() > 0;
    if !cfg_gated {
        return Err(
            "platform/mod.rs: Os::current is not target-gated, so both platforms' answers \
             compile into both builds"
                .into(),
        );
    }
    Ok(())
}

/// Every definition of `install` in `schedule.rs` -- not the first one
/// found, because the mutation corpus puts its variant beside the
/// original and a rule that stops at the first match inspects whichever
/// one it happens to reach.
///
/// Each one must ask the platform's scheduling capability, honour the
/// answer, and ask *before* it writes. Statement text is read through
/// the resolver, so `use std::fs::write as emit;` is still a write and
/// `use Scheduling::for_os as can;` is still the question.
fn scheduling_is_consulted_before_anything_is_installed(root: &Path) -> Result<(), String> {
    let rel = "crates/core/src/schedule.rs";
    let parsed = ast::parse(root, rel)?;
    let funcs = ast::functions(&parsed.ast);
    let installers: Vec<&ast::Func> = funcs.iter().filter(|f| f.name == "install").collect();
    if installers.is_empty() {
        return Err(format!(
            "{rel}: defines no `install`; the scheduling capability guard has nothing to guard"
        ));
    }

    for f in &installers {
        let asks = |stmt: &str| {
            stmt.contains("scheduling (")
                || stmt.contains("scheduling()")
                || stmt.contains("Scheduling :: for_os")
                || stmt.contains("Scheduling::for_os")
        };
        let writes = |stmt: &str| {
            INSTALL_WRITES
                .iter()
                .any(|w| stmt.contains(&w.replace("::", " :: ")) || stmt.contains(w))
        };
        let asked_at = f.stmts.iter().position(|s| asks(s));
        let Some(asked_at) = asked_at else {
            return Err(format!(
                "{rel}::install: never asks whether this platform can schedule anything. On a \
                 platform without a scheduler it would write a job that can never run and \
                 report success."
            ));
        };
        if let Some(wrote_at) = f.stmts.iter().position(|s| writes(s))
            && wrote_at < asked_at
        {
            return Err(format!(
                "{rel}::install: statement {wrote_at} writes to disk before statement \
                 {asked_at} asks whether this platform can schedule at all. A refusal after a \
                 write is not a refusal -- it leaves the state behind and reports failure."
            ));
        }
    }

    // And the answer has to change what happens. Asking is not refusing
    // (GUARDRAILS_SPEC section 17, item 2).
    for c in resolve::calls(&resolve::parse(root, rel)?.ast) {
        if c.func != "install" || c.in_test {
            continue;
        }
        let is_question = resolve::path_ends_with(&c.path, "scheduling")
            || resolve::path_ends_with(&c.path, "Scheduling::for_os");
        if is_question && c.honoured == Honoured::Discarded {
            return Err(format!(
                "{rel}::install: asks the scheduling capability and discards the answer \
                 (`{}` in a discarded position). Asking is not refusing.",
                c.written
            ));
        }
    }
    Ok(())
}

/// "This platform cannot replay history" is decided once, from the
/// platform's continuity source. Anywhere else naming one of the two
/// refusals is a second decision that can disagree with the first.
fn platform_refusals_come_from_the_contract(root: &Path) -> Result<(), String> {
    let rel = "crates/core/src/fs_events.rs";
    let parsed = ast::parse(root, rel)?;

    // `functions` excludes `#[cfg(test)]` modules: a test may name a
    // refusal variant, because asserting which one a platform gives is
    // exactly what the test is for.
    let funcs = ast::functions(&parsed.ast);
    let decider_count = funcs
        .iter()
        .filter(|f| f.name == PLATFORM_REFUSAL_FN)
        .count();
    if decider_count == 0 {
        return Err(format!(
            "{rel}: no `{PLATFORM_REFUSAL_FN}`; the refusal a platform without replay gives is \
             hardcoded somewhere instead of derived from its continuity source"
        ));
    }

    // The decider must actually read the platform contract, not just
    // exist with the right name.
    for f in funcs.iter().filter(|f| f.name == PLATFORM_REFUSAL_FN) {
        if !(f.body.contains("ContinuitySource") || f.body.contains("replays_history")) {
            return Err(format!(
                "{rel}::{PLATFORM_REFUSAL_FN}: does not consult the platform's continuity \
                 source, so it is a hardcoded answer wearing the right name"
            ));
        }
    }

    // And nothing else may name either refusal outside a test.
    for f in &funcs {
        if f.name == PLATFORM_REFUSAL_FN {
            continue;
        }
        for variant in PLATFORM_REFUSALS {
            let needle = format!("RefreshRefusal :: {variant}");
            let compact = format!("RefreshRefusal::{variant}");
            // `as_str`/`explanation` are the enum's own exhaustive
            // matches over every variant; they name them because a match
            // must, and they decide nothing.
            if f.name == "as_str" || f.name == "explanation" {
                continue;
            }
            if f.body.contains(&needle) || f.body.contains(&compact) {
                return Err(format!(
                    "{rel}::{}: names RefreshRefusal::{variant} directly. Which refusal a \
                     platform without replay gives is `{PLATFORM_REFUSAL_FN}`'s decision; a \
                     second copy can disagree with it, and then 'no backend yet' and 'this \
                     kernel keeps no history' become the same message.",
                    f.name
                ));
            }
        }
    }
    Ok(())
}
