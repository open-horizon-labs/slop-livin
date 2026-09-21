//! #101 acceptance, mirroring `agent_units_actions.rs`'s Claude Code
//! coverage for the three adapters #93/#94/#95 added: Codex, Oh My Pi
//! and OpenCode. Disposable fixtures only -- no real tool home is read
//! or written by these tests (PRIVACY IS A HARD RULE); every fixture
//! session body carries a canary string, and every assertion that
//! inspects serialized output checks that canary never appears in it.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use swamp_core::actions;
use swamp_core::agents::{AgentActionCapability, discover_and_measure};
use swamp_core::locations::{Environment, Platform, Registry};
use swamp_core::scope::{ScanConfig, resolve_effective_scope};

const CANARY: &str = "CANARY-NEVER-LEAK-b7e21f";

fn write(path: &Path, content: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn only_detector(id: &str) -> ScanConfig {
    ScanConfig {
        defaults: false,
        include: Vec::new(),
        exclude: Vec::new(),
        // Every other named agent-tool detector must be disabled here,
        // not only the four this file's fixtures originally covered
        // (#93/#94/#95): Pi and Oh My Pi share `PI_CODING_AGENT_DIR` as
        // a disclosed collision-risk override (see
        // `crate::locations::pi`'s own doc comment), so an
        // `only_detector("oh-my-pi")` fixture that also left "pi"
        // enabled had both adapters independently identify the exact
        // same fixture session -- caught by `propose_agents`'s own
        // overlap refusal (#101's refusal-matrix hardening), not a bug
        // in that refusal.
        disabled_detectors: ["cargo-home", "rustup", "homebrew"]
            .into_iter()
            .chain(
                [
                    "claude-code",
                    "codex",
                    "codex-desktop",
                    "oh-my-pi",
                    "opencode",
                    "gemini-cli",
                    "pi",
                    "aider",
                    "github-copilot-cli",
                    "cursor",
                    "windsurf",
                    "cline",
                    "roo-code",
                    "continue",
                ]
                .into_iter()
                .filter(|d| *d != id),
            )
            .map(str::to_string)
            .collect(),
    }
}

fn units_for(
    env_vars: HashMap<String, String>,
    detector_id: &str,
) -> Vec<swamp_core::agents::AgentUnit> {
    let home_dummy = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let env = Environment::fixture(home_dummy.path().to_path_buf(), env_vars, Platform::MacOS);
    let registry = Registry::with_builtins();
    let scope = resolve_effective_scope(&env, &only_detector(detector_id), &[], &registry, 1);
    let units =
        discover_and_measure(&scope, &[], Some(store.path()), true, 1_000, 30, 3600).unwrap();
    let _ = store; // keep tempdir alive for the call above
    units
}

// ---------------------------------------------------------------------
// Codex
// ---------------------------------------------------------------------

fn codex_header(cwd: &str, canary: &str) -> String {
    format!(
        "{{\"type\":\"session_meta\",\"cwd\":\"{cwd}\",\"payload\":{{\"prompt\":\"{canary}\"}}}}\n"
    )
}

fn codex_fixture() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("codex-home");
    let repo = root.path().join("repo");
    fs::create_dir_all(repo.join(".git")).unwrap();
    let jsonl = home.join(
        "sessions/2026/09/21/rollout-2026-09-21T10-00-00-11111111-1111-4111-8111-111111111111.jsonl",
    );
    write(
        &jsonl,
        codex_header(&repo.display().to_string(), CANARY).as_bytes(),
    );
    write(&home.join("log").join("codex.log"), b"debug line");
    write(&home.join("auth.json"), b"[redacted]");
    write(&home.join("config.toml"), b"[redacted]");
    (root, home, repo, jsonl)
}

#[test]
fn codex_cache_removal_preserves_auth_and_config() {
    let (root, home, _repo, _jsonl) = codex_fixture();
    let mut env_vars = HashMap::new();
    env_vars.insert("CODEX_HOME".to_string(), home.display().to_string());
    let units = units_for(env_vars, "codex");
    let store = tempfile::tempdir().unwrap();
    let log_dir = home.join("log");
    let plan = actions::propose_agents(&units, std::slice::from_ref(&log_dir), "test").unwrap();
    actions::save_plan(store.path(), &plan).unwrap();
    actions::approve(store.path(), &plan.id, "human:test").unwrap();
    let trash = tempfile::tempdir().unwrap();
    let result =
        actions::execute_with_trash(store.path(), &plan.id, "human:test", trash.path()).unwrap();
    assert_eq!(
        result.outcomes[0].status, "completed",
        "{:?}",
        result.outcomes[0]
    );
    assert!(!log_dir.exists());
    assert!(home.join("auth.json").exists());
    assert!(home.join("config.toml").exists());
    let _ = root;
}

