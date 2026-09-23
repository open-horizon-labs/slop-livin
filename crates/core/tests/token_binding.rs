//! Re-review 5, findings 1-4 and 7, at runtime: every shortcut the
//! static review named, driven through the real API against disposable
//! fixtures. Each test asserts the same two things: the action is
//! **refused**, and the fixture data is **untouched** (still at its
//! path, same bytes). The compile-time halves are the `compile_fail`
//! cases named in each test's doc.
//!
//! No real home, tool store or swamp state is read; every path is under a
//! per-test `tempfile` directory.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use swamp_core::actions::{self, approve, execute_with_trash, save_plan};
use swamp_core::authority::{
    Confirmable, HumanConfirmed, ProtectChange, SelectedUnit, StandingTerms, authorize_confirmed,
};
use swamp_core::fs_gate::StoreDir;
use swamp_core::locations::{Environment, Platform, Registry};
use swamp_core::scope::{ScanConfig, resolve_effective_scope};

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

fn write(path: &Path, content: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

/// A synthetic Claude Code home with two cache categories (`debug/`,
/// `shell-snapshots/`) and one session with sidecar members.
struct Home {
    _tmp: tempfile::TempDir,
    home: PathBuf,
    debug: PathBuf,
    snapshots: PathBuf,
    session: PathBuf,
    todos: PathBuf,
    units: Vec<swamp_core::agents::AgentUnit>,
}

const SESSION: &str = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";

fn home(store: &Path) -> Home {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("claude");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(repo.join(".git")).unwrap();
    write(&home.join("debug/log.txt"), b"debug-fixture");
    write(&home.join("shell-snapshots/snap.sh"), b"snapshot-fixture");
    let session = home
        .join("projects/-repo-encoded")
        .join(format!("{SESSION}.jsonl"));
    write(
        &session,
        format!(
            "{{\"type\":\"user\",\"sessionId\":\"s\",\"cwd\":\"{}\",\"gitBranch\":\"main\"}}\n",
            repo.display()
        )
        .as_bytes(),
    );
    write(
        &home.join("file-history").join(SESSION).join("snap.txt"),
        b"history-fixture",
    );
    let todos = home.join("todos").join(format!("{SESSION}-agent-1.json"));
    write(&todos, b"[\"todo-fixture\"]");
    write(&home.join("settings.json"), b"{}");
    let env = Environment::fixture(
        tmp.path().to_path_buf(),
        HashMap::from([("CLAUDE_CONFIG_DIR".to_string(), home.display().to_string())]),
        Platform::MacOS,
    );
    let scope =
        resolve_effective_scope(&env, &only_claude(), &[], &Registry::with_builtins(), 1_000);
    let units = swamp_core::agents::discover_and_measure(
        &scope,
        &[],
        Some(store),
        true,
        1_000,
        30,
        3600,
        &swamp_core::fs_events::EventCoverage::untrusted(),
    )
    .unwrap();
    Home {
        debug: home.join("debug"),
        snapshots: home.join("shell-snapshots"),
        session,
        todos,
        home,
        units,
        _tmp: tmp,
    }
}

fn propose(h: &Home, path: &Path) -> actions::Plan {
    actions::propose_agents(&h.units, &[path.to_path_buf()], "agent:fixture").unwrap()
}

fn untouched(path: &Path, expect: &[u8]) {
    assert_eq!(
        fs::read(path).unwrap_or_else(|e| panic!("{} was moved: {e}", path.display())),
        expect,
        "{} changed",
        path.display()
    );
}

fn plan_file(store: &Path, id: &str) -> PathBuf {
    store.join("plans").join(format!("{id}.json"))
}

// ---------------------------------------------------------------------
// Finding 1: authorization provenance
// ---------------------------------------------------------------------

/// A plan file edited after approval -- a second unit spliced in, the
/// shortcut an approval-by-plan-id-alone left open -- no longer loads:
/// its keyed binding does not match. Compile-time half:
/// `plan_content_cannot_be_edited`, `plan_is_not_deserialize`.
#[test]
fn a_plan_edited_after_approval_does_not_execute() {
    let store = tempfile::tempdir().unwrap();
    let h = home(store.path());
    let trash = tempfile::tempdir().unwrap();
    let plan = propose(&h, &h.debug);
    save_plan(store.path(), &plan).unwrap();
    approve(store.path(), &plan.id, "human:fixture").unwrap();

    // Splice the other cache into the approved plan, on disk.
    let other = propose(&h, &h.snapshots);
    let file = plan_file(store.path(), &plan.id);
    let mut record: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
    let other_unit = serde_json::to_value(&other).unwrap()["units"][0].clone();
    record["units"].as_array_mut().unwrap().push(other_unit);
    fs::write(&file, serde_json::to_vec_pretty(&record).unwrap()).unwrap();

    let err = execute_with_trash(store.path(), &plan.id, "agent:fixture", trash.path())
        .expect_err("an edited plan must not load");
    assert!(err.to_string().contains("binding"), "{err}");
    untouched(&h.debug.join("log.txt"), b"debug-fixture");
    untouched(&h.snapshots.join("snap.sh"), b"snapshot-fixture");
}

/// Swamp's own code re-saving *different* content under an approved
/// plan's id (the public `id` field renamed) produces a validly bound
/// plan -- and the one-shot grant, which recorded the approved content's
/// digest, does not cover it.
#[test]
fn an_approval_does_not_cover_different_content_under_the_same_id() {
    let store = tempfile::tempdir().unwrap();
    let h = home(store.path());
    let trash = tempfile::tempdir().unwrap();
    let approved = propose(&h, &h.debug);
    save_plan(store.path(), &approved).unwrap();
    approve(store.path(), &approved.id, "human:fixture").unwrap();

    let mut swapped = propose(&h, &h.snapshots);
    swapped.id = approved.id.clone();
    save_plan(store.path(), &swapped).unwrap();

    let res =
        execute_with_trash(store.path(), &approved.id, "agent:fixture", trash.path()).unwrap();
    assert_eq!(res.state, "awaiting-authorization", "{res:?}");
    untouched(&h.snapshots.join("snap.sh"), b"snapshot-fixture");
    untouched(&h.debug.join("log.txt"), b"debug-fixture");
}

/// A grant widened by hand -- budget, unit cap and expiry raised in
/// `grants.json` -- fails its binding, and the loader refuses the whole
/// file: no execution proceeds on it, not even the plan it was for.
/// Compile-time half: `grant_is_not_a_struct_literal`,
/// `grant_is_not_deserialize`, `authorize_is_not_callable_outside_core`.
#[test]
fn a_hand_edited_grant_refuses_every_execution() {
    let store = tempfile::tempdir().unwrap();
    let h = home(store.path());
    let trash = tempfile::tempdir().unwrap();
    let plan = propose(&h, &h.debug);
    save_plan(store.path(), &plan).unwrap();
    approve(store.path(), &plan.id, "human:fixture").unwrap();

    let grants = store.path().join("grants.json");
    let mut file: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&grants).unwrap()).unwrap();
    let g = &mut file["grants"][0];
    g["budget_bytes"] = serde_json::json!(u64::MAX);
    g["max_units"] = serde_json::json!(u32::MAX);
    g["expires_at"] = serde_json::json!(u64::MAX);
    fs::write(&grants, serde_json::to_vec_pretty(&file).unwrap()).unwrap();

    let err = execute_with_trash(store.path(), &plan.id, "agent:fixture", trash.path())
        .expect_err("a widened grant must refuse");
    assert!(err.to_string().contains("binding"), "{err}");
    untouched(&h.debug.join("log.txt"), b"debug-fixture");
}

