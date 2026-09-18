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
/// `--json`), optionally scoped to one project by name, and optionally
/// narrowed to one named `view` (#33) -- the same view set the CLI's
/// `--view` exposes: worktrees (default, the full structure), builds,
/// deps, docker, kinds, unowned, reconciliation.
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
    let view = args.get("view").and_then(Value::as_str).map(String::from);

    let r = run_report(&root, since.as_deref())?;

    if let Some(view) = view.as_deref() {
        let payload = view_payload(&r, view, project.as_deref());
        return Ok(json!({
            "view": view,
            "project": project,
            "result": payload,
            "observed_at": r.observed_at,
            "since": effective_since(since.as_deref()),
            "index_refreshed": true,
        }));
    }

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

/// Builds the JSON payload for one named `view` over a report, scoped to
/// `only_project` when set. Mirrors `render.rs`'s `render_view_*`
/// functions' selection logic so the CLI and MCP agree on what each view
/// means, just serialized instead of formatted as text.
fn view_payload(r: &Report, view: &str, only_project: Option<&str>) -> Value {
    match view {
        "kinds" => {
            let mut agg: std::collections::BTreeMap<&'static str, (u64, u64)> =
                std::collections::BTreeMap::new();
            for p in &r.projects {
                if only_project.is_some_and(|name| name != p.name) {
                    continue;
                }
                for wt in &p.worktrees {
                    for a in &wt.artifacts {
                        let entry = agg.entry(kind_label(&a.kind)).or_insert((0, 0));
                        entry.0 += a.bytes;
                        entry.1 += 1;
                    }
                }
            }
            json!(
                agg.into_iter()
                    .map(|(kind, (bytes, count))| json!({"kind": kind, "bytes": bytes, "count": count}))
                    .collect::<Vec<_>>()
            )
        }
        "builds" | "deps" => {
            let kinds: &[ArtifactKind] = if view == "builds" {
                &[ArtifactKind::BuildOutput, ArtifactKind::Cache]
            } else {
                &[ArtifactKind::DependencyTree]
            };
            let mut rows: Vec<Value> = Vec::new();
            for p in &r.projects {
                if only_project.is_some_and(|name| name != p.name) {
                    continue;
                }
                for wt in &p.worktrees {
                    for a in &wt.artifacts {
                        if !kinds.contains(&a.kind) {
                            continue;
                        }
                        rows.push(json!({
                            "project": p.name,
                            "kind": kind_label(&a.kind),
                            "path": a.path,
                            "bytes": a.bytes,
                            "growth_bytes": a.growth_bytes,
                        }));
                    }
                }
            }
            rows.sort_by(|a, b| {
                b["bytes"]
                    .as_u64()
                    .unwrap_or(0)
                    .cmp(&a["bytes"].as_u64().unwrap_or(0))
            });
            json!(rows)
        }
        "docker" => docker_objects_payload(r, false, only_project),
        "unowned" => {
            let rows: Vec<Value> = r
                .unowned
                .iter()
                .map(|row| {
                    json!({
                        "path_or_object": row.path_or_object,
                        "bytes": row.bytes,
                        "reason": reason_label(&row.reason),
                        "shared_bytes": row.shared_bytes,
                        "docker_kind": row.docker_kind,
                        "note": row.note,
                    })
                })
                .collect();
            json!(rows)
        }
        "reconciliation" => json!(r.reconciliation),
        _ => json!({
            "projects": r.projects.iter().filter(|p| only_project.is_none_or(|name| name == p.name)).collect::<Vec<_>>(),
        }),
    }
}

/// Shared by `report`'s `view: "docker"` and the standalone
/// `docker_objects` tool: every Docker object joined to a project (with
/// its project name attached) plus every unowned one, sorted by bytes
/// desc. `unowned_only` restricts to the unowned half; `only_project`
/// restricts joined rows to that project and unowned rows to
/// name-alike candidates, mirroring `render_view_docker`.
fn docker_objects_payload(r: &Report, unowned_only: bool, only_project: Option<&str>) -> Value {
    let mut rows: Vec<Value> = Vec::new();
    if !unowned_only {
        for p in &r.projects {
            if only_project.is_some_and(|name| name != p.name) {
                continue;
            }
            for wt in &p.worktrees {
                for a in &wt.artifacts {
                    if !matches!(
                        a.kind,
                        ArtifactKind::DockerImage
                            | ArtifactKind::DockerBuildCache
                            | ArtifactKind::DockerVolume
                    ) {
                        continue;
                    }
                    rows.push(json!({
                        "project": p.name,
                        "object": a.path,
                        "kind": kind_label(&a.kind),
                        "bytes": a.bytes,
                        "shared_bytes": null,
                        "created_at": a.created_at,
                        "shared_with": a.shared_with,
                        "containers": a.containers,
                        "dangling": a.dangling,
                        "note": a.note,
                        "unowned": false,
                    }));
                }
            }
        }
    }
    for row in &r.unowned {
        if row.reason != UnownedReason::DockerNoJoin {
            continue;
        }
        let project_field = match only_project {
            None => Value::Null,
            Some(name) => {
                if row
                    .path_or_object
                    .to_lowercase()
                    .contains(&name.to_lowercase())
                {
                    json!(format!("{name} (unowned, name-alike)"))
                } else {
                    continue;
                }
            }
        };
        rows.push(json!({
            "project": project_field,
            "object": row.path_or_object,
            "kind": row.docker_kind,
            "bytes": row.bytes,
            "shared_bytes": row.shared_bytes,
            "created_at": row.created_at,
            "shared_with": row.shared_with,
            "containers": row.containers,
            "dangling": row.dangling,
            "note": row.note,
            "unowned": true,
        }));
    }
    rows.sort_by(|a, b| {
        b["bytes"]
            .as_u64()
            .unwrap_or(0)
            .cmp(&a["bytes"].as_u64().unwrap_or(0))
    });
    json!(rows)
}

