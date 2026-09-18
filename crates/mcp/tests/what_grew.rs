//! Drives the real `slop-livin-mcp` binary end to end over a fixture with
//! a known growth event: observe, grow an artifact, observe again, then
//! call `what_grew` and assert the grown row is present. This reproduces
//! issue #32 item 4 ("MCP `what_grew` returned `grown: []` while the CLI
//! showed +175 MB for the same store and window") against a minimal
//! fixture instead of a live multi-GB tree.

use serde_json::{Value, json};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};

fn run_git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .unwrap_or_else(|e| panic!("run git {args:?}: {e}"));
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

struct McpProcess {
    child: Child,
    stdin: std::process::ChildStdin,
    reader: BufReader<std::process::ChildStdout>,
}

impl McpProcess {
    fn spawn(store_dir: &Path) -> Self {
        let exe = env!("CARGO_BIN_EXE_slop-livin-mcp");
        let mut child = Command::new(exe)
            .env("SLOP_LIVIN_DIR", store_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn slop-livin-mcp");
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        Self {
            child,
            stdin,
            reader: BufReader::new(stdout),
        }
    }

    fn call(&mut self, id: i64, method: &str, params: Value) -> Value {
        let req = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        writeln!(self.stdin, "{}", serde_json::to_string(&req).unwrap()).unwrap();
        self.stdin.flush().unwrap();
        let mut line = String::new();
        self.reader.read_line(&mut line).expect("read response");
        let resp: Value = serde_json::from_str(&line).expect("valid JSON response");
        resp["result"].clone()
    }

    fn tool_call(&mut self, id: i64, name: &str, arguments: Value) -> Value {
        let result = self.call(
            id,
            "tools/call",
            json!({"name": name, "arguments": arguments}),
        );
        let text = result["content"][0]["text"]
            .as_str()
            .expect("tool result has text content");
        serde_json::from_str(text).expect("tool result is valid JSON")
    }
}

impl Drop for McpProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn what_grew_reports_a_row_after_a_real_growth_event() {
    let tmp = tempfile::tempdir().expect("tmp root");
    let root = tmp.path().join("checkout");
    fs::create_dir_all(&root).expect("mkdir checkout");
    run_git(&root, &["init", "-q", "-b", "main"]);
    run_git(&root, &["config", "commit.gpgsign", "false"]);
    fs::write(root.join("README.md"), b"x").expect("write README");
    run_git(&root, &["add", "README.md"]);
    run_git(&root, &["commit", "-q", "-m", "init"]);

    let artifact_dir = root.join("node_modules");
    fs::create_dir_all(&artifact_dir).expect("mkdir node_modules");
    fs::write(artifact_dir.join("seed"), vec![b'x'; 4096]).expect("write seed");

    let store = tempfile::tempdir().expect("tmp store");

    // First observation: establishes the baseline, no growth expected.
    {
        let mut mcp = McpProcess::spawn(store.path());
        let first = mcp.tool_call(
            1,
            "report",
            json!({"root": root.to_string_lossy(), "since": "1h"}),
        );
        assert!(
            first.get("state").is_none(),
            "first report call errored: {first:?}"
        );
    }

    // Grow the artifact by a known amount.
    let grow_bytes = 2 * 1024 * 1024;
    fs::write(artifact_dir.join("growth-probe"), vec![b'g'; grow_bytes])
        .expect("write growth probe");

    // Second observation, via `what_grew` directly: must see the grown row.
    let mut mcp = McpProcess::spawn(store.path());
    let grown = mcp.tool_call(
        2,
        "what_grew",
        json!({"root": root.to_string_lossy(), "since": "1h"}),
    );
    assert!(
        grown.get("state").is_none(),
        "what_grew call errored: {grown:?}"
    );

    let rows = grown["grown"].as_array().expect("grown is an array");
    assert!(
        !rows.is_empty(),
        "expected at least one grown row, got empty grown: {grown:?}"
    );
    let node_modules_row = rows
        .iter()
        .find(|r| {
            r["path"]
                .as_str()
                .map(|p| p.contains("node_modules"))
                .unwrap_or(false)
        })
        .unwrap_or_else(|| panic!("no node_modules row in grown: {grown:?}"));
    let growth = node_modules_row["growth_bytes"]
        .as_i64()
        .expect("growth_bytes is present");
    assert!(
        growth > 0,
        "expected positive growth for node_modules, got {growth}"
    );

    for key in ["observed_at", "since", "index_refreshed"] {
        assert!(
            grown["coverage"].get(key).is_some(),
            "coverage missing {key}: {grown:?}"
        );
    }
}

/// A `what_grew` call missing its (schema-)required `root` must return a
/// visible error, never a silent scan of the server's own working
/// directory -- that silent-wrong-root shape is indistinguishable from
/// the reported "grown: []" defect (issue #32 item 4) from the caller's
/// side: no error, just an empty/irrelevant result.
#[test]
fn what_grew_without_root_errors_instead_of_scanning_silently() {
    let store = tempfile::tempdir().expect("tmp store");
    let mut mcp = McpProcess::spawn(store.path());
    let result = mcp.tool_call(1, "what_grew", json!({"since": "1h"}));
    assert_eq!(
        result["state"], "error",
        "expected an error for missing root, got: {result:?}"
    );
}