/// A grant appended by something other than swamp (no binding at all)
/// refuses too: it is not skipped quietly next to real ones.
#[test]
fn a_grant_written_outside_swamp_refuses() {
    let store = tempfile::tempdir().unwrap();
    let h = home(store.path());
    let trash = tempfile::tempdir().unwrap();
    let plan = propose(&h, &h.debug);
    save_plan(store.path(), &plan).unwrap();
    fs::write(
        store.path().join("grants.json"),
        serde_json::to_vec(&serde_json::json!({"grants": [{
            "id": "forged", "verb": "delete", "predicate": "", "plan_id": plan.id,
            "budget_bytes": u64::MAX, "spent_bytes": 0, "max_units": 99, "used_units": 0,
            "created_at": 0, "expires_at": u64::MAX, "actor": "agent", "revoked": false,
            "confirmation": "none", "site": "cli-approve"
        }]}))
        .unwrap(),
    )
    .unwrap();
    assert!(execute_with_trash(store.path(), &plan.id, "agent:fixture", trash.path()).is_err());
    untouched(&h.debug.join("log.txt"), b"debug-fixture");
}

/// Plan and grant copied into another store: that store's key did not
/// bind them, so nothing loads there -- the store an execution reads
/// protection from is always the store its plan and grant came from
/// (finding 3's "a store dir that does not match the plan's store
/// refuses").
#[test]
fn a_plan_and_grant_copied_into_another_store_do_not_load() {
    let store = tempfile::tempdir().unwrap();
    let h = home(store.path());
    let trash = tempfile::tempdir().unwrap();
    let plan = propose(&h, &h.debug);
    save_plan(store.path(), &plan).unwrap();
    approve(store.path(), &plan.id, "human:fixture").unwrap();

    // The other store has a key of its own (it has seen a plan).
    let other = tempfile::tempdir().unwrap();
    save_plan(other.path(), &propose(&h, &h.snapshots)).unwrap();
    fs::copy(
        plan_file(store.path(), &plan.id),
        plan_file(other.path(), &plan.id),
    )
    .unwrap();
    fs::copy(
        store.path().join("grants.json"),
        other.path().join("grants.json"),
    )
    .unwrap();
    assert!(execute_with_trash(other.path(), &plan.id, "agent:fixture", trash.path()).is_err());

    // And a store with no key at all refuses the copied files as well.
    let bare = tempfile::tempdir().unwrap();
    fs::create_dir_all(bare.path().join("plans")).unwrap();
    fs::copy(
        plan_file(store.path(), &plan.id),
        plan_file(bare.path(), &plan.id),
    )
    .unwrap();
    let err = actions::load_plan(bare.path(), &plan.id).unwrap_err();
    assert!(err.to_string().contains("authority key"), "{err}");
    untouched(&h.debug.join("log.txt"), b"debug-fixture");
}

