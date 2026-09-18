//! Spawns the real `slop-livin-mcp` binary and asserts its `tools/list`
//! response is exactly `docker_objects` (added by #33), `list_projects`,
//! `list_worktrees` (added by #35), `report`, and `what_grew` --
//! everything else R6 removes (`pressure`, `truth`, `candidates`,
//! `measure`, and any grant/ledger/execution tool) must not be reachable
//! from this surface.

use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

#[test]
fn tools_list_is_exactly_report_what_grew_list_projects_list_worktrees_and_docker_objects() {
    let exe = env!("CARGO_BIN_EXE_slop-livin-mcp");
    let mut child = Command::new(exe)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn slop-livin-mcp");

    let mut stdin = child.stdin.take().unwrap();
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"tools/list"}}"#).unwrap();
    drop(stdin);

    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line).expect("read response line");
    let _ = child.kill();
    let _ = child.wait();

    let resp: Value = serde_json::from_str(&line).expect("valid JSON response");
    let tools = resp["result"]["tools"]
        .as_array()
        .expect("tools array present");
    let mut names: Vec<&str> = tools
        .iter()
        .map(|t| t["name"].as_str().expect("tool name"))
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        vec![
            "docker_objects",
            "list_projects",
            "list_worktrees",
            "report",
            "what_grew"
        ]
    );
}
