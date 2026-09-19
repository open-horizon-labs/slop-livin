//! One named audit per guardrail in `.oh/guardrails/`. Each reads the AST
//! (`ast.rs`) and returns `Err(why)` naming the file and the shape that
//! broke the constraint. `adr_validation` closes the loop: every guardrail
//! `audit:` and every ADR `validate:` entry must resolve to an audit here
//! or a test that exists.

use crate::ast::{self, SourceFile};
use std::path::Path;

pub type Audit = fn(&Path) -> Result<(), String>;

pub const AUDITS: &[(&str, Audit)] = &[
    ("one_byte_formatter", one_byte_formatter),
    ("legacy_invariants", legacy_invariants),
    ("fsevents_before_full_walk", fsevents_before_full_walk),
    ("column_store_parquet_zstd", column_store_parquet_zstd),
    (
        "reverse_delta_current_plus_deltas",
        reverse_delta_current_plus_deltas,
    ),
    (
        "scheduled_refresh_launchagent",
        scheduled_refresh_launchagent,
    ),
    ("folding_only_for_artifacts", folding_only_for_artifacts),
    ("symlinks_never_followed", symlinks_never_followed),
    (
        "incremental_walk_only_changed_subtrees",
        incremental_walk_only_changed_subtrees,
    ),
    ("walk_optimized_parallel_pool", walk_optimized_parallel_pool),
    ("dir_mtime_int32_minutes", dir_mtime_int32_minutes),
    (
        "agent_interface_facts_not_verdicts",
        agent_interface_facts_not_verdicts,
    ),
    ("human_only_authorization", human_only_authorization),
    (
        "no_consumer_knows_other_consumers",
        no_consumer_knows_other_consumers,
    ),
    ("static_registration_only", static_registration_only),
    ("all_report_paths_through_bus", all_report_paths_through_bus),
    ("extractors_are_pluggable", extractors_are_pluggable),
    (
        "event_bus_pluggable_consumers",
        event_bus_pluggable_consumers,
    ),
    ("adr_validation", adr_validation),
];

/// The guardrail-level umbrella: the three bus audits together.
fn event_bus_pluggable_consumers(root: &Path) -> Result<(), String> {
    no_consumer_knows_other_consumers(root)?;
    static_registration_only(root)?;
    all_report_paths_through_bus(root)
}

fn core(root: &Path, name: &str) -> Result<SourceFile, String> {
    ast::parse(root, &format!("crates/core/src/{name}"))
}

fn one_byte_formatter(root: &Path) -> Result<(), String> {
    let tui = ast::parse(root, "crates/tui/src/model.rs")?;
    let defines = ast::functions(&tui.ast)
        .iter()
        .any(|f| f.name == "human_bytes" || f.name == "human_signed_bytes");
    if defines {
        return Err("tui/src/model.rs defines a byte formatter; it must re-export core's".into());
    }
    if !tui
        .text
        .contains("pub use slop_livin_core::render::human_bytes_pub as human_bytes")
    {
        return Err("tui/src/model.rs must re-export core's byte formatter".into());
    }
    let render = core(root, "render.rs")?;
    let f = ast::functions(&render.ast);
    let hb = ast::function(&f, "human_bytes")?;
    if !hb.body.contains("1000") || hb.body.contains("1024") {
        return Err("render::human_bytes must divide by 1000 under SI labels".into());
    }
    Ok(())
}

fn legacy_invariants(root: &Path) -> Result<(), String> {
    let scan = core(root, "scan.rs")?;
    if !ast::referenced_idents(&scan.ast)
        .iter()
        .any(|i| i == "canonical_roots")
    {
        return Err(
            "scan.rs: roots must be canonicalized at the boundary (canonical_roots)".into(),
        );
    }
    let grants = core(root, "grants.rs")?;
    if !grants.text.contains("created_outside_index") {
        return Err("grants.rs: grants need non-index provenance".into());
    }
    Ok(())
}

fn fsevents_before_full_walk(root: &Path) -> Result<(), String> {
    let g = core(root, "growth.rs")?;
    let funcs = ast::functions(&g.ast);
    let f = ast::function(&funcs, "observe_tracked_with_source")?;
    let replay_at = f
        .stmts
        .iter()
        .position(|s| s.contains(". replay (") || s.contains(".replay("))
        .ok_or("observe_tracked_with_source never calls FSEvents replay")?;
    for (i, s) in f.stmts.iter().enumerate() {
        if i < replay_at && s.contains("full_walk") && !s.contains("force_full") {
            return Err(format!(
                "observe_tracked_with_source: statement {i} runs full_walk before the FSEvents replay without a force_full guard"
            ));
        }
    }
    Ok(())
}