// ---------------------------------------------------------------------
// Finding 2: a confirmation binds what was confirmed, once
// ---------------------------------------------------------------------

/// A confirmation minted while plan A was shown does not approve plan B,
/// and writes no grant. Compile-time half:
/// `human_confirmation_is_spent_once`,
/// `human_confirmation_names_what_was_confirmed`.
#[test]
fn a_confirmation_for_one_plan_does_not_approve_another() {
    let store = tempfile::tempdir().unwrap();
    let h = home(store.path());
    let trash = tempfile::tempdir().unwrap();
    let shown = propose(&h, &h.debug);
    let other = propose(&h, &h.snapshots);
    save_plan(store.path(), &shown).unwrap();
    save_plan(store.path(), &other).unwrap();

    let confirmed = HumanConfirmed::cli_approve("human:fixture", &shown);
    let err = actions::approve_confirmed(store.path(), &other.id, confirmed).unwrap_err();
    assert!(err.to_string().contains("not for plan"), "{err}");
    assert!(actions::list_grants(store.path()).unwrap().is_empty());
    let res = execute_with_trash(store.path(), &other.id, "agent:fixture", trash.path()).unwrap();
    assert_eq!(res.state, "awaiting-authorization");
    untouched(&h.snapshots.join("snap.sh"), b"snapshot-fixture");
}

