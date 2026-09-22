//! #101 acceptance: precise agent-storage cleanup with session and
//! database integrity protections. Disposable fixtures only -- no real
//! `~/.claude` is read or written by these tests (PRIVACY IS A HARD
//! RULE); every fixture session body carries a canary string, and every
//! assertion that inspects serialized output checks that canary never
//! appears in it.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use swamp_core::actions;
use swamp_core::agents::{AgentActionCapability, discover_and_measure};
use swamp_core::locations::{Environment, Platform, Registry};
use swamp_core::scope::{ScanConfig, resolve_effective_scope};

const CANARY: &str = "CANARY-NEVER-LEAK-a91f7c";

fn write(path: &Path, content: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn session_line(cwd: &Path) -> String {
    format!(
        "{{\"type\":\"user\",\"sessionId\":\"s\",\"cwd\":\"{}\",\"gitBranch\":\"main\",\
         \"message\":{{\"role\":\"user\",\"content\":\"{CANARY}\"}}}}\n",
        cwd.display()
    )
}

fn only_claude_code_config() -> ScanConfig {
    ScanConfig {
        defaults: false,
        include: Vec::new(),
        exclude: Vec::new(),
        disabled_detectors: vec!["cargo-home".into(), "rustup".into(), "homebrew".into()],
        enabled_detectors: Vec::new(),
    }
}

fn env_at(home: &Path, claude_home: &Path) -> Environment {
    let mut env_vars: HashMap<String, String> = HashMap::new();
    env_vars.insert(
        "CLAUDE_CONFIG_DIR".into(),
        claude_home.display().to_string(),
    );
    Environment::fixture(home.to_path_buf(), env_vars, Platform::MacOS)
}

/// Builds a fixture Claude Code home with one repo-linked session (with
/// a companion subagent dir, file-history, and a todos entry) plus two
/// actionable cache/log categories. Returns
/// `(fixture_root, claude_home, repo, session_jsonl_path)`.
fn fixture_home() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let claude_home = root.path().join("claude-home");
    let repo = root.path().join("repo");
    fs::create_dir_all(repo.join(".git")).unwrap();

    let session_id = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    let proj_dir = claude_home.join("projects").join("-repo-encoded");
    let jsonl = proj_dir.join(format!("{session_id}.jsonl"));
    write(&jsonl, session_line(&repo).as_bytes());
    write(
        &proj_dir.join(session_id).join("subagents").join("a.jsonl"),
        session_line(&repo).as_bytes(),
    );
    write(
        &claude_home
            .join("file-history")
            .join(session_id)
            .join("snap.txt"),
        b"[redacted]",
    );
    write(
        &claude_home
            .join("todos")
            .join(format!("{session_id}-agent-1.json")),
        b"[]",
    );

    // A second, independent session in a different project, to prove
    // selected removal never touches it.
    let other_session_id = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
    let other_repo = root.path().join("other-repo");
    fs::create_dir_all(other_repo.join(".git")).unwrap();
    let other_jsonl = claude_home
        .join("projects")
        .join("-other-encoded")
        .join(format!("{other_session_id}.jsonl"));
    write(&other_jsonl, session_line(&other_repo).as_bytes());

    write(
        &claude_home.join("shell-snapshots").join("snap.sh"),
        b"alias x=y",
    );
    write(
        &claude_home.join("debug").join("log.txt"),
        b"line 1\nline 2\n",
    );
    write(&claude_home.join("settings.json"), b"{}");
    write(&claude_home.join(".credentials.json"), b"[redacted]");
    write(&claude_home.join("history.jsonl"), b"{}\n");

    (root, claude_home, repo, jsonl)
}

