use anyhow::Result;
use serde_json::{Value, json};
use slop_livin_core::filter;
use slop_livin_core::github::{GithubFacts, MergeComplete, MergedStatus, PrStatus, TriState};
use slop_livin_core::growth::{DEFAULT_SINCE, load_config, parse_duration_secs};
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

/// The `since` window this call actually used: the caller's explicit
/// value if it parses, else the store's configured default, else the
/// hard-coded default -- the same resolution order `report_with` applies
/// internally. Every MCP result echoes this back (never just the raw,
/// possibly-absent argument) so a caller can see what window its numbers
/// actually reflect.
fn effective_since(since: Option<&str>) -> String {
    if let Some(s) = since
        && parse_duration_secs(s).is_some()
    {
        return s.to_string();
    }
    let config = load_config(&slop_livin_dir());
    if parse_duration_secs(&config.since).is_some() {
        return config.since;
    }
    DEFAULT_SINCE.to_string()
}

fn kind_label(kind: &ArtifactKind) -> &'static str {
    match kind {
        ArtifactKind::BuildOutput => "build",
        ArtifactKind::DependencyTree => "deps",
        ArtifactKind::Git => "git",
        ArtifactKind::Cache => "cache",
        ArtifactKind::Source => "source",
        ArtifactKind::DockerImage => "docker-image",
        ArtifactKind::DockerBuildCache => "docker-cache",
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
        UnownedReason::DockerNoJoin => "docker-no-join",
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
    let mut value = serde_json::to_value(&r)?;
    value["since"] = json!(effective_since(since.as_deref()));
    value["index_refreshed"] = json!(true);
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
        value["projects"] = json!(filtered);
    }
    Ok(value)
}

/// `list_projects` tool: name, id, total bytes, growth, checkout+worktree
/// count, and remote for every discovered project, ranked by growth then
/// bytes desc -- the same ordering `render_overview` uses -- so an agent
/// can get the ranked project list in one call instead of walking the
/// full `report` tree itself.
fn tool_list_projects(params: &Value) -> Result<Value> {
    let args = params.get("arguments").cloned().unwrap_or_default();
    let root = args
        .get("root")
        .and_then(Value::as_str)
        .unwrap_or(".")
        .to_string();
    let since = args.get("since").and_then(Value::as_str).map(String::from);

    let r = run_report(&root, since.as_deref())?;

    let mut rows: Vec<Value> = r
        .projects
        .iter()
        .map(|p| {
            let checkout_count = p
                .worktrees
                .iter()
                .filter(|w| {
                    matches!(
                        w.kind,
                        slop_livin_core::report::WorktreeKind::Main
                            | slop_livin_core::report::WorktreeKind::Clone
                    )
                })
                .count();
            let worktree_count = p.worktrees.len();
            let mut bytes = 0u64;
            let mut growth: Option<i64> = None;
            for wt in &p.worktrees {
                for a in &wt.artifacts {
                    bytes += a.bytes;
                    if let Some(g) = a.growth_bytes {
                        growth = Some(growth.unwrap_or(0) + g);
                    }
                }
            }
            json!({
                "name": p.name,
                "project_id": p.project_id,
                "bytes": bytes,
                "growth_bytes": growth,
                "checkout_count": checkout_count,
                "worktree_count": worktree_count,
                "remote": p.remote,
            })
        })
        .collect();
    rows.sort_by(|a, b| {
        let ga = a["growth_bytes"].as_i64().unwrap_or(i64::MIN);
        let gb = b["growth_bytes"].as_i64().unwrap_or(i64::MIN);
        gb.cmp(&ga).then_with(|| {
            b["bytes"]
                .as_u64()
                .unwrap_or(0)
                .cmp(&a["bytes"].as_u64().unwrap_or(0))
        })
    });

    Ok(json!({
        "projects": rows,
        "observed_at": r.observed_at,
        "since": effective_since(since.as_deref()),
        "index_refreshed": true,
    }))
}

/// `what_grew` tool: rows with growth > 0 across every project/worktree/
/// artifact, sorted desc, plus an unowned summary and a coverage block
/// (walked vs du vs unowned vs permission-denied, observed_at, whether
/// this call refreshed the growth index).
fn tool_what_grew(params: &Value) -> Result<Value> {
    let args = params.get("arguments").cloned().unwrap_or_default();
    // `root` and `since` are declared `required` in this tool's schema
    // (unlike `report`, which defaults both), but a schema is advisory
    // to the *client*: nothing on this side previously enforced it, so a
    // caller that omitted `root` silently got a report scanned against
    // the MCP server's own working directory instead of an error --
    // which reads as "growth exists but `what_grew` returned an empty
    // `grown: []`" (issue #32 item 4) rather than the missing-argument
    // mistake it actually is. Refusing here turns that into a visible,
    // actionable error instead of a silent wrong-root scan.
    let root = match args.get("root").and_then(Value::as_str) {
        Some(root) if !root.is_empty() => root.to_string(),
        _ => {
            return Ok(json!({"state":"error","cause":"missing required argument \"root\""}));
        }
    };
    let since = match args.get("since").and_then(Value::as_str) {
        Some(since) if !since.is_empty() => since.to_string(),
        _ => {
            return Ok(json!({"state":"error","cause":"missing required argument \"since\""}));
        }
    };
    let since = Some(since);

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
            "since": effective_since(since.as_deref()),
            "index_refreshed": true,
        },
    }))
}

