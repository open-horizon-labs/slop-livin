//! Every way live state can move between approval and execution
//! (`.oh/guardrails/execution-sinks-recheck-live-state.md`).
//!
//! The 2026-09-21 reviews found three separate paths by which an
//! approved plan spent its authorization on data nobody had reviewed,
//! and the repair is one shared recheck model (`swamp_core::recheck`)
//! rather than three patches. These tests drive that model through the
//! real `execute` surface for each kind of drift, and assert the same
//! two things every time: **nothing moved**, and the refusal says why.
//!
//! Every fixture is a disposable `tempfile` tree; no real home, tool
//! store or user swamp state is read, and nothing is removed outside the
//! per-test temp directory.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use swamp_core::actions::{self, approve, execute_with_trash, save_plan};
use swamp_core::locations::{Environment, Platform, Registry};
use swamp_core::scope::{EffectiveScope, ScanConfig, resolve_effective_scope};

/// One test in this file shadows `PATH` to make `lsof` unavailable, and
/// `PATH` is process-global. Serializing the whole file on one lock
/// makes that safe under the default test harness rather than relying on
/// every caller remembering `--test-threads=1`.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

fn only_claude() -> ScanConfig {
    let registry = Registry::with_builtins();
    ScanConfig {
        defaults: false,
        disabled_detectors: registry
            .detectors()
            .iter()
            .map(|d| d.id().to_string())
            .filter(|id| id != "claude-code")
            .collect(),
        ..Default::default()
    }
}

/// A synthetic Claude Code home with one cache directory holding one
/// file: the smallest unit that has both an anchor and a member, which
/// is what the descendant checks need.
struct Fixture {
    _tmp: tempfile::TempDir,
    home: PathBuf,
    cache: PathBuf,
    member: PathBuf,
    scope: EffectiveScope,
    store: tempfile::TempDir,
    trash: tempfile::TempDir,
}

fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("claude");
    fs::create_dir_all(home.join("debug")).unwrap();
    fs::write(home.join("debug/log.txt"), b"original").unwrap();
    let env = Environment::fixture(
        tmp.path().to_path_buf(),
        HashMap::from([("CLAUDE_CONFIG_DIR".to_string(), home.display().to_string())]),
        Platform::MacOS,
    );
    let scope =
        resolve_effective_scope(&env, &only_claude(), &[], &Registry::with_builtins(), 1_000);
    Fixture {
        cache: home.join("debug"),
        member: home.join("debug/log.txt"),
        home,
        scope,
        store: tempfile::tempdir().unwrap(),
        trash: tempfile::tempdir().unwrap(),
        _tmp: tmp,
    }
}

impl Fixture {
    /// Proposes and approves a plan for the cache unit, returning the
    /// plan id. Everything after this point is the sink's problem.
    fn approved_plan(&self) -> String {
        let units = swamp_core::agents::discover_and_measure(
            &self.scope,
            &[],
            Some(self.store.path()),
            false,
            1_000,
            30,
            3600,
        )
        .expect("discovery");
        let plan = actions::propose_agents(
            &units,
            std::slice::from_ref(&self.cache),
            "test:execution-rechecks",
        )
        .expect("the fixture's cache unit must be proposable before anything drifts");
        save_plan(self.store.path(), &plan).unwrap();
        approve(self.store.path(), &plan.id, "human:test").unwrap();
        plan.id
    }

    fn execute(&self, plan_id: &str) -> swamp_core::actions::ExecuteResult {
        execute_with_trash(self.store.path(), plan_id, "human:test", self.trash.path())
            .expect("execute returns outcomes rather than failing")
    }

    /// The ledger's own record of what happened, so a refusal is proven
    /// durable and not merely returned.
    fn ledger_outcomes(&self) -> Vec<(String, Option<String>)> {
        let ledger = swamp_core::ledger::Ledger::open(self.store.path().join("ledger.jsonl"))
            .expect("ledger");
        ledger
            .all()
            .unwrap_or_default()
            .into_iter()
            .map(|r| (r.outcome, r.observed_path_state))
            .collect()
    }
}

/// Asserts the one thing that matters: the reviewed data is still there,
/// the outcome is not "completed", and the cause names the drift.
fn assert_refused(
    result: &swamp_core::actions::ExecuteResult,
    still_present: &Path,
    expect_in_cause: &[&str],
) {
    assert!(
        still_present.exists(),
        "reviewed data was moved despite drift: {:?}",
        result.outcomes
    );
    let outcome = result
        .outcomes
        .first()
        .expect("one unit was planned, so one outcome is reported");
    assert_ne!(
        outcome.status, "completed",
        "drift must refuse: {:?}",
        result.outcomes
    );
    let cause = outcome.cause.clone().unwrap_or_default();
    assert!(
        expect_in_cause.iter().any(|w| cause.contains(w)),
        "the refusal must say what drifted (looking for one of {expect_in_cause:?}): {cause}"
    );
    assert_eq!(result.trashed_bytes, 0, "nothing may reach Trash");
}

#[test]
fn protection_added_on_the_unit_after_approval_refuses() {
    let _serial = serial();
    let fx = fixture();
    let plan = fx.approved_plan();
    swamp_core::agents::protect_add(fx.store.path(), &fx.cache).unwrap();
    let result = fx.execute(&plan);
    assert_refused(&result, &fx.cache, &["protect"]);
}

#[test]
fn protection_added_on_an_ancestor_after_approval_refuses() {
    let _serial = serial();
    // The obvious direction, at a distance: protecting the tool home
    // must protect everything inside it.
    let fx = fixture();
    let plan = fx.approved_plan();
    swamp_core::agents::protect_add(fx.store.path(), &fx.home).unwrap();
    let result = fx.execute(&plan);
    assert_refused(&result, &fx.cache, &["protect", "beneath"]);
}