fn column_store_parquet_zstd(root: &Path) -> Result<(), String> {
    for name in ["growth.rs", "store.rs"] {
        let f = core(root, name)?;
        for func in ast::functions(&f.ast) {
            if func.body.contains("ArrowWriter")
                && !(func.body.contains("ZSTD") || func.body.contains("zstd_properties"))
            {
                return Err(format!(
                    "{name}::{}: writes Parquet without declaring zstd compression",
                    func.name
                ));
            }
        }
    }
    Ok(())
}

fn reverse_delta_current_plus_deltas(root: &Path) -> Result<(), String> {
    let g = core(root, "growth.rs")?;
    let funcs = ast::functions(&g.ast);
    for (name, current, delta) in [
        ("observe_and_annotate", "current_path", "next_delta_path"),
        (
            "observe_and_annotate_dirs",
            "dirs_current_path",
            "next_seq_path",
        ),
        (
            "observe_and_annotate_files",
            "files_current_path",
            "next_seq_path",
        ),
    ] {
        let f = ast::function(&funcs, name)?;
        if !f.body.contains(current) || !f.body.contains(delta) {
            return Err(format!(
                "growth::{name} must rewrite the current file ({current}) and append a delta ({delta})"
            ));
        }
    }
    Ok(())
}

fn scheduled_refresh_launchagent(root: &Path) -> Result<(), String> {
    let cli = ast::parse(root, "crates/cli/src/main.rs")?;
    if !ast::enum_has_variant(&cli.ast, "Command", "Schedule") {
        return Err("cli: Command::Schedule subcommand missing".into());
    }
    let sched = core(root, "schedule.rs")?;
    if !ast::string_literals(&sched.ast)
        .iter()
        .any(|s| s == "launchctl")
    {
        return Err("core::schedule never invokes launchctl".into());
    }
    Ok(())
}

fn folding_only_for_artifacts(root: &Path) -> Result<(), String> {
    // 1. The parallel walk folds (builds a Size job) only under a
    //    classify_at guard, or while already inside a folded unit.
    let w = core(root, "walk.rs")?;
    let sites = ast::guarded_sites(&w.ast, "AttrJob::Size");
    if sites.is_empty() {
        return Err("walk.rs never builds an AttrJob::Size: folding is gone".into());
    }
    for s in sites {
        let guarded = s
            .enclosing_if_let_inits
            .iter()
            .any(|init| init.contains("classify_at"));
        // `process_size` recurses inside an already-folded unit;
        // `resize_artifact` re-sizes a path the store already classified.
        if !guarded && s.func != "process_size" && s.func != "resize_artifact" {
            return Err(format!(
                "walk::{}: builds AttrJob::Size outside an `if let Some(kind) = classify_at(..)`: a directory would be folded without being an artifact",
                s.func
            ));
        }
    }
    // 2. The serial walk records an artifact only under the same guard.
    let a = core(root, "attribution.rs")?;
    let sites = ast::guarded_sites(&a.ast, "record_artifact");
    if sites.is_empty() {
        return Err("attribution.rs never records an artifact".into());
    }
    for s in sites {
        if !s
            .enclosing_if_let_inits
            .iter()
            .any(|init| init.contains("classify_at"))
        {
            return Err(format!(
                "attribution::{}: record_artifact outside a classify_at guard",
                s.func
            ));
        }
    }
    // 3. classify_at is table-driven and refuses by default: its tail is
    //    `None`, and it constructs no ArtifactKind of its own.
    let funcs = ast::functions(&a.ast);
    let ca = ast::function(&funcs, "classify_at")?;
    let tail = ast::tail_of(ca);
    let table_driven = tail.trim() == "None"
        || [
            "ARTIFACT_KINDS",
            "MARKED_ARTIFACT_KINDS",
            "classify_gated",
            "classify (",
        ]
        .iter()
        .any(|t| tail.contains(t));
    if !table_driven || tail.contains("Some (ArtifactKind") || tail.contains("or (") {
        return Err(format!(
            "attribution::classify_at must end in a table lookup or `None` (unknown names are never folded); tail is `{tail}`"
        ));
    }
    if ca.body.contains("Some (ArtifactKind") || ca.body.contains("Some(ArtifactKind") {
        return Err(
            "attribution::classify_at invents a kind instead of consulting the tables".into(),
        );
    }
    for name in ["classify", "classify_marked"] {
        if let Ok(f) = ast::function(&funcs, name)
            && (f.body.contains("Some (ArtifactKind") || f.body.contains("Some(ArtifactKind"))
        {
            return Err(format!(
                "attribution::{name} invents a kind instead of consulting the tables"
            ));
        }
    }
    let e = core(root, "ecosystem.rs")?;
    let efuncs = ast::functions(&e.ast);
    let cg = ast::function(&efuncs, "classify_gated")?;
    if cg.body.contains("Some (ArtifactKind") || cg.body.contains("Some(ArtifactKind") {
        return Err(
            "ecosystem::classify_gated invents a kind instead of consulting ECOSYSTEMS".into(),
        );
    }
    Ok(())
}

