//! Drives the real `swamp` binary end to end against disposable git
//! fixtures and asserts the bounded JSON contract `report --json` (and
//! friends) promises agent callers: deterministic envelope fields
//! (`since`, `observed_at`, `index_refreshed`, `total`, `truncated`),
//! no-change and partial-coverage behavior, an invalid filter erroring
//! to stderr with nothing on stdout, pagination that never silently
//! drops rows without saying so, a plan awaiting authorization, and a
//! refused execution. This is the CLI-first replacement for the MCP
//! crate's `crates/mcp/tests/what_grew.rs` and `tool_list.rs`: same
//! guarantees, driven through the noninteractive CLI surface instead of
//! a JSON-RPC stdin/stdout protocol.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_swamp"))
}

fn run(store: &Path, args: &[&str]) -> Output {
    Command::new(bin())
        .args(args)
        .env("SWAMP_DIR", store)
        .env("SWAMP_TEST_MODE", "1")
        .output()
        .expect("run swamp")
}

fn run_json(store: &Path, args: &[&str]) -> serde_json::Value {
    let out = run(store, args);
    assert!(
        out.status.success(),
        "swamp {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "swamp {args:?} did not print JSON on stdout: {e}\nstdout: {}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

fn run_git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "commit.gpgsign")
        .env("GIT_CONFIG_VALUE_0", "false")
        .output()
        .unwrap_or_else(|e| panic!("run git {args:?}: {e}"));
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// One synthetic checkout: a git repo with a committed file and an
/// ignored `node_modules` "dependency tree" artifact of `seed_bytes`.
fn make_checkout(root: &Path, name: &str, seed_bytes: usize) -> PathBuf {
    let dir = root.join(name);
    fs::create_dir_all(&dir).unwrap();
    run_git(&dir, &["init", "-q", "-b", "main"]);
    fs::write(dir.join("README.md"), b"x").unwrap();
    fs::write(dir.join(".gitignore"), b"node_modules/\n").unwrap();
    run_git(&dir, &["add", "README.md", ".gitignore"]);
    run_git(&dir, &["commit", "-q", "-m", "init"]);
    let deps = dir.join("node_modules");
    fs::create_dir_all(&deps).unwrap();
    fs::write(deps.join("seed"), vec![b'x'; seed_bytes]).unwrap();
    dir
}

/// A fresh `report --json` call (no prior observation) is a coherent
/// no-growth baseline, not an error: the grown view is empty and the
/// coverage block says plainly that there is no history yet instead of
/// fabricating a window.
#[test]
fn grown_view_reports_partial_coverage_before_any_history_exists() {
    let root = tempfile::tempdir().unwrap();
    make_checkout(root.path(), "repo", 4096);
    let store = tempfile::tempdir().unwrap();

    let v = run_json(
        store.path(),
        &[
            "report",
            root.path().to_str().unwrap(),
            "--view",
            "grown",
            "--json",
            "--since",
            "1h",
        ],
    );
    assert_eq!(v["view"], "grown");
    assert_eq!(v["result"]["grown"].as_array().unwrap().len(), 0);
    let history = &v["coverage"]["history"];
    // The very first observation this store ever wrote has zero span
    // (nothing to diff against yet): the asked 1h window cannot be
    // honestly honored, and the note says exactly that instead of
    // silently reporting growth over a window that never existed.
    assert_eq!(history["history_secs"], serde_json::json!(0));
    assert_eq!(history["effective_window_secs"], serde_json::json!(0));
    assert!(
        history["note"]
            .as_str()
            .unwrap()
            .contains("the store holds 0s of observations"),
        "{v:#}"
    );
    for key in ["observed_at", "since", "index_refreshed"] {
        assert!(
            v["coverage"].get(key).is_some(),
            "coverage missing {key}: {v:#}"
        );
    }
}

/// After a real growth event, `--view grown --json` names the row and a
/// second call with nothing changed shows no growth: growth reporting
/// reflects an actual event, not the passage of time or re-observation.
#[test]
fn grown_view_reports_a_row_after_growth_then_nothing_on_a_no_change_rerun() {
    let root = tempfile::tempdir().unwrap();
    let checkout = make_checkout(root.path(), "repo", 4096);
    let store = tempfile::tempdir().unwrap();

    // Baseline observation.
    let first = run_json(
        store.path(),
        &[
            "report",
            root.path().to_str().unwrap(),
            "--view",
            "grown",
            "--json",
            "--since",
            "1h",
        ],
    );
    assert_eq!(first["result"]["grown"].as_array().unwrap().len(), 0);

    // A real growth event.
    fs::write(
        checkout.join("node_modules/growth-probe"),
        vec![b'g'; 2 * 1024 * 1024],
    )
    .unwrap();

    let grown = run_json(
        store.path(),
        &[
            "report",
            root.path().to_str().unwrap(),
            "--view",
            "grown",
            "--json",
            "--since",
            "1h",
        ],
    );
    let rows = grown["result"]["grown"].as_array().unwrap();
    let row = rows
        .iter()
        .find(|r| r["path"].as_str().unwrap().contains("node_modules"))
        .unwrap_or_else(|| panic!("no node_modules row in {grown:#}"));
    assert!(row["growth_bytes"].as_i64().unwrap() > 0);

    // No-change rerun: growth is an event, not a clock.
    let again = run_json(
        store.path(),
        &[
            "report",
            root.path().to_str().unwrap(),
            "--view",
            "grown",
            "--json",
            "--since",
            "1h",
        ],
    );
    assert_eq!(
        again["result"]["grown"].as_array().unwrap().len(),
        0,
        "no filesystem change since the last observation must show no growth: {again:#}"
    );
}

/// An invalid `--filter` expression fails loudly: a nonzero exit, the
/// parse error on stderr, and *nothing* on stdout -- never a partial or
/// malformed JSON document a caller might try to parse anyway.
#[test]
fn invalid_filter_errors_to_stderr_with_no_stdout_json() {
    let root = tempfile::tempdir().unwrap();
    make_checkout(root.path(), "repo", 4096);
    let store = tempfile::tempdir().unwrap();

    let out = run(
        store.path(),
        &[
            "report",
            root.path().to_str().unwrap(),
            "--json",
            "--filter",
            "this is not a valid filter expression (((",
        ],
    );
    assert!(!out.status.success());
    assert!(
        out.stdout.is_empty(),
        "stdout: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(!out.stderr.is_empty());
}

/// `--limit`/`--offset` bound an array-shaped `--view` result and say so:
/// `total` is the unbounded count, `truncated` is true whenever the page
/// is not the whole answer. A caller must never mistake a page for the
/// full inventory.
#[test]
fn view_worktrees_json_is_bounded_by_limit_and_offset() {
    let root = tempfile::tempdir().unwrap();
    make_checkout(root.path(), "repo-a", 1024);
    make_checkout(root.path(), "repo-b", 1024);
    let store = tempfile::tempdir().unwrap();

    let whole = run_json(
        store.path(),
        &[
            "report",
            root.path().to_str().unwrap(),
            "--view",
            "worktrees",
            "--json",
        ],
    );
    let total = whole["result"].as_array().unwrap().len();
    assert_eq!(total, 2, "{whole:#}");
    assert_eq!(whole["total"], serde_json::json!(2));
    assert_eq!(whole["truncated"], serde_json::json!(false));

    let page = run_json(
        store.path(),
        &[
            "report",
            root.path().to_str().unwrap(),
            "--view",
            "worktrees",
            "--json",
            "--limit",
            "1",
        ],
    );
    assert_eq!(page["result"].as_array().unwrap().len(), 1);
    assert_eq!(page["total"], serde_json::json!(2));
    assert_eq!(page["truncated"], serde_json::json!(true));

    let second_page = run_json(
        store.path(),
        &[
            "report",
            root.path().to_str().unwrap(),
            "--view",
            "worktrees",
            "--json",
            "--limit",
            "1",
            "--offset",
            "1",
        ],
    );
    assert_ne!(
        page["result"][0]["path"], second_page["result"][0]["path"],
        "offset must move the window, not repeat the first page"
    );
}

/// `propose --json` never authorizes anything: the plan comes back
/// `awaiting-authorization` with the exact `swamp approve` command a
/// human runs next, and nothing on disk is touched.
#[test]
fn propose_json_is_awaiting_authorization_and_names_the_approve_command() {
    let root = tempfile::tempdir().unwrap();
    let checkout = make_checkout(root.path(), "repo", 4096);
    let store = tempfile::tempdir().unwrap();

    let plan = run_json(
        store.path(),
        &[
            "propose",
            root.path().to_str().unwrap(),
            "--json",
            "--filter",
            "kind:DependencyTree",
        ],
    );
    assert_eq!(plan["state"], "awaiting-authorization");
    let plan_id = plan["id"].as_str().expect("plan id");
    assert!(
        plan["next_step"]
            .as_str()
            .unwrap()
            .contains(&format!("swamp approve {plan_id}")),
        "{plan:#}"
    );
    assert!(checkout.join("node_modules").exists());
}

/// `execute --json` on a plan nothing has approved refuses: no grant
/// covers it, the state says `awaiting-authorization`, and the artifact
/// is left exactly where it was.
#[test]
fn execute_json_without_a_grant_is_refused_and_deletes_nothing() {
    let root = tempfile::tempdir().unwrap();
    let checkout = make_checkout(root.path(), "repo", 4096);
    let store = tempfile::tempdir().unwrap();

    let plan = run_json(
        store.path(),
        &[
            "propose",
            root.path().to_str().unwrap(),
            "--json",
            "--filter",
            "kind:DependencyTree",
        ],
    );
    let plan_id = plan["id"].as_str().unwrap();

    let res = run_json(store.path(), &["execute", plan_id, "--json"]);
    assert_eq!(res["state"], "awaiting-authorization");
    assert_eq!(res["outcomes"].as_array().unwrap().len(), 0);
    assert!(
        checkout.join("node_modules").exists(),
        "nothing may be touched without a grant"
    );

    // plans --json wraps the array with a total, and grant list --json
    // is empty and self-describing: no grant was ever minted here.
    let plans = run_json(store.path(), &["plans", "--json"]);
    assert_eq!(plans["total"], serde_json::json!(1));
    let grants = run_json(store.path(), &["grant", "list", "--json"]);
    assert_eq!(grants["total"], serde_json::json!(0));
    assert!(grants["note"].as_str().unwrap().contains("human"));
}

/// Approving, then executing, actually removes the artifact -- the
/// positive-path counterpart to the refusal test above, so a reader of
/// this file sees both halves of the same contract.
#[test]
fn approve_then_execute_json_trashes_the_artifact() {
    let root = tempfile::tempdir().unwrap();
    let checkout = make_checkout(root.path(), "repo", 4096);
    let store = tempfile::tempdir().unwrap();

    let plan = run_json(
        store.path(),
        &[
            "propose",
            root.path().to_str().unwrap(),
            "--json",
            "--filter",
            "kind:DependencyTree",
        ],
    );
    let plan_id = plan["id"].as_str().unwrap().to_string();

    let approve_out = run(store.path(), &["approve", &plan_id]);
    assert!(
        approve_out.status.success(),
        "{}",
        String::from_utf8_lossy(&approve_out.stderr)
    );

    let res = run_json(store.path(), &["execute", &plan_id, "--json"]);
    assert_eq!(res["state"], "executed", "{res:#}");
    assert_eq!(res["outcomes"][0]["status"], "completed", "{res:#}");
    assert!(!checkout.join("node_modules").exists());
}

/// `--view projects`/`--view grown` are JSON-only: in text mode they
/// name themselves in a clear error on stderr instead of silently
/// falling back to a different view.
#[test]
fn json_only_views_refuse_text_mode_explicitly() {
    let root = tempfile::tempdir().unwrap();
    make_checkout(root.path(), "repo", 4096);
    let store = tempfile::tempdir().unwrap();

    for view in ["projects", "grown"] {
        let out = run(
            store.path(),
            &["report", root.path().to_str().unwrap(), "--view", view],
        );
        assert!(!out.status.success());
        assert!(out.stdout.is_empty());
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains("JSON only"), "{stderr}");
    }
}