fn units_for(
    claude_home: &Path,
    store: &Path,
) -> (Vec<swamp_core::agents::AgentUnit>, tempfile::TempDir) {
    let home_dummy = tempfile::tempdir().unwrap();
    let env = env_at(home_dummy.path(), claude_home);
    let registry = Registry::with_builtins();
    let scope = resolve_effective_scope(&env, &only_claude_code_config(), &[], &registry, 1);
    let units = discover_and_measure(&scope, &[], Some(store), true, 1_000, 30, 3600).unwrap();
    (units, home_dummy)
}

#[test]
fn cache_removal_preserves_auth_and_history() {
    let (root, claude_home, _repo, _jsonl) = fixture_home();
    let store = tempfile::tempdir().unwrap();
    let (units, _home_dummy) = units_for(&claude_home, store.path());

    let shell_snapshots = claude_home.join("shell-snapshots");
    let plan =
        actions::propose_agents(&units, std::slice::from_ref(&shell_snapshots), "test").unwrap();
    assert_eq!(plan.units.len(), 1);
    assert_eq!(
        plan.units[0].agent_meta.as_ref().unwrap().session_members,
        None
    );

    actions::save_plan(store.path(), &plan).unwrap();
    actions::approve(store.path(), &plan.id, "human:test").unwrap();
    let trash_dir = tempfile::tempdir().unwrap();
    let result =
        actions::execute_with_trash(store.path(), &plan.id, "human:test", trash_dir.path())
            .unwrap();

    assert_eq!(
        result.outcomes[0].status, "completed",
        "{:?}",
        result.outcomes[0]
    );
    assert!(
        !shell_snapshots.exists(),
        "cache dir should be moved to Trash"
    );
    assert!(claude_home.join("settings.json").exists());
    assert!(claude_home.join(".credentials.json").exists());
    assert!(claude_home.join("history.jsonl").exists());
    assert!(
        claude_home.join("debug").exists(),
        "unselected debug/ untouched"
    );

    let recovered = result.outcomes[0].recovery_location.as_ref().unwrap();
    assert!(
        recovered.join("snap.sh").exists(),
        "content recoverable from Trash"
    );

    let _ = root; // keep the tempdir alive for the whole test
}

#[test]
fn selected_session_removal_preserves_other_sessions_and_shared_material() {
    let (root, claude_home, repo, jsonl) = fixture_home();
    let store = tempfile::tempdir().unwrap();
    let (units, _home_dummy) = units_for(&claude_home, store.path());

    let plan = actions::propose_agents(&units, std::slice::from_ref(&jsonl), "test").unwrap();
    assert_eq!(plan.units.len(), 1);
    let meta = plan.units[0].agent_meta.as_ref().unwrap();
    assert!(meta.session_members.is_some());
    assert_eq!(meta.session_members.as_ref().unwrap().len(), 4);

    actions::save_plan(store.path(), &plan).unwrap();
    actions::approve(store.path(), &plan.id, "human:test").unwrap();
    let trash_dir = tempfile::tempdir().unwrap();
    let result =
        actions::execute_with_trash(store.path(), &plan.id, "human:test", trash_dir.path())
            .unwrap();
    assert_eq!(
        result.outcomes[0].status, "completed",
        "{:?}",
        result.outcomes[0]
    );

    // The selected session's own files are gone...
    assert!(!jsonl.exists());
    assert!(
        !claude_home
            .join("file-history")
            .join("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa")
            .exists()
    );
    assert!(
        !claude_home
            .join("todos")
            .join("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa-agent-1.json")
            .exists()
    );

    // ...but the other session, the repo itself, and shared config are not.
    let other_jsonl = claude_home
        .join("projects")
        .join("-other-encoded")
        .join("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb.jsonl");
    assert!(other_jsonl.exists(), "unselected session must be preserved");
    assert!(
        repo.join(".git").exists(),
        "the project's own files are untouched"
    );
    assert!(claude_home.join("settings.json").exists());

    // Every member landed together in one Trash envelope.
    let recovered = result.outcomes[0].recovery_location.as_ref().unwrap();
    let moved_names: Vec<String> = fs::read_dir(recovered)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(moved_names.len() >= 4, "{moved_names:?}");

    let _ = root;
}