fn symlinks_never_followed(root: &Path) -> Result<(), String> {
    for name in ["walk.rs", "attribution.rs"] {
        let f = core(root, name)?;
        // 1. Nothing in a walker asks the filesystem through a symlink:
        //    `fs::metadata` follows, `symlink_metadata` does not.
        for c in ast::call_paths(&f.ast) {
            if c == "metadata"
                || c.ends_with("::metadata")
                || c.ends_with("canonicalize") && !c.contains("dunce")
            {
                if c.ends_with("canonicalize") && name == "walk.rs" {
                    // The root itself may be canonicalized once; children never.
                    continue;
                }
                return Err(format!(
                    "{name}: calls `{c}`, which follows symlinks; use symlink_metadata"
                ));
            }
        }
        // 2. No type question on a *path* (`entry.path().is_dir()`,
        //    `x.join(y).exists()`): those stat through the link.
        for m in ["is_dir", "is_file", "exists"] {
            let hits = ast::method_on_receiver_methods(&f.ast, m, &["path", "join"]);
            if let Some(h) = hits.first() {
                return Err(format!(
                    "{name}: `{h}` asks about a path, which follows symlinks; decide on file_type()/symlink_metadata instead"
                ));
            }
        }
        // 3. In every directory loop that descends, the symlink guard
        //    comes before the directory branch.
        for lo in ast::descent_guard_order(&f.ast) {
            if let Some(d) = lo.descent
                && lo.guard.is_none_or(|g| g > d)
            {
                return Err(format!(
                    "{name}::{}: a directory loop descends (is_dir) before discarding symlinks (is_symlink)",
                    lo.func
                ));
            }
        }
    }
    Ok(())
}

fn incremental_walk_only_changed_subtrees(root: &Path) -> Result<(), String> {
    let g = core(root, "growth.rs")?;
    let funcs = ast::functions(&g.ast);
    let f = ast::function(&funcs, "apply_incremental")?;
    if !f.body.contains("attribute_one_worktree") || !f.body.contains("resize_artifact") {
        return Err(
            "growth::apply_incremental must re-walk only implicated worktrees/artifacts".into(),
        );
    }
    if f.body.contains("full_walk") || f.body.contains("attribute_parallel") {
        return Err("growth::apply_incremental falls back to a full walk".into());
    }
    Ok(())
}

fn walk_optimized_parallel_pool(root: &Path) -> Result<(), String> {
    let w = core(root, "walk.rs")?;
    let funcs = ast::functions(&w.ast);
    let f = ast::function(&funcs, "discover_and_attribute")?;
    if !f.body.contains("attribute_parallel") {
        return Err("walk::discover_and_attribute does not use the parallel walk".into());
    }
    let mut files = vec!["crates/core/src/report.rs".to_string()];
    files.extend(ast::rust_files_under(root, "crates/core/src/consumers"));
    for rel in files {
        let f = ast::parse(root, &rel)?;
        if ast::call_paths(&f.ast)
            .iter()
            .any(|p| p.ends_with("attribution::attribute"))
        {
            return Err(format!(
                "{rel}: calls the serial test-only walk on the report path"
            ));
        }
    }
    Ok(())
}

fn dir_mtime_int32_minutes(root: &Path) -> Result<(), String> {
    let g = core(root, "growth.rs")?;
    let funcs = ast::functions(&g.ast);
    for name in ["dirs_schema", "files_schema"] {
        let f = ast::function(&funcs, name)?;
        if !f.body.contains("\"mod_time_min\"") || !f.body.contains("Int32") {
            return Err(format!(
                "growth::{name}: mod_time_min must be an Int32 (minutes) column"
            ));
        }
    }
    Ok(())
}