#[test]
fn codex_session_removal_is_exactly_one_file_and_preserves_the_repo() {
    let (root, home, repo, jsonl) = codex_fixture();
    let mut env_vars = HashMap::new();
    env_vars.insert("CODEX_HOME".to_string(), home.display().to_string());
    let units = units_for(env_vars, "codex");
    let unit = units
        .iter()
        .find(|u| u.path == jsonl)
        .expect("session unit");
    assert_eq!(unit.action, AgentActionCapability::SessionRemoval);
    assert_eq!(
        unit.members.len(),
        1,
        "no companion dir documented for Codex"
    );

    let store = tempfile::tempdir().unwrap();
    let plan = actions::propose_agents(&units, std::slice::from_ref(&jsonl), "test").unwrap();
    actions::save_plan(store.path(), &plan).unwrap();
    actions::approve(store.path(), &plan.id, "human:test").unwrap();
    let trash = tempfile::tempdir().unwrap();
    let result =
        actions::execute_with_trash(store.path(), &plan.id, "human:test", trash.path()).unwrap();
    assert_eq!(
        result.outcomes[0].status, "completed",
        "{:?}",
        result.outcomes[0]
    );
    assert!(!jsonl.exists());
    assert!(
        repo.join(".git").exists(),
        "the project's own files are untouched"
    );

    let serialized = serde_json::to_string(&units).unwrap();
    assert!(!serialized.contains(CANARY));
    let ledger_text = fs::read_to_string(store.path().join("ledger.jsonl")).unwrap();
    assert!(!ledger_text.contains(CANARY));
    let _ = root;
}

// ---------------------------------------------------------------------
// Oh My Pi
// ---------------------------------------------------------------------

fn omp_session_bytes(cwd: &str, canary: &str) -> Vec<u8> {
    let mut title = vec![b' '; 256];
    let title_json = b"{\"type\":\"title\"}";
    title[..title_json.len()].copy_from_slice(title_json);
    title[255] = b'\n';
    let header =
        format!("{{\"type\":\"session\",\"cwd\":\"{cwd}\",\"additionalDirectories\":[]}}\n");
    let entry =
        format!("{{\"id\":\"1\",\"parentId\":null,\"timestamp\":0,\"content\":\"{canary}\"}}\n");
    let mut out = title;
    out.extend_from_slice(header.as_bytes());
    out.extend_from_slice(entry.as_bytes());
    out
}

fn omp_fixture() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("omp-agent");
    let repo = root.path().join("repo");
    fs::create_dir_all(repo.join(".git")).unwrap();
    write(&home.join("config.yml"), b"providers: {}");
    let jsonl = home.join("sessions/-repo/1700000000_11111111-1111-4111-8111-111111111111.jsonl");
    write(
        &jsonl,
        &omp_session_bytes(&repo.display().to_string(), CANARY),
    );
    write(&home.join("terminal-sessions").join("t1"), b"breadcrumb");
    write(&home.join("agent.db"), b"sqlite-bytes");
    (root, home, repo, jsonl)
}

#[test]
fn oh_my_pi_cache_removal_preserves_config_and_auth_db() {
    let (root, home, _repo, _jsonl) = omp_fixture();
    let mut env_vars = HashMap::new();
    env_vars.insert(
        "PI_CODING_AGENT_DIR".to_string(),
        home.display().to_string(),
    );
    let units = units_for(env_vars, "oh-my-pi");
    let store = tempfile::tempdir().unwrap();
    let terminal_dir = home.join("terminal-sessions");
    let plan =
        actions::propose_agents(&units, std::slice::from_ref(&terminal_dir), "test").unwrap();
    actions::save_plan(store.path(), &plan).unwrap();
    actions::approve(store.path(), &plan.id, "human:test").unwrap();
    let trash = tempfile::tempdir().unwrap();
    let result =
        actions::execute_with_trash(store.path(), &plan.id, "human:test", trash.path()).unwrap();
    assert_eq!(
        result.outcomes[0].status, "completed",
        "{:?}",
        result.outcomes[0]
    );
    assert!(!terminal_dir.exists());
    assert!(home.join("config.yml").exists());
    assert!(home.join("agent.db").exists());
    let _ = root;
}