fn render_pr_json(pr: &PrStatus) -> Value {
    match pr {
        PrStatus::None => json!("none"),
        PrStatus::Unknown => json!("unknown"),
        PrStatus::Some(pr) => json!({
            "number": pr.number,
            "state": format!("{:?}", pr.state).to_lowercase(),
            "draft": pr.draft,
            "url": pr.url,
            "title": pr.title,
            "review_decision": format!("{:?}", pr.review_decision).to_lowercase(),
            "updated_at": pr.updated_at,
        }),
    }
}

/// `list_worktrees` tool: one entry per worktree (optionally scoped by
/// `filter`, same grammar as the CLI's `--filter`), with branch, idle,
/// the `merge_complete` composite fact and its terms, and PR status.
/// Never a verdict, never an action -- the `remove_command` field is
/// text for a human to run, not something this tool executes.
fn tool_list_worktrees(params: &Value) -> Result<Value> {
    let args = params.get("arguments").cloned().unwrap_or_default();
    let root = args
        .get("root")
        .and_then(Value::as_str)
        .unwrap_or(".")
        .to_string();
    let since = args.get("since").and_then(Value::as_str).map(String::from);
    let filter_expr = args.get("filter").and_then(Value::as_str);
    let parsed_filter = match filter_expr {
        Some(expr) => filter::parse(expr)?,
        None => filter::Filter::default(),
    };

    let r = run_report(&root, since.as_deref())?;
    let mut rows: Vec<Value> = Vec::new();
    for project in &r.projects {
        for wt in &project.worktrees {
            let empty_pr = PrStatus::Unknown;
            let (pr, verdict, merge_complete_terms, merged) = match (&wt.github, &wt.merge_complete)
            {
                (
                    Some(GithubFacts {
                        pull_request,
                        merged,
                        ..
                    }),
                    Some(MergeComplete { verdict, terms }),
                ) => (pull_request, *verdict, Some(terms.clone()), merged),
                (
                    Some(GithubFacts {
                        pull_request,
                        merged,
                        ..
                    }),
                    None,
                ) => (pull_request, TriState::Unknown, None, merged),
                (None, _) => (&empty_pr, TriState::Unknown, None, &MergedStatus::Unknown),
            };
            let facts = filter::WorktreeFacts {
                merge_complete: verdict == TriState::Yes,
                idle_secs: wt.idle_secs,
                pr,
                merged,
            };
            if !parsed_filter.matches_worktree(project, wt, &facts) {
                continue;
            }
            let verdict_str = match verdict {
                TriState::Yes => "yes",
                TriState::No => "no",
                TriState::Unknown => "unknown",
            };
            rows.push(json!({
                "project": project.name,
                "path": wt.path,
                "branch": wt.branch,
                "idle_secs": wt.idle_secs,
                "merge_complete": merge_complete_terms.map(|terms| json!({
                    "verdict": verdict_str,
                    "terms": terms,
                })),
                "pull_request": render_pr_json(pr),
                "remove_command": format!("git worktree remove {}", wt.path.display()),
            }));
        }
    }
    Ok(json!({ "worktrees": rows }))
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
                    },
                    {
                        "name":"list_projects",
                        "description":"Ranked list of every discovered project: name, id, total bytes, growth, checkout+worktree count, and remote. One call instead of walking the full report tree.",
                        "inputSchema":{"type":"object","properties":{
                            "root":{"type":"string","description":"Root directory to report on (default: '.')"},
                            "since":{"type":"string","description":"Growth baseline window, e.g. '24h', '7d'"}
                        }}
                    },
                    {
                        "name":"list_worktrees",
                        "description":"Worktree merge-complete semantics: branch, idle time, the merge_complete composite fact with its terms, and GitHub PR status. Never a verdict -- always reported with the terms that fed it. remove_command is text for a human to run, never executed by this tool.",
                        "inputSchema":{"type":"object","properties":{
                            "root":{"type":"string","description":"Root directory to report on (default: '.')"},
                            "since":{"type":"string","description":"Growth baseline window, e.g. '24h', '7d'"},
                            "filter":{"type":"string","description":"Filter expression, e.g. 'merge-complete idle > 48h pr:merged'"}
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
                    "list_projects" => tool_list_projects(&params)
                        .unwrap_or_else(|e| json!({"state":"error","cause":e.to_string()})),
                    "list_worktrees" => tool_list_worktrees(&params)
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