const VERDICTS: &[&str] = &[
    "safe to delete",
    "safe to remove",
    "can be deleted",
    "should delete",
    "stale",
    "unused",
];

fn agent_interface_facts_not_verdicts(root: &Path) -> Result<(), String> {
    let mut files = vec![
        "crates/core/src/render.rs".to_string(),
        "crates/mcp/src/main.rs".to_string(),
    ];
    files.extend(ast::rust_files_under(root, "crates/tui/src"));
    for rel in files {
        let f = ast::parse(root, &rel)?;
        for lit in ast::string_literals(&f.ast) {
            let l = lit.to_ascii_lowercase();
            if let Some(v) = VERDICTS.iter().find(|v| l.contains(*v)) {
                return Err(format!(
                    "{rel}: string literal {lit:?} carries the verdict word {v:?}"
                ));
            }
        }
    }
    Ok(())
}

fn human_only_authorization(root: &Path) -> Result<(), String> {
    let f = ast::parse(root, "crates/mcp/src/main.rs")?;
    for ident in ast::referenced_idents(&f.ast) {
        if matches!(
            ident.as_str(),
            "approve" | "write_grants" | "revoke_grant" | "add_grant" | "grant_add"
        ) {
            return Err(format!(
                "mcp/main.rs reaches `{ident}`: the MCP server must not mint or change authorization"
            ));
        }
    }
    Ok(())
}

fn consumer_files(root: &Path) -> Result<Vec<SourceFile>, String> {
    let rels = ast::rust_files_under(root, "crates/core/src/consumers");
    if rels.is_empty() {
        return Err(
            "crates/core/src/consumers/ does not exist: the pipeline is not on the bus".into(),
        );
    }
    rels.iter()
        .filter(|r| !r.ends_with("/mod.rs"))
        .map(|r| ast::parse(root, r))
        .collect()
}

fn no_consumer_knows_other_consumers(root: &Path) -> Result<(), String> {
    let files = consumer_files(root)?;
    let all: Vec<(String, Vec<String>)> = files
        .iter()
        .map(|f| (f.rel.clone(), ast::impls_of(&f.ast, "Consumer")))
        .collect();
    for f in &files {
        let idents = ast::referenced_idents(&f.ast);
        if idents
            .iter()
            .any(|i| i == "EventBus" || i == "with_builtins")
        {
            return Err(format!("{}: a consumer names the bus", f.rel));
        }
        for (other_rel, structs) in &all {
            if other_rel == &f.rel {
                continue;
            }
            if let Some(s) = structs.iter().find(|s| idents.iter().any(|i| i == *s)) {
                return Err(format!(
                    "{}: names consumer `{s}` from {other_rel}; the bus is the only coupling",
                    f.rel
                ));
            }
        }
    }
    Ok(())
}

fn static_registration_only(root: &Path) -> Result<(), String> {
    let mut files = consumer_files(root)?;
    files.push(core(root, "bus/mod.rs")?);
    for f in &files {
        for func in ast::functions(&f.ast) {
            if func.name == "on_event"
                && (func.body.contains(". register (") || func.body.contains("with_builtins"))
            {
                return Err(format!(
                    "{}: on_event registers consumers at runtime",
                    f.rel
                ));
            }
        }
    }
    Ok(())
}

/// Stage entry points that may be called only from a consumer.
const STAGE_CALLS: &[&str] = &[
    "observe_tracked_with_source",
    "discover_and_attribute",
    "compute_signals_raw_parallel",
    "observe_all",
    "read_cached",
    "join_docker_facts",
    "observe_and_annotate",
    "observe_and_annotate_dirs",
    "observe_and_annotate_files",
    "annotate_readonly",
    "annotate_readonly_dirs",
    "annotate_readonly_files",
    "annotate_tracking",
    "history_series",
    "summarize",
];

fn all_report_paths_through_bus(root: &Path) -> Result<(), String> {
    let r = core(root, "report.rs")?;
    let calls = ast::call_paths(&r.ast);
    for c in &calls {
        let last = c.rsplit("::").next().unwrap_or(c);
        if STAGE_CALLS.contains(&last) {
            return Err(format!(
                "report.rs calls stage `{c}` directly instead of through the bus"
            ));
        }
    }
    if !calls.iter().any(|c| c.ends_with("run_report")) {
        return Err("report.rs never runs the bus (bus::run_report)".into());
    }
    let _ = consumer_files(root)?;
    Ok(())
}