#[test]
fn oh_my_pi_session_removal_preserves_repo_and_blob_store() {
    let (root, home, repo, jsonl) = omp_fixture();
    let mut env_vars = HashMap::new();
    env_vars.insert(
        "PI_CODING_AGENT_DIR".to_string(),
        home.display().to_string(),
    );
    let units = units_for(env_vars.clone(), "oh-my-pi");
    let unit = units
        .iter()
        .find(|u| u.path == jsonl)
        .expect("session unit");
    assert_eq!(unit.action, AgentActionCapability::SessionRemoval);
    assert_eq!(unit.members.len(), 1, "a blob is never a session member");

    let store = tempfile::tempdir().unwrap();
    let plan = actions::propose_agents(&units, std::slice::from_ref(&jsonl), "test").unwrap();
    actions::save_plan(store.path(), &plan).unwrap();
    actions::approve(store.path(), &plan.id, "human:test").unwrap();
    let trash = tempfile::tempdir().unwrap();
    let result =
        actions::execute_with_trash(store.path(), &plan.id, "human:test", trash.path()).unwrap();
    assert_eq!(
        result.outcomes[0].status, "completed",
        "{:?}",
        result.outcomes[0]
    );
    assert!(!jsonl.exists());
    assert!(repo.join(".git").exists());

    let serialized = serde_json::to_string(&units).unwrap();
    assert!(!serialized.contains(CANARY));
    let ledger_text = fs::read_to_string(store.path().join("ledger.jsonl")).unwrap();
    assert!(!ledger_text.contains(CANARY));
    let _ = root;
}

// ---------------------------------------------------------------------
// OpenCode
// ---------------------------------------------------------------------

fn opencode_fixture() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("opencode-data");
    let repo = root.path().join("repo");
    fs::create_dir_all(repo.join(".git")).unwrap();
    write(
        &home.join("storage/project/p1.json"),
        format!(
            "{{\"id\":\"p1\",\"vcs\":\"git\",\"worktree\":\"{}\"}}",
            repo.display()
        )
        .as_bytes(),
    );
    let session = home.join("storage/session/p1/s1.json");
    write(
        &session,
        format!("{{\"id\":\"s1\",\"title\":\"{CANARY}\"}}").as_bytes(),
    );
    write(
        &home.join("storage/message/s1/m1.json"),
        format!("{{\"content\":\"{CANARY}\"}}").as_bytes(),
    );
    write(&home.join("log").join("app.log"), b"debug line");
    write(&home.join("auth.json"), b"[redacted]");
    (root, home, repo, session)
}

#[test]
fn opencode_cache_removal_preserves_auth_and_project_metadata() {
    let (root, home, _repo, _session) = opencode_fixture();
    let mut env_vars = HashMap::new();
    env_vars.insert("OPENCODE_DATA_DIR".to_string(), home.display().to_string());
    let units = units_for(env_vars, "opencode");
    let store = tempfile::tempdir().unwrap();
    let log_dir = home.join("log");
    let plan = actions::propose_agents(&units, std::slice::from_ref(&log_dir), "test").unwrap();
    actions::save_plan(store.path(), &plan).unwrap();
    actions::approve(store.path(), &plan.id, "human:test").unwrap();
    let trash = tempfile::tempdir().unwrap();
    let result =
        actions::execute_with_trash(store.path(), &plan.id, "human:test", trash.path()).unwrap();
    assert_eq!(
        result.outcomes[0].status, "completed",
        "{:?}",
        result.outcomes[0]
    );
    assert!(!log_dir.exists());
    assert!(home.join("auth.json").exists());
    assert!(home.join("storage/project/p1.json").exists());
    let _ = root;
}

#[test]
fn opencode_session_removal_moves_the_message_companion_together() {
    let (root, home, repo, session) = opencode_fixture();
    let mut env_vars = HashMap::new();
    env_vars.insert("OPENCODE_DATA_DIR".to_string(), home.display().to_string());
    let units = units_for(env_vars, "opencode");
    let unit = units
        .iter()
        .find(|u| u.category == swamp_core::agents::AgentCategory::Sessions && u.path == session)
        .expect("session unit");
    assert_eq!(unit.action, AgentActionCapability::SessionRemoval);
    assert_eq!(unit.members.len(), 2, "transcript + message companion");
    assert!(matches!(
        unit.project_link,
        swamp_core::agents::ProjectLinkState::Linked { .. }
    ));

    let store = tempfile::tempdir().unwrap();
    let plan = actions::propose_agents(&units, std::slice::from_ref(&session), "test").unwrap();
    actions::save_plan(store.path(), &plan).unwrap();
    actions::approve(store.path(), &plan.id, "human:test").unwrap();
    let trash = tempfile::tempdir().unwrap();
    let result =
        actions::execute_with_trash(store.path(), &plan.id, "human:test", trash.path()).unwrap();
    assert_eq!(
        result.outcomes[0].status, "completed",
        "{:?}",
        result.outcomes[0]
    );
    assert!(!session.exists());
    assert!(!home.join("storage/message/s1").exists());
    assert!(home.join("storage/project/p1.json").exists());
    assert!(repo.join(".git").exists());

    let serialized = serde_json::to_string(&units).unwrap();
    assert!(!serialized.contains(CANARY));
    let ledger_text = fs::read_to_string(store.path().join("ledger.jsonl")).unwrap();
    assert!(!ledger_text.contains(CANARY));
    let _ = root;
}