/// A plan whose stored content changed between being shown and being
/// approved is refused: the confirmation carries the digest of what was
/// shown.
#[test]
fn a_confirmation_refuses_a_plan_that_changed_after_it_was_shown() {
    let store = tempfile::tempdir().unwrap();
    let h = home(store.path());
    let shown = propose(&h, &h.debug);
    save_plan(store.path(), &shown).unwrap();
    let confirmed = HumanConfirmed::cli_approve("human:fixture", &shown);

    let mut swapped = propose(&h, &h.snapshots);
    swapped.id = shown.id.clone();
    save_plan(store.path(), &swapped).unwrap();

    assert!(actions::approve_confirmed(store.path(), &shown.id, confirmed).is_err());
    assert!(actions::list_grants(store.path()).unwrap().is_empty());
    untouched(&h.snapshots.join("snap.sh"), b"snapshot-fixture");
}

/// Standing-grant terms are bound too: a confirmation for a small,
/// short grant does not mint a large, long one.
#[test]
fn a_standing_grant_confirmation_binds_its_terms() {
    let store = tempfile::tempdir().unwrap();
    let confirmed = HumanConfirmed::cli_grant(
        "human:fixture",
        StandingTerms {
            predicate: "kind:Cache".into(),
            budget_bytes: 1024,
            max_units: Some(1),
            expires_in_secs: 60,
        },
    );
    assert!(
        actions::add_standing_grant_confirmed(
            store.path(),
            "kind:Cache",
            u64::MAX,
            None,
            u64::MAX / 4,
            confirmed,
        )
        .is_err()
    );
    assert!(actions::list_grants(store.path()).unwrap().is_empty());
}

/// One TUI confirmation authorizes the one path it names: asking it for
/// a different path refuses, and nothing moves.
#[test]
fn a_tui_confirmation_refuses_a_path_it_does_not_name() {
    let store = tempfile::tempdir().unwrap();
    let h = home(store.path());
    let store_dir = StoreDir::at(store.path()).unwrap();
    let mut confirmed = HumanConfirmed::tui_dialog(
        "human",
        &store_dir,
        vec![Confirmable::Unit(SelectedUnit {
            path: h.debug.clone(),
            reviewed: swamp_core::recheck::capture_anchor(&h.debug).ok(),
            docker: None,
            linked_worktree: false,
            preserve_into: None,
        })],
    );
    let err = authorize_confirmed(confirmed.remove(0), &h.snapshots).unwrap_err();
    assert!(err.to_string().contains("names"), "{err}");
    untouched(&h.snapshots.join("snap.sh"), b"snapshot-fixture");

    // A plan confirmation is not a unit authorization either.
    let plan = propose(&h, &h.debug);
    let mut as_plan =
        HumanConfirmed::tui_dialog("human", &store_dir, vec![Confirmable::Plan(&plan)]);
    assert!(authorize_confirmed(as_plan.remove(0), &h.debug).is_err());
    // Nor is a CLI approval.
    assert!(authorize_confirmed(HumanConfirmed::cli_approve("human", &plan), &h.debug).is_err());
    untouched(&h.debug.join("log.txt"), b"debug-fixture");
}

// ---------------------------------------------------------------------
// Finding 3: the recheck's inputs come from the authorization
// ---------------------------------------------------------------------