fn extractors_are_pluggable(root: &Path) -> Result<(), String> {
    let files = consumer_files(root)?;
    let bus = core(root, "bus/mod.rs")?;
    let funcs = ast::functions(&bus.ast);
    let wb = ast::function(&funcs, "with_builtins")?;
    for f in &files {
        for s in ast::impls_of(&f.ast, "Consumer") {
            if !wb.body.contains(&format!("Box :: new ({s}"))
                && !wb.body.contains(&format!("Box::new({s}"))
            {
                return Err(format!(
                    "{}: consumer `{s}` is not registered in EventBus::with_builtins",
                    f.rel
                ));
            }
        }
    }
    let w = core(root, "walk.rs")?;
    if ast::referenced_idents(&w.ast)
        .iter()
        .any(|i| i == "consumers")
    {
        return Err(
            "walk.rs reaches into consumers/: a new source must not require walker changes".into(),
        );
    }
    Ok(())
}

fn frontmatter_list(text: &str, key: &str) -> Vec<String> {
    // `key:` at any indent inside the leading `---` block, followed by `- item` lines.
    let mut out = Vec::new();
    let mut in_front = false;
    let mut in_key = false;
    for (i, line) in text.lines().enumerate() {
        if line.trim() == "---" {
            if i == 0 {
                in_front = true;
                continue;
            }
            break;
        }
        if !in_front {
            continue;
        }
        let t = line.trim();
        if t == format!("{key}:") {
            in_key = true;
            continue;
        }
        if in_key {
            if let Some(item) = t.strip_prefix("- ") {
                out.push(item.trim().to_string());
            } else {
                in_key = false;
            }
        }
    }
    out
}

fn frontmatter_value(text: &str, key: &str) -> Option<String> {
    text.lines()
        .skip(1)
        .take_while(|l| l.trim() != "---")
        .find_map(|l| {
            l.strip_prefix(&format!("{key}:"))
                .map(|v| v.trim().trim_matches('"').to_string())
        })
}

fn adr_validation(root: &Path) -> Result<(), String> {
    let names: Vec<&str> = AUDITS.iter().map(|(n, _)| *n).collect();
    // Guardrails: every `audit:` resolves.
    let gdir = root.join(".oh/guardrails");
    let mut seen = 0;
    for e in std::fs::read_dir(&gdir)
        .map_err(|e| format!("read .oh/guardrails: {e}"))?
        .flatten()
    {
        let text = std::fs::read_to_string(e.path()).map_err(|e| e.to_string())?;
        seen += 1;
        if let Some(a) = frontmatter_value(&text, "audit")
            && a != "none"
            && !names.contains(&a.as_str())
        {
            return Err(format!(
                "{}: names audit `{a}` which does not exist",
                e.path().display()
            ));
        }
    }
    if seen == 0 {
        return Err(".oh/guardrails is empty".into());
    }
    // ADRs: every audit resolves; every cargo test exists as a fn somewhere.
    let mut all_rust = String::new();
    for krate in ["core", "cli", "mcp", "tui"] {
        for rel in ast::rust_files_under(root, &format!("crates/{krate}/src")) {
            all_rust.push_str(&std::fs::read_to_string(root.join(&rel)).unwrap_or_default());
        }
        for rel in ast::rust_files_under(root, &format!("crates/{krate}/src/bus"))
            .into_iter()
            .chain(ast::rust_files_under(
                root,
                &format!("crates/{krate}/src/consumers"),
            ))
        {
            all_rust.push_str(&std::fs::read_to_string(root.join(&rel)).unwrap_or_default());
        }
    }
    let adr_dir = root.join("docs/ADRs");
    for e in std::fs::read_dir(&adr_dir)
        .map_err(|e| format!("read docs/ADRs: {e}"))?
        .flatten()
    {
        if e.path().extension().and_then(|x| x.to_str()) != Some("md") {
            continue;
        }
        let text = std::fs::read_to_string(e.path()).map_err(|e| e.to_string())?;
        for a in frontmatter_list(&text, "audits") {
            if !names.contains(&a.as_str()) {
                return Err(format!(
                    "{}: names audit `{a}` which does not exist",
                    e.path().display()
                ));
            }
        }
        for t in frontmatter_list(&text, "cargo_tests") {
            let fn_name = t.rsplit("::").next().unwrap_or(&t);
            if !all_rust.contains(&format!("fn {fn_name}(")) {
                return Err(format!(
                    "{}: names test `{t}` which does not exist",
                    e.path().display()
                ));
            }
        }
    }
    Ok(())
}
