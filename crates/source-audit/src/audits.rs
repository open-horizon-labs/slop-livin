//! One named audit per guardrail in `.oh/guardrails/`. Each reads the AST
//! (`ast.rs`) and returns `Err(why)` naming the file and the shape that
//! broke the constraint. `adr_validation` closes the loop: every guardrail
//! `audit:` and every ADR `validate:` entry must resolve to an audit here
//! or a test that exists.

use crate::ast::{self, SourceFile};
use std::path::Path;

pub type Audit = fn(&Path) -> Result<(), String>;

pub const AUDITS: &[(&str, Audit)] = &[
    (
        "tui_actions_off_event_thread",
        crate::tui_nonblocking::audit,
    ),
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
    // The 2026-09-21 review repairs (`repair_audits.rs`). Landed before
    // the repairs themselves, deliberately failing, so the baseline is
    // the audit's own enumeration rather than a prose list.
    (
        "execution_sinks_recheck_live_state",
        crate::repair_audits::execution_sinks_recheck_live_state,
    ),
    (
        "protection_fails_closed",
        crate::repair_audits::protection_fails_closed,
    ),
    (
        "discovery_consumes_effective_scope",
        crate::repair_audits::discovery_consumes_effective_scope,
    ),
    (
        "explicit_only_scope_when_defaults_false",
        crate::repair_audits::explicit_only_scope_when_defaults_false,
    ),
    (
        "history_sweeps_are_owned",
        crate::repair_audits::history_sweeps_are_owned,
    ),
    (
        "no_second_traversal_on_report_path",
        crate::repair_audits::no_second_traversal_on_report_path,
    ),
    (
        "occupancy_is_tristate_at_sinks",
        crate::repair_audits::occupancy_is_tristate_at_sinks,
    ),
    (
        "tui_refresh_preserves_scope",
        crate::repair_audits::tui_refresh_preserves_scope,
    ),
    (
        "store_data_is_parquet_not_json_sidecars",
        crate::repair_audits::store_data_is_parquet_not_json_sidecars,
    ),
    (
        "json_persistence_is_allowlisted",
        crate::repair_audits::json_persistence_is_allowlisted,
    ),
    (
        "agent_adapters_are_pluggable",
        crate::repair_audits::agent_adapters_are_pluggable,
    ),
    (
        "agent_adapters_read_bounded_headers_only",
        crate::repair_audits::agent_adapters_read_bounded_headers_only,
    ),
    (
        "agent_adapters_do_not_traverse",
        crate::repair_audits::agent_adapters_do_not_traverse,
    ),
    (
        "agent_adapters_are_inspection_only",
        crate::repair_audits::agent_adapters_are_inspection_only,
    ),
    (
        "agent_adapters_do_not_emit_content",
        crate::repair_audits::agent_adapters_do_not_emit_content,
    ),
    (
        "agent_units_built_through_builder",
        crate::repair_audits::agent_units_built_through_builder,
    ),
    (
        "agent_adapters_are_environment_free",
        crate::repair_audits::agent_adapters_are_environment_free,
    ),
    (
        "agent_adapters_do_not_reach_detectors",
        crate::repair_audits::agent_adapters_do_not_reach_detectors,
    ),
    (
        "agent_adapter_test_contract",
        crate::repair_audits::agent_adapter_test_contract,
    ),
    (
        "detector_ids_only_in_registry",
        crate::repair_audits::detector_ids_only_in_registry,
    ),
    (
        "discovery_owned_by_report_pipeline",
        crate::repair_audits::discovery_owned_by_report_pipeline,
    ),
    (
        "no_dead_public_evidence_api",
        crate::repair_audits::no_dead_public_evidence_api,
    ),
    // The 2026-09-22 re-review: the three guardrails with no audit were
    // the three it broke. Landed before their repairs, deliberately
    // failing (`.oh/sessions/2026-09-22-review-2-repairs.md`).
    (
        "computed_but_not_delivered",
        crate::repair_audits::computed_but_not_delivered,
    ),
    (
        "coverage_changes_are_not_storage_changes",
        crate::repair_audits::coverage_changes_are_not_storage_changes,
    ),
    (
        "activity_and_consumer_evidence_have_limits",
        crate::repair_audits::activity_and_consumer_evidence_have_limits,
    ),
    // GUARDRAILS_SPEC.md section 18: the build-artifact adapter set,
    // the mirror of sections 13/14 for `crates/core/src/build_adapters/`.
    // Landed before the adapters themselves, deliberately failing, so
    // the baseline is the audit's own enumeration
    // (`.oh/sessions/2026-09-21-build-adapters-node-jvm.md`).
    (
        "build_adapters_are_pluggable",
        crate::build_audits::build_adapters_are_pluggable,
    ),
    (
        "build_adapters_are_inspection_only",
        crate::build_audits::build_adapters_are_inspection_only,
    ),
    (
        "build_adapters_read_bounded_manifests_only",
        crate::build_audits::build_adapters_read_bounded_manifests_only,
    ),
    (
        "build_adapters_do_not_traverse",
        crate::build_audits::build_adapters_do_not_traverse,
    ),
    (
        "build_units_built_through_builder",
        crate::build_audits::build_units_built_through_builder,
    ),
    (
        "build_adapter_test_contract",
        crate::build_audits::build_adapter_test_contract,
    ),
    (
        "build_adapter_matrix_matches_docs",
        crate::build_audits::build_adapter_matrix_matches_docs,
    ),
    (
        "build_adapters_reuse_under_event_coverage",
        crate::build_audits::build_adapters_reuse_under_event_coverage,
    ),
    // Landed before the store join it governs, failing: machine-wide
    // stores reach an adapter by declared capability, never by an id
    // (`.oh/sessions/2026-09-21-build-adapters-python-go-apple-android-docker.md`).
    (
        "build_stores_join_by_capability",
        crate::build_audits::build_stores_join_by_capability,
    ),
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
        .contains("pub use swamp_core::render::human_bytes_pub as human_bytes")
    {
        return Err("tui/src/model.rs must re-export core's byte formatter".into());
    }
    let render = core(root, "render.rs")?;
    let f = ast::functions(&render.ast);
    let hb = ast::function(&f, "human_bytes")?;
    if !hb.body.contains("1000") || hb.body.contains("1024") {
        return Err("render::human_bytes must divide by 1000 under SI labels".into());
    }
    // "One formatter" is about the whole workspace, not about the name
    // `human_bytes`. The mutation sweep added a *second* formatter
    // beside the re-export: `n / 1024` with a `KiB` label, which no
    // check on `human_bytes` could see. Any function anywhere that
    // divides a byte count by 1024 and labels the result is a second
    // formatter, whatever it is called.
    let mut second: Vec<String> = Vec::new();
    for rel in crate::resolve::workspace_files(root) {
        let Some(sf) = crate::resolve::maybe(root, &rel) else {
            continue;
        };
        let literals = ast::string_literals(&sf.ast);
        let labels_binary = literals
            .iter()
            .any(|l| ["KiB", "MiB", "GiB", "TiB"].iter().any(|u| l.contains(u)));
        if !labels_binary {
            continue;
        }
        // A *formatter* produces the string; a parser consumes one.
        // `docker::parse_size` reads the daemon's own "1.5GiB" strings
        // through `strip_suffix`, which is the opposite job.
        let formatting_macros: Vec<crate::resolve::MacroSite> =
            crate::resolve::macro_sites(&sf.ast)
                .into_iter()
                .filter(|m| {
                    !m.in_test
                        && ["format", "write", "writeln", "push_str"].contains(&m.name.as_str())
                        && m.literals
                            .iter()
                            .any(|l| ["KiB", "MiB", "GiB", "TiB"].iter().any(|u| l.contains(u)))
                })
                .collect();
        for func in ast::functions(&sf.ast) {
            let divides = func.body.contains("1024");
            let labels = formatting_macros.iter().any(|m| m.func == func.name);
            if divides && labels {
                second.push(format!(
                    "{rel}::{} formats bytes with a 1024 divisor and a binary unit label: there \
                     is one byte formatter (render::human_bytes, SI, /1000) and everything else \
                     re-exports it",
                    func.name
                ));
            }
        }
    }
    if !second.is_empty() {
        second.sort();
        second.dedup();
        return Err(second.join("\n  "));
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
    // A name is not the invariant. Every definition called
    // `canonical_roots` has to canonicalize -- one that returns its
    // argument unchanged satisfies the reference check above while the
    // boundary does nothing (slip class 4: an existence check standing
    // in for semantics). Every definition, not the first: a second one
    // beside the real one is the same defect.
    for f in ast::functions(&scan.ast)
        .iter()
        .filter(|f| f.name == "canonical_roots")
    {
        if !f.body.contains("canonicalize") {
            return Err(format!(
                "scan.rs::{} is named `canonical_roots` and never calls `canonicalize`: the \
                 boundary invariant is the canonicalization, not the name",
                f.name
            ));
        }
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
    // *Every* definition with these names, not the first one found: the
    // mutation sweep's variants live beside the audited original, and a
    // rule that stops at the first match inspects whichever one it
    // happens to reach.
    let entries: Vec<&ast::Func> = funcs
        .iter()
        .filter(|f| f.name == "observe_tracked_with_source")
        .collect();
    if entries.is_empty() {
        return Err("growth.rs defines no `observe_tracked_with_source`".into());
    }
    for entry in entries {
        if !entry.body.contains("stage_tracked_with_source") || entry.body.contains("full_walk") {
            return Err(
                "observe_tracked_with_source must delegate replay planning to \
                 stage_tracked_with_source and must not walk fully itself"
                    .into(),
            );
        }
    }
    let staged: Vec<&ast::Func> = funcs
        .iter()
        .filter(|f| f.name == "stage_tracked_with_source")
        .collect();
    if staged.is_empty() {
        return Err("growth.rs defines no `stage_tracked_with_source`".into());
    }
    for f in staged {
        let replay_at = f
            .stmts
            .iter()
            .position(|s| s.contains(". replay (") || s.contains(".replay("))
            .ok_or("stage_tracked_with_source never calls FSEvents replay")?;
        for (i, s) in f.stmts.iter().enumerate() {
            if i < replay_at && s.contains("full_walk") && !s.contains("force_full") {
                return Err(format!(
                    "stage_tracked_with_source: statement {i} runs full_walk before the FSEvents \
                     replay without a force_full guard"
                ));
            }
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
        for f in ast::functions_named(&funcs, name)? {
            if !f.body.contains(current) || !f.body.contains(delta) {
                return Err(format!(
                    "growth::{name} must rewrite the current file ({current}) and append a delta ({delta})"
                ));
            }
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
        // `process_size` recurses inside an already-folded unit; the
        // `resize_artifact*` family (the `_with_dirs`, `_excluding` and
        // `_stamped` variants) re-sizes a path the caller already
        // classified -- the exclusion list only prunes what is folded
        // and the stamps only record what was listed, neither widens
        // what may be folded without a classify_at guard. A prefix
        // rather than a list of three spellings: the 2026-09-22 mutation
        // sweep's first structural finding is that a rename defeats a
        // name list, and this audit's own list went stale the first time
        // the entry point was renamed.
        if !guarded && s.func != "process_size" && !s.func.starts_with("resize_artifact") {
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
    // Until the resolver landed, this check read the *written* text and
    // so could not see `use ArtifactKind::{DependencyTree as Deps}`
    // followed by `Some(Deps)` -- slip class 1. Now that it can, the
    // rule has to say what is actually allowed rather than what the
    // alias happened to hide: a kind may be produced from the
    // ECOSYSTEMS table, or under a guard calling a *convention
    // predicate* defined in this module whose own body names a manifest
    // marker file. `if name == "node_modules" { return Some(Cache) }`
    // has no such guard and is rejected; `ruby_vendor_bundle`, which
    // requires a sibling `Gemfile`, has one.
    if cg.body.contains("Some (ArtifactKind") || cg.body.contains("Some(ArtifactKind") {
        let guards: Vec<&str> = cg
            .body
            .split("Some (ArtifactKind")
            .next()
            .into_iter()
            .collect();
        let predicate = efuncs.iter().find(|f| {
            f.name != "classify_gated"
                && guards.iter().any(|g| g.contains(&format!("{} (", f.name)))
                && (f.body.contains("marker_present") || f.body.contains("manifest"))
        });
        let Some(predicate) = predicate else {
            return Err(
                "ecosystem::classify_gated invents a kind instead of consulting ECOSYSTEMS or a \
                 marker-file convention predicate defined in this module"
                    .into(),
            );
        };
        let _ = predicate;
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
    for f in ast::functions_named(&funcs, "apply_incremental")? {
        if !f.body.contains("attribute_one_worktree") || !f.body.contains("resize_artifact") {
            return Err(
                "growth::apply_incremental must re-walk only implicated worktrees/artifacts".into(),
            );
        }
        if f.body.contains("full_walk") || f.body.contains("attribute_parallel") {
            return Err("growth::apply_incremental falls back to a full walk".into());
        }
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
    // Every definition with the name, not the first: a second, wrong
    // `dirs_schema` beside the right one is how "seconds in the minutes
    // column" walked past this audit.
    for name in ["dirs_schema", "files_schema"] {
        for f in ast::functions_named(&funcs, name)? {
            if !f.body.contains("\"mod_time_min\"") || !f.body.contains("Int32") {
                return Err(format!(
                    "growth::{name}: mod_time_min must be an Int32 (minutes) column"
                ));
            }
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

/// Every file whose strings might reach a human or an agent verbatim:
/// the text renderer, the bounded agent-facing JSON shaper, the CLI, and
/// the TUI. Transport-independent by construction -- it does not matter
/// which of these binaries a given string ships through, only that none
/// of them ever asserts a verdict.
fn agent_interface_facts_not_verdicts(root: &Path) -> Result<(), String> {
    let mut files = vec![
        "crates/core/src/render.rs".to_string(),
        "crates/core/src/agent_json.rs".to_string(),
    ];
    files.extend(ast::rust_files_under(root, "crates/cli/src"));
    files.extend(ast::rust_files_under(root, "crates/tui/src"));
    for rel in files {
        let f = ast::parse(root, &rel)?;
        // A macro this layer cannot see through is not a pass: the sweep
        // hid `concat!("can", " be deleted")` from this rule.
        let blind = crate::resolve::unknown_macros(&f.ast);
        if let Some((func, name)) = blind.first() {
            return Err(format!(
                "{rel}::{func} emits through `{name}!`, which the resolver cannot see through; a \
                 verdict assembled inside an unknown macro would be invisible here"
            ));
        }
        for lit in ast::string_literals(&f.ast) {
            let l = strip_negations(&lit.to_ascii_lowercase());
            if let Some(v) = VERDICTS.iter().find(|v| l.contains(*v)) {
                return Err(format!(
                    "{rel}: string literal {lit:?} carries the verdict word {v:?}"
                ));
            }
        }
    }
    Ok(())
}

/// Phrases that *deny* a verdict. "unchecked = review required, not
/// proven unused" is the guardrail being honoured in prose, not broken;
/// scanning for the bare word would ban the sentence that exists to say
/// the tool does not conclude it.
///
/// This is deliberately a list of whole negating phrases rather than a
/// "preceded by not" rule: `"not safe to keep"` would otherwise pass, and
/// that *is* a verdict.
const NEGATED_VERDICTS: &[&str] = &[
    "not proven unused",
    "not proven safe",
    "not proven stale",
    "never unused",
    "never safe",
    "never stale",
    "not a verdict",
];

fn strip_negations(lit: &str) -> String {
    let mut out = lit.to_string();
    for phrase in NEGATED_VERDICTS {
        out = out.replace(phrase, "");
    }
    out
}

/// The three functions that mint or change authorization
/// (`swamp_core::actions::approve`, `add_standing_grant`,
/// `revoke_grant`). This audit is transport-independent: it does not
/// check which *binary* calls them (a shell-capable agent can invoke
/// any binary in this workspace), only which *named function* in the
/// source calls them directly. That is a real, checkable boundary --
/// "authorization is minted from exactly these reviewed call sites and
/// nowhere else" -- unlike "an agent cannot reach this", which no
/// static check can honestly claim. See
/// `.oh/guardrails/human-only-authorization.md` and
/// `skills/swamp/references/trust-model.md`.
const AUTH_MINTING_SINKS: &[&str] = &["approve", "add_standing_grant", "revoke_grant"];

/// `(file, allowed function/method names)`: the only call sites in the
/// workspace allowed to reach an `AUTH_MINTING_SINKS` function. The
/// CLI's own subcommand handling for `approve`/`grant add`/`grant
/// revoke` is factored into these three named functions precisely so
/// this list can name them; the TUI's confirmed-execution path is
/// `execute_one`, reached only after its own confirm-prompt flow.
const AUTH_MINTING_ALLOWED_CALLERS: &[(&str, &[&str])] = &[
    (
        "crates/cli/src/main.rs",
        &["cmd_approve", "cmd_grant_add", "cmd_grant_revoke"],
    ),
    ("crates/tui/src/actions.rs", &["execute_one"]),
];

fn human_only_authorization(root: &Path) -> Result<(), String> {
    let mut files = Vec::new();
    for krate in ["core", "cli", "tui"] {
        files.extend(ast::rust_files_under(root, &format!("crates/{krate}/src")));
    }
    for rel in files {
        // `actions.rs` is the sinks' own definition file: `approve`
        // calling its own internal `write_grants` helper, or `execute`
        // spending an already-minted grant's budget, is the sinks'
        // ordinary internal wiring, not a new minting call site.
        if rel == "crates/core/src/actions.rs" {
            continue;
        }
        let allowed: &[&str] = AUTH_MINTING_ALLOWED_CALLERS
            .iter()
            .find(|(f, _)| *f == rel)
            .map(|(_, fns)| *fns)
            .unwrap_or(&[]);
        let f = ast::parse(root, &rel)?;
        for func in ast::functions(&f.ast) {
            let hits: Vec<&str> = AUTH_MINTING_SINKS
                .iter()
                .copied()
                .filter(|sink| func.body.contains(&format!("actions :: {sink} (")))
                .collect();
            if !hits.is_empty() && !allowed.contains(&func.name.as_str()) {
                return Err(format!(
                    "{rel}: `{}` calls authorization-minting function(s) {hits:?} outside the reviewed CLI approve/grant command handling or TUI confirmation path -- see .oh/guardrails/human-only-authorization.md",
                    func.name
                ));
            }
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
        if idents.iter().any(|i| i == "EventBus") {
            return Err(format!("{}: a consumer names the bus", f.rel));
        }
        // `with_builtins` is a constructor name three static registries
        // share (`bus::EventBus`, `agents::Registry`,
        // `locations::Registry`, and now `build_adapters::Registry`), so
        // matching the bare identifier reported a consumer using its own
        // domain's registry as if it were assembling the bus.
        //
        // The property this rule is about is *whose* builtin set: a
        // consumer must not assemble the set of consumers. So the call is
        // rejected when it is unqualified (the shape the mutation corpus
        // uses) or qualified by the bus, and allowed when it names some
        // other module's registry explicitly -- which is visible in the
        // diff and cannot be the consumer set.
        for c in crate::resolve::calls(&f.ast) {
            if !crate::resolve::path_ends_with(&c.path, "with_builtins") {
                continue;
            }
            let qualifier = c
                .path
                .trim_end_matches("with_builtins")
                .trim_end_matches("::");
            let assembles_the_bus = qualifier.is_empty()
                || qualifier == "Self"
                || qualifier.contains("bus")
                || qualifier.contains("EventBus");
            if assembles_the_bus {
                return Err(format!(
                    "{}: `{}` assembles a builtin set with no module qualifying it; a consumer                      never builds the consumer set, and an unqualified `with_builtins` cannot be                      shown not to",
                    f.rel, c.written
                ));
            }
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
    "stage_tracked_with_source",
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
    for krate in ["core", "cli", "tui"] {
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

#[cfg(test)]
mod human_only_authorization_tests {
    use super::human_only_authorization;
    use std::fs;
    use std::path::Path;

    /// A minimal fake workspace: `crates/core/src/actions.rs` (the
    /// sinks' own definitions, always skipped), plus whatever extra
    /// `(rel_path, contents)` files the case supplies.
    fn fake_workspace(extra: &[(&str, &str)]) -> tempfile::TempDir {
        let tmp = tempfile::tempdir().expect("tempdir");
        let write = |rel: &str, text: &str| {
            let p = tmp.path().join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, text).unwrap();
        };
        write(
            "crates/core/src/actions.rs",
            "pub fn approve() { write_grants(); }\n\
             fn write_grants() {}\n\
             pub fn add_standing_grant() { write_grants(); }\n\
             pub fn revoke_grant() { write_grants(); }\n",
        );
        for (rel, text) in extra {
            write(rel, text);
        }
        tmp
    }

    #[test]
    fn allowed_cli_and_tui_call_sites_pass() {
        let tmp = fake_workspace(&[
            (
                "crates/cli/src/main.rs",
                "fn cmd_approve() { actions::approve(); }\n\
                 fn cmd_grant_add() { actions::add_standing_grant(); }\n\
                 fn cmd_grant_revoke() { actions::revoke_grant(); }\n",
            ),
            (
                "crates/tui/src/actions.rs",
                "fn execute_one() { swamp_core::actions::approve(); }\n",
            ),
        ]);
        assert_eq!(human_only_authorization(tmp.path()), Ok(()));
    }

    #[test]
    fn a_call_site_outside_the_allowlist_is_rejected() {
        let tmp = fake_workspace(&[(
            "crates/cli/src/main.rs",
            // Same call, wrong function name: not one of the reviewed
            // command handlers the allowlist names.
            "fn main() { actions::approve(); }\n",
        )]);
        let err = human_only_authorization(tmp.path()).unwrap_err();
        assert!(err.contains("main"), "{err}");
        assert!(err.contains("approve"), "{err}");
    }

    #[test]
    fn a_new_call_site_in_core_outside_actions_rs_is_rejected() {
        let tmp = fake_workspace(&[(
            "crates/core/src/growth.rs",
            "fn refresh() { crate::actions::add_standing_grant(); }\n",
        )]);
        let err = human_only_authorization(tmp.path()).unwrap_err();
        assert!(err.contains("growth.rs"), "{err}");
    }

    #[test]
    fn internal_wiring_inside_actions_rs_itself_is_not_flagged() {
        // approve/add_standing_grant/revoke_grant calling their own
        // private write_grants helper is ordinary internal wiring, not
        // a new external minting call site -- actions.rs is skipped
        // entirely.
        let tmp = fake_workspace(&[]);
        assert_eq!(human_only_authorization(tmp.path()), Ok(()));
    }

    #[test]
    fn repository_call_sites_match_the_allowlist() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        human_only_authorization(root).unwrap();
    }
}