/// A session's sidecar (its `todos` file, outside the anchor) replaced
/// after review -- same path, same name, new file -- refuses the whole
/// session removal. The old recheck added caller-listed members to its
/// coverage without comparing their identity, so only a path match was
/// required. Compile-time half: `recheck_inputs_come_from_the_authorization`,
/// `capture_is_private_to_the_propose_path`.
#[test]
fn a_sidecar_member_replaced_after_review_refuses_the_session_removal() {
    let store = tempfile::tempdir().unwrap();
    let h = home(store.path());
    let trash = tempfile::tempdir().unwrap();
    let plan = propose(&h, &h.session);
    assert!(
        plan.units()[0]
            .agent_meta()
            .and_then(|m| m.session_members.as_ref())
            .is_some_and(|m| m.contains(&h.todos)),
        "fixture: the todos file is a session member"
    );
    save_plan(store.path(), &plan).unwrap();
    approve(store.path(), &plan.id, "human:fixture").unwrap();

    // Replace the sidecar with a different file under the same name.
    let aside = h.home.join("aside.json");
    fs::rename(&h.todos, &aside).unwrap();
    write(&h.todos, b"[\"todo-fixture\"]");

    let res = execute_with_trash(store.path(), &plan.id, "agent:fixture", trash.path()).unwrap();
    assert_ne!(res.outcomes[0].status, "completed", "{res:?}");
    assert!(
        res.outcomes[0]
            .cause
            .as_deref()
            .is_some_and(|c| c.contains("reviewed") || c.contains("replaced")),
        "{:?}",
        res.outcomes[0].cause
    );
    assert!(h.session.exists(), "the session anchor must not move");
    untouched(&h.todos, b"[\"todo-fixture\"]");
    untouched(
        &h.home.join("file-history").join(SESSION).join("snap.txt"),
        b"history-fixture",
    );
    assert_eq!(fs::read_dir(trash.path()).unwrap().count(), 0);
}

/// Protection is read from the store the authorization came from. A
/// protect entry in the plan's store stops the execution even though a
/// caller has no way to point the recheck at another directory.
#[test]
fn protection_is_read_from_the_authorizations_store() {
    let store = tempfile::tempdir().unwrap();
    let h = home(store.path());
    let trash = tempfile::tempdir().unwrap();
    let plan = propose(&h, &h.debug);
    save_plan(store.path(), &plan).unwrap();
    approve(store.path(), &plan.id, "human:fixture").unwrap();
    swamp_core::agents::protect_add(store.path(), &h.debug.join("log.txt")).unwrap();
    let res = execute_with_trash(store.path(), &plan.id, "agent:fixture", trash.path()).unwrap();
    assert_ne!(res.outcomes[0].status, "completed", "{res:?}");
    untouched(&h.debug.join("log.txt"), b"debug-fixture");
}

// ---------------------------------------------------------------------
// Finding 4: no pass-throughs in the gate
// ---------------------------------------------------------------------

/// The ledger appends only to a `ledger.jsonl`: a caller cannot point it
/// at another file. Compile-time half: `logs_are_not_caller_paths`,
/// `store_files_live_in_a_store_dir`.
#[test]
fn the_ledger_appends_to_nothing_but_a_ledger() {
    let tmp = tempfile::tempdir().unwrap();
    let rc = tmp.path().join(".zshrc");
    fs::write(&rc, b"export KEEP=1\n").unwrap();
    assert!(swamp_core::ledger::Ledger::open(&rc).is_err());
    untouched(&rc, b"export KEEP=1\n");
}

/// A store directory is absolute and a real directory: a relative path,
/// a regular file and a symlink are refused before anything is written.
#[test]
fn a_store_dir_is_a_real_absolute_directory() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(StoreDir::at(Path::new("relative/store")).is_err());
    let file = tmp.path().join("file");
    fs::write(&file, b"keep").unwrap();
    assert!(StoreDir::at(&file).is_err());
    let target = tmp.path().join("real");
    fs::create_dir(&target).unwrap();
    let link = tmp.path().join("link");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert!(StoreDir::at(&link).is_err());
    untouched(&file, b"keep");
    assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
}