#[test]
fn protection_added_on_a_descendant_after_approval_refuses() {
    let _serial = serial();
    // The direction the review falsified: protecting one file inside a
    // directory must stop the directory being removed, or the
    // protection means nothing.
    let fx = fixture();
    let plan = fx.approved_plan();
    swamp_core::agents::protect_add(fx.store.path(), &fx.member).unwrap();
    let result = fx.execute(&plan);
    assert_refused(&result, &fx.cache, &["contains human-protected"]);
}

#[test]
fn a_member_appended_after_approval_refuses() {
    let _serial = serial();
    let fx = fixture();
    let plan = fx.approved_plan();
    fs::write(fx.cache.join("appeared-later.txt"), b"never reviewed").unwrap();
    let result = fx.execute(&plan);
    assert_refused(&result, &fx.cache, &["gained a member", "changed"]);
    assert!(
        fx.cache.join("appeared-later.txt").exists(),
        "the unreviewed member must also still be there"
    );
}

#[test]
fn a_member_removed_after_approval_refuses() {
    let _serial = serial();
    let fx = fixture();
    let plan = fx.approved_plan();
    fs::remove_file(&fx.member).unwrap();
    let result = fx.execute(&plan);
    assert_refused(&result, &fx.cache, &["is gone", "changed"]);
}

#[test]
fn a_replaced_directory_refuses_even_though_the_path_is_still_a_directory() {
    let _serial = serial();
    let fx = fixture();
    let plan = fx.approved_plan();
    fs::rename(&fx.cache, fx.home.join("moved-aside")).unwrap();
    fs::create_dir(&fx.cache).unwrap();
    fs::write(
        fx.cache.join("unrelated.txt"),
        b"different content entirely",
    )
    .unwrap();
    let result = fx.execute(&plan);
    assert_refused(&result, &fx.cache, &["replaced or renamed"]);
    assert!(
        fx.cache.join("unrelated.txt").exists(),
        "the replacement's own content must be untouched"
    );
}

#[test]
fn an_open_member_refuses_the_parent_removal() {
    let _serial = serial();
    let fx = fixture();
    let plan = fx.approved_plan();
    let _held = fs::File::open(&fx.member).expect("hold the member open");
    let result = fx.execute(&plan);
    assert_refused(&result, &fx.cache, &["open file handle"]);
}

#[test]
fn an_unanswerable_occupancy_probe_refuses() {
    let _serial = serial();
    // `lsof` is how occupancy is answered. With it unavailable the
    // answer is Unknown, and Unknown refuses -- the PR #123 review found
    // the recheck it replaced falling *through* on exactly this case.
    let fx = fixture();
    let plan = fx.approved_plan();
    let shim_dir = tempfile::tempdir().unwrap();
    // An empty directory first on PATH does not hide lsof; a directory
    // containing a non-executable `lsof` does.
    let shim = shim_dir.path().join("lsof");
    fs::write(&shim, b"not executable").unwrap();
    let previous = std::env::var("PATH").unwrap_or_default();
    unsafe {
        std::env::set_var("PATH", shim_dir.path().display().to_string());
    }
    let result = fx.execute(&plan);
    unsafe {
        std::env::set_var("PATH", previous);
    }
    assert_refused(
        &result,
        &fx.cache,
        &["occupancy could not be determined", "lsof"],
    );
}

#[test]
fn a_corrupt_protect_file_refuses_and_protect_list_reports_the_corruption() {
    let _serial = serial();
    let fx = fixture();
    let plan = fx.approved_plan();
    // Protect something, then corrupt the file: the protections are
    // unreadable, not absent.
    swamp_core::agents::protect_add(fx.store.path(), &fx.member).unwrap();
    let protect_file = swamp_core::agents::protect_path(fx.store.path());
    fs::write(&protect_file, b"{\"paths\": [\"").unwrap();

    let listed = swamp_core::agents::protect_list(fx.store.path());
    let err = listed
        .expect_err("an unreadable protect file is an error, never an empty keep list")
        .to_string();
    assert!(
        err.contains("protection state unknown"),
        "`swamp protect list` must report the corruption: {err}"
    );

    let result = fx.execute(&plan);
    assert_refused(&result, &fx.cache, &["protection state unknown"]);
}

#[test]
fn every_refusal_is_recorded_in_the_ledger_as_unchanged() {
    let _serial = serial();
    let fx = fixture();
    let plan = fx.approved_plan();
    swamp_core::agents::protect_add(fx.store.path(), &fx.member).unwrap();
    let result = fx.execute(&plan);
    assert_refused(&result, &fx.cache, &["contains human-protected"]);
    let recorded = fx.ledger_outcomes();
    assert!(
        !recorded.is_empty(),
        "a refusal is a durable outcome, not just a returned string"
    );
    assert!(
        recorded
            .iter()
            .all(|(outcome, state)| outcome != "completed" && state.as_deref() != Some("trashed")),
        "the ledger must record the refusal, never a completion: {recorded:?}"
    );
}

#[test]
fn an_unchanged_reviewed_unit_still_executes() {
    let _serial = serial();
    // The control. A recheck that refuses everything is not a safety
    // boundary, it is a broken tool.
    let fx = fixture();
    let plan = fx.approved_plan();
    let result = fx.execute(&plan);
    let outcome = &result.outcomes[0];
    assert_eq!(
        outcome.status, "completed",
        "an unchanged, unprotected, unoccupied reviewed unit must still be actionable: {:?}",
        outcome.cause
    );
    assert!(!fx.cache.exists(), "the reviewed unit moved to Trash");
    assert!(outcome.recovery_location.is_some(), "recovery is recorded");
}
