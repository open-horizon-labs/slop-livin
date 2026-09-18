use anyhow::Result;
use serde_json::{Value, json};
use slop_livin_core::report::{ArtifactKind, Report, UnownedReason, report_with};
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

/// `${SLOP_LIVIN_DIR}`, defaulting to `~/.local/share/slop-livin`. Same
/// resolution as the CLI so both surfaces observe into the same store.
fn slop_livin_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("SLOP_LIVIN_DIR") {
        return PathBuf::from(dir);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".local/share/slop-livin")
}

fn run_report(root: &str, since: Option<&str>) -> Result<Report> {
    report_with(
        &PathBuf::from(root),
        None,
        false,
        Some(&slop_livin_dir()),
        since,
    )
}

fn kind_label(kind: &ArtifactKind) -> &'static str {
    match kind {
        ArtifactKind::BuildOutput => "build",
        ArtifactKind::DependencyTree => "deps",
        ArtifactKind::Git => "git",
        ArtifactKind::Cache => "cache",
        ArtifactKind::Source => "source",
        ArtifactKind::DockerImage => "docker-image",
        ArtifactKind::DockerCache => "docker-cache",
        ArtifactKind::DockerVolume => "docker-volume",
        ArtifactKind::Loose => "loose",
        ArtifactKind::Unknown => "unknown",
    }
}

fn reason_label(reason: &UnownedReason) -> &'static str {
    match reason {
        UnownedReason::OutsideAnyCheckout => "outside-any-checkout",
        UnownedReason::OwnedByNothing => "owned-by-nothing",
        UnownedReason::InconclusiveEvidence => "inconclusive-evidence",
        UnownedReason::NoContainingRepo => "no-containing-repo",
        UnownedReason::SharedCache => "shared-cache",
        UnownedReason::PermissionDenied => "permission-denied",
    }
}

/// `report` tool: the full report structure (same shape as the CLI's
/// `--json`), optionally scoped to one project by name.
fn tool_report(params: &Value) -> Result<Value> {
    let args = params.get("arguments").cloned().unwrap_or_default();
    let root = args
        .get("root")
        .and_then(Value::as_str)
        .unwrap_or(".")
        .to_string();
    let since = args.get("since").and_then(Value::as_str).map(String::from);
    let project = args
        .get("project")
        .and_then(Value::as_str)
        .map(String::from);

    let r = run_report(&root, since.as_deref())?;
    let value = serde_json::to_value(&r)?;
    if let Some(name) = project {
        let projects = value
            .get("projects")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let filtered: Vec<Value> = projects
            .into_iter()
            .filter(|p| p.get("name").and_then(Value::as_str) == Some(name.as_str()))
            .collect();
        let mut scoped = value.clone();
        scoped["projects"] = json!(filtered);
        Ok(scoped)
    } else {
        Ok(value)
    }
}

/// `what_grew` tool: rows with growth > 0 across every project/worktree/
/// artifact, sorted desc, plus an unowned summary and a coverage block
/// (walked vs du vs unowned vs permission-denied, observed_at, whether
/// this call refreshed the growth index).
fn tool_what_grew(params: &Value) -> Result<Value> {
    let args = params.get("arguments").cloned().unwrap_or_default();
    let root = args
        .get("root")
        .and_then(Value::as_str)
        .unwrap_or(".")
        .to_string();
    let since = args.get("since").and_then(Value::as_str).map(String::from);

    let r = run_report(&root, since.as_deref())?;

    let mut grown: Vec<Value> = Vec::new();
    for project in &r.projects {
        for wt in &project.worktrees {
            for a in &wt.artifacts {
                if let Some(g) = a.growth_bytes
                    && g > 0
                {
                    grown.push(json!({
                        "project": project.name,
                        "worktree_id": wt.worktree_id,
                        "kind": kind_label(&a.kind),
                        "path": a.path,
                        "bytes": a.bytes,
                        "growth_bytes": g,
                        "regrowth_count": a.regrowth_count,
                    }));
                }
            }
        }
    }
    grown.sort_by(|a, b| {
        b["growth_bytes"]
            .as_i64()
            .unwrap_or(0)
            .cmp(&a["growth_bytes"].as_i64().unwrap_or(0))
    });

    let mut unowned_by_reason: std::collections::BTreeMap<&'static str, u64> =
        std::collections::BTreeMap::new();
    let mut permission_denied_count = 0u64;
    for row in &r.unowned {
        *unowned_by_reason
            .entry(reason_label(&row.reason))
            .or_insert(0) += row.bytes;
        if row.reason == UnownedReason::PermissionDenied {
            permission_denied_count += 1;
        }
    }

    Ok(json!({
        "grown": grown,
        "unowned_by_reason": unowned_by_reason,
        "coverage": {
            "walked_total": r.reconciliation.walked_total,
            "du_total": r.reconciliation.du_total,
            "unowned_total": r.reconciliation.unowned,
            "attributed_total": r.reconciliation.attributed,
            "permission_denied_count": permission_denied_count,
            "observed_at": r.observed_at,
            "index_refreshed": true,
        },
    }))
}

fn main() -> Result<()> {
    for line in io::stdin().lock().lines() {
        let line = line?;
        let req: Value = serde_json::from_str(&line)?;
        let id = req.get("id").cloned().unwrap_or(Value::Null);
        let method = req.get("method").and_then(Value::as_str).unwrap_or("");
        let result = match method {
            "initialize" => {
                json!({"protocolVersion":"2025-03-26","capabilities":{"tools":{}},"serverInfo":{"name":"slop-livin","version":env!("CARGO_PKG_VERSION")}})
            }
            "tools/list" => {
                json!({"tools":[
                    {
                        "name":"report",
                        "description":"Project x worktree x artifact growth report for a root: reclaimable bytes, growth, and git activity signals. Never a verdict.",
                        "inputSchema":{"type":"object","properties":{
                            "root":{"type":"string","description":"Root directory to report on (default: '.')"},
                            "since":{"type":"string","description":"Growth baseline window, e.g. '24h', '7d'"},
                            "project":{"type":"string","description":"Scope the report to one project by name"}
                        }}
                    },
                    {
                        "name":"what_grew",
                        "description":"Artifacts that grew since the window, sorted desc, plus unowned summary and coverage. One call instead of several ad hoc queries.",
                        "inputSchema":{"type":"object","required":["root","since"],"properties":{
                            "root":{"type":"string","description":"Root directory to report on"},
                            "since":{"type":"string","description":"Growth baseline window, e.g. '24h', '7d'"}
                        }}
                    }
                ]})
            }
            "tools/call" => {
                let params = req.get("params").cloned().unwrap_or_default();
                let name = params.get("name").and_then(Value::as_str).unwrap_or("");
                let answer = match name {
                    "report" => tool_report(&params)
                        .unwrap_or_else(|e| json!({"state":"error","cause":e.to_string()})),
                    "what_grew" => tool_what_grew(&params)
                        .unwrap_or_else(|e| json!({"state":"error","cause":e.to_string()})),
                    _ => json!({"state":"unsupported","cause":"unknown tool"}),
                };
                json!({"content":[{"type":"text","text":serde_json::to_string(&answer)?}]})
            }
            _ => json!({"state":"unsupported","cause":format!("unknown method {method}")}),
        };
        writeln!(
            io::stdout(),
            "{}",
            json!({"jsonrpc":"2.0","id":id,"result":result})
        )?;
        io::stdout().flush()?;
    }
    Ok(())
}