/// The destructive verbs build their arguments from the proof and the
/// authorization. With a path proof in hand, `docker rm` refuses (and
/// spawns nothing); the executable copy refuses when the authorization
/// records no worktree, or the source is outside the rechecked unit; and
/// `git worktree prune` refuses the receipt of a move that was not a
/// linked worktree's. Compile-time half: `destructive_verbs_need_a_proof`,
/// `trash_receipt_is_minted_by_the_move`.
#[test]
fn destructive_verbs_refuse_what_the_proof_does_not_cover() {
    let store = tempfile::tempdir().unwrap();
    let h = home(store.path());
    let trash = tempfile::tempdir().unwrap();
    let store_dir = StoreDir::at(store.path()).unwrap();
    let confirm = |path: &Path| {
        HumanConfirmed::tui_dialog(
            "human",
            &store_dir,
            vec![Confirmable::Unit(SelectedUnit {
                path: path.to_path_buf(),
                reviewed: swamp_core::recheck::capture_anchor(path).ok(),
                docker: None,
                linked_worktree: false,
                preserve_into: None,
            })],
        )
        .remove(0)
    };

    let auth = authorize_confirmed(confirm(&h.debug), &h.debug).unwrap();
    let proof = swamp_core::recheck::run_all(&auth).unwrap();
    // No worktree recorded: nothing is copied anywhere.
    assert!(
        swamp_core::fs_gate::destroy::copy_preserved(&proof, &auth, &h.debug.join("log.txt"), None)
            .is_err()
    );
    // Outside the rechecked unit.
    assert!(
        swamp_core::fs_gate::destroy::copy_preserved(
            &proof,
            &auth,
            &h.home.join("settings.json"),
            None
        )
        .is_err()
    );
    let (refused, counted) = swamp_core::work_counters::measured(|| {
        swamp_core::fs_gate::destroy::docker_remove(proof, &auth)
    });
    assert!(refused.is_err());
    assert_eq!(counted.subprocess_spawns, 0);
    untouched(&h.debug.join("log.txt"), b"debug-fixture");
    assert!(!h.home.join("bin").exists());

    // A licensed move of a plain directory (into the disposable Trash),
    // then a prune with its receipt: refused, nothing spawned.
    let auth = authorize_confirmed(confirm(&h.snapshots), &h.snapshots).unwrap();
    let proof = swamp_core::recheck::run_all(&auth).unwrap();
    let moved =
        swamp_core::fs_gate::destroy::trash_move(proof, &auth, trash.path(), "snap").unwrap();
    assert!(moved.path().join("snap.sh").exists());
    let (refused, counted) = swamp_core::work_counters::measured(|| {
        swamp_core::fs_gate::destroy::git_worktree_prune(moved, &auth)
    });
    assert!(refused.is_err());
    assert_eq!(counted.subprocess_spawns, 0);
}

// ---------------------------------------------------------------------
// Finding 7: protection changes are human-only
// ---------------------------------------------------------------------

/// A protect confirmation names one change: a confirmation to protect A
/// does not remove B's protection, nor protect B, and the keep list is
/// unchanged. Compile-time half: `protect_changes_are_human_only`,
/// `protect_listing_is_display_only`.
#[test]
fn a_protect_confirmation_names_one_change() {
    let store = tempfile::tempdir().unwrap();
    let a = store.path().join("a");
    let b = store.path().join("b");
    swamp_core::agents::protect_add(store.path(), &b).unwrap();
    let before = fs::read(store.path().join("agent_protect.json")).unwrap();

    let for_a = || HumanConfirmed::cli_protect("human:cli", ProtectChange::Add(a.clone()));
    assert!(swamp_core::agents::protect_remove_confirmed(store.path(), &b, for_a()).is_err());
    assert!(swamp_core::agents::protect_add_confirmed(store.path(), &b, for_a()).is_err());
    // And a plan approval is not a protect confirmation.
    let h = home(store.path());
    let plan = propose(&h, &h.debug);
    assert!(
        swamp_core::agents::protect_remove_confirmed(
            store.path(),
            &b,
            HumanConfirmed::cli_approve("human:cli", &plan),
        )
        .is_err()
    );
    assert_eq!(
        fs::read(store.path().join("agent_protect.json")).unwrap(),
        before
    );
    // The right confirmation does what it names.
    swamp_core::agents::protect_add_confirmed(store.path(), &a, for_a()).unwrap();
    assert_eq!(
        swamp_core::agents::protect_listing(store.path())
            .unwrap()
            .len(),
        2
    );
}
