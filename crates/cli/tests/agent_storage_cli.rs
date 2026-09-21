//! #100/#101 end-to-end: `swamp report --view agents`, `swamp protect`,
//! and the `propose-agents` -> `approve` -> `execute` action path,
//! through the real built binary. Every fixture Claude Code home here is
//! synthetic (PRIVACY IS A HARD RULE): no real `~/.claude` is read.

use std::path::PathBuf;
use std::process::Command;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_swamp"))
}

fn write(path: &std::path::Path, content: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

fn session_line(cwd: &std::path::Path) -> String {
    format!(
        "{{\"type\":\"user\",\"sessionId\":\"s\",\"cwd\":\"{}\",\"gitBranch\":\"main\"}}\n",
        cwd.display()
    )
}

/// Builds a fixture Claude Code home with one repo-linked session plus a
/// `shell-snapshots` cache directory. Returns `(home_root, claude_home, repo)`.
fn fixture(home_root: &std::path::Path) -> (PathBuf, PathBuf) {
    let claude_home = home_root.join("claude-home");
    let repo = home_root.join("repo");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    let session_id = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";
    write(
        &claude_home
            .join("projects")
            .join("-repo-encoded")
            .join(format!("{session_id}.jsonl")),
        session_line(&repo).as_bytes(),
    );
    write(
        &claude_home.join("shell-snapshots").join("snap.sh"),
        b"alias x=y",
    );
    write(&claude_home.join("settings.json"), b"{}");
    (claude_home, repo)
}

#[test]
fn report_view_agents_lists_a_claude_code_session_with_project_linkage() {
    let home = tempfile::tempdir().unwrap();
    let (claude_home, repo) = fixture(home.path());
    let store = tempfile::tempdir().unwrap();
    std::fs::write(
        store.path().join("config.toml"),
        "[scan]\ndefaults = false\ndisabled_detectors = [\"cargo-home\", \"rustup\", \"homebrew\"]\n",
    )
    .unwrap();

    let output = Command::new(bin())
        .arg("report")
        .arg("--view")
        .arg("agents")
        .arg("--json")
        .env("SWAMP_DIR", store.path())
        .env("HOME", home.path())
        .env("CLAUDE_CONFIG_DIR", &claude_home)
        .env("SWAMP_TEST_MODE", "1")
        .output()
        .expect("run report --view agents");
    assert!(
        output.status.success(),
        "report --view agents failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["view"], "agents");
    let units = json["result"]["units"].as_array().expect("units array");
    let session = units
        .iter()
        .find(|u| u["category"] == "sessions")
        .expect("session unit present");
    assert_eq!(session["project_link"]["state"], "linked");
    assert_eq!(
        session["project_link"]["project_path"],
        repo.display().to_string()
    );
    // Redaction: no prompt/message content field exists anywhere in the output.
    let text = serde_json::to_string(&json).unwrap();
    assert!(!text.contains("\"message\""));
}

#[test]
fn report_view_agents_project_filter_narrows_to_linked_units() {
    let home = tempfile::tempdir().unwrap();
    let (claude_home, _repo) = fixture(home.path());
    let store = tempfile::tempdir().unwrap();
    std::fs::write(
        store.path().join("config.toml"),
        "[scan]\ndefaults = false\ndisabled_detectors = [\"cargo-home\", \"rustup\", \"homebrew\"]\n",
    )
    .unwrap();

    let output = Command::new(bin())
        .arg("report")
        .arg("--view")
        .arg("agents")
        .arg("--project")
        .arg("no-such-project")
        .arg("--json")
        .env("SWAMP_DIR", store.path())
        .env("HOME", home.path())
        .env("CLAUDE_CONFIG_DIR", &claude_home)
        .env("SWAMP_TEST_MODE", "1")
        .output()
        .expect("run report --view agents --project");
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let units = json["result"]["units"].as_array().expect("units array");
    assert!(
        units.is_empty(),
        "a project filter matching nothing must narrow to zero units: {units:?}"
    );
}

#[test]
fn protect_add_list_remove_round_trip_through_the_binary() {
    let home = tempfile::tempdir().unwrap();
    let (claude_home, _repo) = fixture(home.path());
    let store = tempfile::tempdir().unwrap();
    let target = claude_home.join("shell-snapshots");

    let add = Command::new(bin())
        .arg("protect")
        .arg("add")
        .arg(&target)
        .env("SWAMP_DIR", store.path())
        .env("HOME", home.path())
        .output()
        .expect("run protect add");
    assert!(
        add.status.success(),
        "{}",
        String::from_utf8_lossy(&add.stderr)
    );

    let list = Command::new(bin())
        .arg("protect")
        .arg("list")
        .arg("--json")
        .env("SWAMP_DIR", store.path())
        .env("HOME", home.path())
        .output()
        .expect("run protect list");
    assert!(list.status.success());
    let json: serde_json::Value = serde_json::from_slice(&list.stdout).unwrap();
    let paths: Vec<String> = json
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(paths.contains(&target.display().to_string()), "{paths:?}");

    let remove = Command::new(bin())
        .arg("protect")
        .arg("remove")
        .arg(&target)
        .env("SWAMP_DIR", store.path())
        .env("HOME", home.path())
        .output()
        .expect("run protect remove");
    assert!(remove.status.success());

    let list2 = Command::new(bin())
        .arg("protect")
        .arg("list")
        .arg("--json")
        .env("SWAMP_DIR", store.path())
        .env("HOME", home.path())
        .output()
        .expect("run protect list again");
    let json2: serde_json::Value = serde_json::from_slice(&list2.stdout).unwrap();
    assert!(json2.as_array().unwrap().is_empty());
}

/// A `swamp protect`-ed cache directory refuses `propose-agents`, and an
/// unprotected one goes all the way through propose -> approve ->
/// execute, moving it to a fixture Trash directory (never the real
/// developer `~/.Trash`, via `SWAMP_TRASH_DIR`).
#[test]
fn propose_agents_approve_execute_moves_an_unprotected_cache_to_trash() {
    let home = tempfile::tempdir().unwrap();
    let (claude_home, _repo) = fixture(home.path());
    let store = tempfile::tempdir().unwrap();
    let trash = tempfile::tempdir().unwrap();
    let target = claude_home.join("shell-snapshots");

    let envs = |cmd: &mut Command| {
        cmd.env("SWAMP_DIR", store.path())
            .env("HOME", home.path())
            .env("CLAUDE_CONFIG_DIR", &claude_home)
            .env("SWAMP_TRASH_DIR", trash.path())
            .env("SWAMP_TEST_MODE", "1");
    };

    let mut propose_cmd = Command::new(bin());
    propose_cmd
        .arg("propose-agents")
        .arg("--path")
        .arg(&target)
        .arg("--json");
    envs(&mut propose_cmd);
    let propose = propose_cmd.output().expect("run propose-agents");
    assert!(
        propose.status.success(),
        "{}",
        String::from_utf8_lossy(&propose.stderr)
    );
    let propose_json: serde_json::Value = serde_json::from_slice(&propose.stdout).unwrap();
    let plan_id = propose_json["plan_id"]
        .as_str()
        .or_else(|| propose_json["id"].as_str())
        .expect("plan id present in propose-agents --json output")
        .to_string();

    let mut approve_cmd = Command::new(bin());
    approve_cmd.arg("approve").arg(&plan_id);
    envs(&mut approve_cmd);
    let approve = approve_cmd.output().expect("run approve");
    assert!(
        approve.status.success(),
        "{}",
        String::from_utf8_lossy(&approve.stderr)
    );

    let mut execute_cmd = Command::new(bin());
    execute_cmd.arg("execute").arg(&plan_id).arg("--json");
    envs(&mut execute_cmd);
    let execute = execute_cmd.output().expect("run execute");
    assert!(
        execute.status.success(),
        "{}",
        String::from_utf8_lossy(&execute.stderr)
    );
    let exec_json: serde_json::Value = serde_json::from_slice(&execute.stdout).unwrap();
    assert_eq!(
        exec_json["outcomes"][0]["status"], "completed",
        "{exec_json}"
    );
    assert!(
        !target.exists(),
        "cache dir should be gone from its original location"
    );
    assert!(
        std::fs::read_dir(trash.path()).unwrap().next().is_some(),
        "moved content must land in the fixture Trash dir, not the real one"
    );

    // The real developer Trash directory is never touched by this test.
}

#[test]
fn propose_agents_refuses_a_protected_path() {
    let home = tempfile::tempdir().unwrap();
    let (claude_home, _repo) = fixture(home.path());
    let store = tempfile::tempdir().unwrap();
    let settings = claude_home.join("settings.json");

    let output = Command::new(bin())
        .arg("propose-agents")
        .arg("--path")
        .arg(&settings)
        .env("SWAMP_DIR", store.path())
        .env("HOME", home.path())
        .env("CLAUDE_CONFIG_DIR", &claude_home)
        .env("SWAMP_TEST_MODE", "1")
        .output()
        .expect("run propose-agents on a protected path");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("protected") || stderr.contains("refused"),
        "{stderr}"
    );
    assert!(settings.exists());
}