/// `docker_objects` tool: every Docker object with full detail
/// (created_at, compose service via its own labels, layer-derived
/// `shared_with`, referencing containers, dangling flag), sorted by
/// unique bytes desc. `unowned_only` restricts to objects with no join
/// evidence; `project` restricts joined rows to one project and unowned
/// rows to name-alike candidates, explicitly labelled and never
/// attributed.
fn tool_docker_objects(params: &Value) -> Result<Value> {
    let args = params.get("arguments").cloned().unwrap_or_default();
    let root = args
        .get("root")
        .and_then(Value::as_str)
        .unwrap_or(".")
        .to_string();
    let unowned_only = args
        .get("unowned_only")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let project = args
        .get("project")
        .and_then(Value::as_str)
        .map(String::from);

    let r = run_report(&root, None)?;
    let objects = docker_objects_payload(&r, unowned_only, project.as_deref());

    Ok(json!({
        "objects": objects,
        "observed_at": r.observed_at,
        "since": effective_since(None),
        "index_refreshed": true,
    }))
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
                            "project":{"type":"string","description":"Scope the report to one project by name"},
                            "view":{"type":"string","description":"Named view instead of the full structure: worktrees, builds, deps, docker, kinds, unowned, reconciliation"}
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
                    },
                    {
                        "name":"docker_objects",
                        "description":"Every Docker object (image/build-cache/volume) with full detail -- created_at, layer-derived shared_with, referencing containers, dangling -- sorted by unique bytes desc. Unowned objects are labelled, never attributed by name similarity.",
                        "inputSchema":{"type":"object","properties":{
                            "root":{"type":"string","description":"Root directory to report on (default: '.')"},
                            "unowned_only":{"type":"boolean","description":"Restrict to objects with no join evidence"},
                            "project":{"type":"string","description":"Restrict joined rows to one project; unowned rows to name-alike candidates, labelled unowned"}
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
                    "docker_objects" => tool_docker_objects(&params)
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

#[cfg(test)]
mod tests {
    use super::*;
    use slop_livin_core::entities::Confidence;
    use slop_livin_core::report::{
        ArtifactRow, ProjectRow, Reconciliation, Signal, Source, WorktreeKind, WorktreeRow,
    };
    use std::path::PathBuf;

    fn docker_image_row(reference: &str, bytes: u64) -> ArtifactRow {
        ArtifactRow {
            kind: ArtifactKind::DockerImage,
            path: PathBuf::from(reference),
            bytes,
            growth_bytes: None,
            regrowth_count: 0,
            observed_at: 1,
            confidence: Confidence::High,
            source: Source::new("docker.system_df"),
            note: None,
            created_at: Some("2026-09-10T12:00:00Z".to_string()),
            containers: vec![
                "hiphi-staging-relay-1 (exited, finished 2026-09-10T13:00:00Z)".to_string(),
            ],
            shared_with: vec!["hiphi-authorizer:staging".to_string()],
            dangling: false,
        }
    }

    fn fixture_report() -> Report {
        Report {
            observed_at: 1_000_000,
            root: PathBuf::from("/src"),
            projects: vec![ProjectRow {
                project_id: "p1".to_string(),
                name: "hiphi-relay".to_string(),
                remote: None,
                worktrees: vec![WorktreeRow {
                    worktree_id: "wt1".to_string(),
                    path: PathBuf::from("/src/hiphi-relay"),
                    kind: WorktreeKind::Main,
                    artifacts: vec![docker_image_row("hiphi-relay:staging", 500_000_000)],
                    signals: vec![Signal {
                        name: "dirty".to_string(),
                        value: "dirty".to_string(),
                    }],
                    branch: None,
                    github: None,
                    merge_complete: None,
                    idle_secs: None,
                }],
            }],
            unowned: vec![],
            reconciliation: Reconciliation {
                attributed: 500_000_000,
                unowned: 0,
                walked_total: 500_000_000,
                du_total: None,
                docker_attributed: 500_000_000,
                docker_unowned: 0,
            },
            notes: vec![],
            dirs_by_worktree: None,
            files_by_worktree: None,
            schedule_line: None,
            github_enrichment: None,
        }
    }

    #[test]
    fn docker_objects_payload_carries_container_and_shared_with_detail() {
        let report = fixture_report();
        let payload = docker_objects_payload(&report, false, None);
        let rows = payload.as_array().expect("array");
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row["object"], json!("hiphi-relay:staging"));
        assert_eq!(row["created_at"], json!("2026-09-10T12:00:00Z"));
        assert!(
            row["containers"][0]
                .as_str()
                .unwrap()
                .contains("hiphi-staging-relay-1")
        );
        assert_eq!(row["shared_with"][0], json!("hiphi-authorizer:staging"));
    }

    #[test]
    fn view_payload_reconciliation_matches_report_struct() {
        let report = fixture_report();
        let payload = view_payload(&report, "reconciliation", None);
        assert_eq!(payload["attributed"], json!(500_000_000));
        assert_eq!(payload["docker_attributed"], json!(500_000_000));
    }

    #[test]
    fn view_payload_worktrees_scopes_to_project() {
        let mut report = fixture_report();
        report.projects.push(ProjectRow {
            project_id: "p2".to_string(),
            name: "other".to_string(),
            remote: None,
            worktrees: vec![],
        });
        let payload = view_payload(&report, "worktrees", Some("hiphi-relay"));
        let projects = payload["projects"].as_array().expect("array");
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0]["name"], json!("hiphi-relay"));
    }
}