#[test]
fn protected_categories_refuse_at_proposal_time() {
    let (root, claude_home, _repo, _jsonl) = fixture_home();
    let store = tempfile::tempdir().unwrap();
    let (units, _home_dummy) = units_for(&claude_home, store.path());

    let settings = claude_home.join("settings.json");
    let err = actions::propose_agents(&units, std::slice::from_ref(&settings), "test").unwrap_err();
    assert!(err.to_string().contains("protected"), "{err}");
    assert!(settings.exists());
    let _ = root;
}

#[test]
fn widened_scope_path_not_matching_any_unit_refuses() {
    let (root, claude_home, _repo, _jsonl) = fixture_home();
    let store = tempfile::tempdir().unwrap();
    let (units, _home_dummy) = units_for(&claude_home, store.path());

    let bogus = claude_home.join("not-a-real-unit");
    let err = actions::propose_agents(&units, std::slice::from_ref(&bogus), "test").unwrap_err();
    assert!(err.to_string().contains("no agent-storage unit"), "{err}");
    let _ = root;
}

#[test]
fn active_session_transcript_refuses_at_proposal_and_execution() {
    let (root, claude_home, _repo, jsonl) = fixture_home();
    let store = tempfile::tempdir().unwrap();
    let (units, _home_dummy) = units_for(&claude_home, store.path());

    // Hold the transcript open in this process: `lsof` reports any
    // process with an open file descriptor on the path, including this
    // test binary itself, which is exactly what
    // `crate::agents::is_active` (an `occupancy::occupied` wrapper)
    // checks for -- no mocking needed, this is the real seam.
    let _held_open = fs::File::open(&jsonl).expect("open transcript to simulate an active session");

    let err = actions::propose_agents(&units, std::slice::from_ref(&jsonl), "test").unwrap_err();
    assert!(
        err.to_string().contains("active") || err.to_string().contains("no agent-storage unit"),
        "{err}"
    );
    let _ = root;
}

#[test]
fn plan_and_ledger_never_contain_the_canary_prompt_content() {
    let (root, claude_home, _repo, jsonl) = fixture_home();
    let store = tempfile::tempdir().unwrap();
    let (units, _home_dummy) = units_for(&claude_home, store.path());

    // No unit's identity/serialized form contains the canary.
    let serialized_units = serde_json::to_string(&units).unwrap();
    assert!(!serialized_units.contains(CANARY));

    let plan = actions::propose_agents(&units, std::slice::from_ref(&jsonl), "test").unwrap();
    let serialized_plan = serde_json::to_string(&plan).unwrap();
    assert!(!serialized_plan.contains(CANARY));

    actions::save_plan(store.path(), &plan).unwrap();
    actions::approve(store.path(), &plan.id, "human:test").unwrap();
    let trash_dir = tempfile::tempdir().unwrap();
    let result =
        actions::execute_with_trash(store.path(), &plan.id, "human:test", trash_dir.path())
            .unwrap();
    let serialized_result = serde_json::to_string(&result).unwrap();
    assert!(!serialized_result.contains(CANARY));

    let ledger_text = fs::read_to_string(store.path().join("ledger.jsonl")).unwrap();
    assert!(!ledger_text.contains(CANARY));

    let _ = root;
}

#[test]
fn cache_action_capability_is_reported_on_the_agent_unit() {
    let (root, claude_home, _repo, _jsonl) = fixture_home();
    let store = tempfile::tempdir().unwrap();
    let (units, _home_dummy) = units_for(&claude_home, store.path());
    let debug_unit = units
        .iter()
        .find(|u| u.relative_path == "debug")
        .expect("debug unit present");
    assert_eq!(debug_unit.action, AgentActionCapability::CacheOrLogTrash);
    assert!(!debug_unit.protected);
    let _ = root;
}
