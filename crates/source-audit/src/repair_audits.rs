//! The audits added after the 2026-09-21 independent review
//! (`.oh/sessions/2026-09-21-foundation-repairs.md`). Each one names a
//! specific falsified assumption -- an approved plan spending itself on
//! replaced data, a discovery pass reading detector output instead of
//! the authorized scope, a history sweep tombstoning another family's
//! rows -- and makes the repaired shape structural rather than a habit.
//!
//! These landed *before* the repairs, deliberately failing, so the
//! baseline is the audit's own enumeration of violations rather than a
//! prose list someone has to trust.

use crate::ast::{self, SourceFile};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

// ---------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------

fn parse(root: &Path, rel: &str) -> Result<SourceFile, String> {
    ast::parse(root, rel)
}

fn maybe_parse(root: &Path, rel: &str) -> Option<SourceFile> {
    ast::parse(root, rel).ok()
}

/// Every Rust file under `crates/{core,cli,tui}/src`, including the
/// nested `agents/`, `bus/`, `consumers/` and `locations/` directories.
fn workspace_src_files(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for krate in ["core", "cli", "tui"] {
        let base = format!("crates/{krate}/src");
        out.extend(ast::rust_files_under(root, &base));
        for sub in ["agents", "bus", "consumers", "locations"] {
            out.extend(ast::rust_files_under(root, &format!("{base}/{sub}")));
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Modules under `crates/core/src/agents/` that are *adapters*: the
/// per-tool identification code the adapter guardrails govern.
/// `mod.rs` is the shared model, `matrix.rs` the required-tool catalog,
/// `registry.rs` the static registration, and the neutral helpers are
/// shared mechanics that name no tool.
const AGENT_NON_ADAPTERS: &[&str] = &[
    "mod.rs",
    "matrix.rs",
    "registry.rs",
    "vscode_family.rs",
    "pi_family.rs",
    "bounded_io.rs",
];

fn agent_adapter_files(root: &Path) -> Vec<String> {
    ast::rust_files_under(root, "crates/core/src/agents")
        .into_iter()
        .filter(|rel| {
            let name = rel.rsplit('/').next().unwrap_or(rel);
            !AGENT_NON_ADAPTERS.contains(&name)
        })
        .collect()
}

fn adapter_module_name(rel: &str) -> String {
    rel.rsplit('/')
        .next()
        .unwrap_or(rel)
        .trim_end_matches(".rs")
        .to_string()
}

/// Token-stream text search. `syn`'s `ToTokens` spaces punctuation, so a
/// path written `fs::rename` becomes `fs :: rename`. Every needle in
/// this module is written in that spaced form.
fn find_first(haystack: &str, needles: &[&str]) -> Option<(usize, String)> {
    needles
        .iter()
        .filter_map(|n| haystack.find(n).map(|i| (i, (*n).to_string())))
        .min_by_key(|(i, _)| *i)
}

// ---------------------------------------------------------------------
// 1. execution_sinks_recheck_live_state
// ---------------------------------------------------------------------

/// Calls that move or remove user data. A function whose body contains
/// one of these is a destructive sink and must have rechecked live state
/// first.
const DESTRUCTIVE_CALLS: &[&str] = &[
    ":: rename (",
    ":: remove_file (",
    ":: remove_dir (",
    ":: remove_dir_all (",
    ":: move_reviewed (",
    "docker :: remove (",
];

/// The three rechecks, by the exact names the repair introduces.
const RECHECK_SNAPSHOT: &str = "reviewed_snapshot (";
const RECHECK_PROTECTION: &str = "live_protection (";
const RECHECK_OCCUPANCY: &str = "member_occupancy (";

/// Files that define destructive sinks. `recheck.rs` is the recheck
/// model itself and `growth.rs`/`store.rs` rename their own Parquet temp
/// files (store bookkeeping, never user data), so both are excluded.
const SINK_FILES: &[&str] = &[
    "crates/core/src/actions.rs",
    "crates/core/src/cargo_cleanup.rs",
    "crates/core/src/docker.rs",
];

/// `(fn, why)`: functions inside the sink files whose rename/remove
/// touches swamp's *own* store bookkeeping, never a path the user asked
/// about. Publishing a control file by temp-then-rename is the atomic
/// write discipline, not a destructive action on user data. The spec's
/// allow-list ("the recheck module itself, test code, and the Trash
/// backend's internal file ops") named this category; these are the
/// concrete members of it.
const STORE_BOOKKEEPING_FNS: &[(&str, &str)] = &[
    (
        "save_plan",
        "publishes an unapproved plan file into the store by temp + rename",
    ),
    (
        "write_restore_manifest",
        "writes a Trash envelope's own recovery manifest beside content already moved",
    ),
    (
        "write_atomic",
        "the shared temp + rename primitive every small control file is written through",
    ),
];

#[derive(Default, Clone, Copy, PartialEq, Eq)]
struct Rechecks {
    snapshot: bool,
    protection: bool,
    occupancy: bool,
}

impl Rechecks {
    fn complete(self) -> bool {
        self.snapshot && self.protection && self.occupancy
    }
    fn union(self, other: Self) -> Self {
        Self {
            snapshot: self.snapshot || other.snapshot,
            protection: self.protection || other.protection,
            occupancy: self.occupancy || other.occupancy,
        }
    }
    fn missing(self) -> Vec<&'static str> {
        let mut m = Vec::new();
        if !self.snapshot {
            m.push("recheck::reviewed_snapshot");
        }
        if !self.protection {
            m.push("recheck::live_protection");
        }
        if !self.occupancy {
            m.push("recheck::member_occupancy");
        }
        m
    }
}

/// Rechecks a function performs anywhere in its body, following calls to
/// other functions in the same crate transitively.
fn rechecks_provided(
    name: &str,
    bodies: &HashMap<String, String>,
    seen: &mut HashSet<String>,
) -> Rechecks {
    if !seen.insert(name.to_string()) {
        return Rechecks::default();
    }
    let Some(body) = bodies.get(name) else {
        return Rechecks::default();
    };
    let mut r = Rechecks {
        snapshot: body.contains(RECHECK_SNAPSHOT),
        protection: body.contains(RECHECK_PROTECTION),
        occupancy: body.contains(RECHECK_OCCUPANCY),
    };
    for callee in bodies.keys() {
        if callee != name && body.contains(&format!("{callee} (")) {
            r = r.union(rechecks_provided(callee, bodies, seen));
        }
    }
    r
}

pub fn execution_sinks_recheck_live_state(root: &Path) -> Result<(), String> {
    // Every function in the crate, so helper credit can be followed
    // across files (a sink in actions.rs may recheck through a helper
    // defined in recheck.rs).
    let mut bodies: HashMap<String, String> = HashMap::new();
    for rel in workspace_src_files(root) {
        let Some(f) = maybe_parse(root, &rel) else {
            continue;
        };
        for func in ast::functions(&f.ast) {
            bodies.entry(func.name.clone()).or_insert(func.body);
        }
    }

    for rel in SINK_FILES {
        let f = parse(root, rel)?;
        for func in ast::functions(&f.ast) {
            if STORE_BOOKKEEPING_FNS.iter().any(|(n, _)| *n == func.name) {
                continue;
            }
            let Some((destructive_at, call)) = find_first(&func.body, DESTRUCTIVE_CALLS) else {
                continue;
            };
            let prefix = &func.body[..destructive_at];
            let mut have = Rechecks {
                snapshot: prefix.contains(RECHECK_SNAPSHOT),
                protection: prefix.contains(RECHECK_PROTECTION),
                occupancy: prefix.contains(RECHECK_OCCUPANCY),
            };
            for callee in bodies.keys() {
                if callee != &func.name && prefix.contains(&format!("{callee} (")) {
                    let mut seen = HashSet::new();
                    have = have.union(rechecks_provided(callee, &bodies, &mut seen));
                }
            }
            if !have.complete() {
                return Err(format!(
                    "{rel}::{} performs `{}` without {} first -- every sink that moves user data \
                     rechecks identity, protection and occupancy before its first destructive \
                     call (see .oh/guardrails/execution-sinks-recheck-live-state.md)",
                    func.name,
                    call.trim(),
                    have.missing().join(" + ")
                ));
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------
// 2. protection_fails_closed
// ---------------------------------------------------------------------

/// Ways to turn a `Result` protection lookup back into "nothing is
/// protected". Every one of them reopens the hole the review found.
const ERROR_DISCARDS: &[&str] = &[
    ". unwrap_or_default ()",
    ". unwrap_or (",
    ". unwrap_or_else (",
    ". ok ()",
    ". unwrap_or_default()",
];

pub fn protection_fails_closed(root: &Path) -> Result<(), String> {
    let agents = parse(root, "crates/core/src/agents/mod.rs")?;
    let funcs = ast::functions(&agents.ast);

    // Both directions of the containment test must be present.
    let conflict = ast::function(&funcs, "protection_conflict").or_else(|_| {
        ast::function(&funcs, "is_human_protected").map_err(|_| {
            "agents/mod.rs defines neither `protection_conflict` nor `is_human_protected`"
                .to_string()
        })
    })?;
    let candidate_under = conflict.body.contains("candidate . starts_with (");
    let protected_under = conflict.body.contains("p . starts_with (candidate)")
        || conflict.body.contains("p . starts_with (candidate )");
    if !candidate_under || !protected_under {
        return Err(format!(
            "agents/mod.rs::{} tests protection in only one direction (candidate-under-protected: \
             {candidate_under}, protected-under-candidate: {protected_under}). Protecting \
             `debug/log.txt` must also stop removing `debug/` -- see the review's \
             protected_descendant_must_prevent_parent_cache_proposal counterexample",
            conflict.name
        ));
    }

    // Atomic writes.
    for name in ["protect_add", "protect_remove"] {
        let f = ast::function(&funcs, name)?;
        let mut seen = HashSet::new();
        let mut bodies: HashMap<String, String> = HashMap::new();
        for func in ast::functions(&agents.ast) {
            bodies.insert(func.name.clone(), func.body);
        }
        fn reaches(
            name: &str,
            target: &str,
            bodies: &HashMap<String, String>,
            seen: &mut HashSet<String>,
        ) -> bool {
            if !seen.insert(name.to_string()) {
                return false;
            }
            let Some(body) = bodies.get(name) else {
                return false;
            };
            if body.contains(&format!("{target} (")) {
                return true;
            }
            bodies.keys().any(|c| {
                c != name && body.contains(&format!("{c} (")) && reaches(c, target, bodies, seen)
            })
        }
        if !reaches(&f.name, "write_atomic", &bodies, &mut seen) {
            return Err(format!(
                "agents/mod.rs::{name} does not write the protect list through `write_atomic` \
                 (temp file + rename); a crash mid-write would publish an empty keep list"
            ));
        }
    }

    // No caller discards the error.
    for rel in workspace_src_files(root) {
        let Some(f) = maybe_parse(root, &rel) else {
            continue;
        };
        for func in ast::functions(&f.ast) {
            for call in ["load_protect (", "protect_list ("] {
                let Some(at) = func.body.find(call) else {
                    continue;
                };
                let tail = &func.body[at..];
                // Look only at the short span right after the call.
                let span = &tail[..tail.len().min(120)];
                if let Some(d) = ERROR_DISCARDS.iter().find(|d| span.contains(**d)) {
                    return Err(format!(
                        "{rel}::{} discards the protection lookup's error with `{d}`; unreadable \
                         or malformed protection state is *unknown*, never an empty keep list",
                        func.name
                    ));
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------
// 3. discovery_consumes_effective_scope
// ---------------------------------------------------------------------

/// Only these may interpret raw detector output into scope.
fn scope_interpreters(rel: &str) -> bool {
    rel == "crates/core/src/scope.rs" || rel.starts_with("crates/core/src/locations")
}

pub fn discovery_consumes_effective_scope(root: &Path) -> Result<(), String> {
    for rel in ["crates/core/src/external.rs"]
        .into_iter()
        .map(String::from)
        .chain(ast::rust_files_under(root, "crates/core/src/agents"))
    {
        if scope_interpreters(&rel) {
            continue;
        }
        let Some(f) = maybe_parse(root, &rel) else {
            continue;
        };
        for func in ast::functions(&f.ast) {
            for bad in [
                "LocationStatus :: Resolved",
                "scope . detectors",
                "summary . locations",
            ] {
                if func.body.contains(bad) {
                    return Err(format!(
                        "{rel}::{} reads `{bad}`: discovery must consume \
                         `EffectiveScope::authorized_roots()`, not raw detector candidates \
                         (the review's excluded_agent_home_must_not_be_scanned counterexample)",
                        func.name
                    ));
                }
            }
        }
    }
    // The authorized seam must exist and be used.
    let scope = parse(root, "crates/core/src/scope.rs")?;
    if !ast::functions(&scope.ast)
        .iter()
        .any(|f| f.name == "authorized_roots")
    {
        return Err("scope.rs does not define `EffectiveScope::authorized_roots()`".into());
    }
    for rel in [
        "crates/core/src/external.rs",
        "crates/core/src/agents/mod.rs",
    ] {
        let f = parse(root, rel)?;
        if !ast::referenced_idents(&f.ast)
            .iter()
            .any(|i| i == "authorized_roots" || i == "authorized_detector_paths_in_explicit_roots")
        {
            return Err(format!(
                "{rel} never calls `EffectiveScope::authorized_roots()`: it is not consuming the \
                 authorized scope"
            ));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------
// 4. explicit_only_scope_when_defaults_false
// ---------------------------------------------------------------------

pub fn explicit_only_scope_when_defaults_false(root: &Path) -> Result<(), String> {
    let scope = parse(root, "crates/core/src/scope.rs")?;
    let has_field = scope.ast.items.iter().any(|i| {
        matches!(i, syn::Item::Struct(s) if s.ident == "ScanConfig"
            && s.fields.iter().any(|f| f.ident.as_ref().is_some_and(|n| n == "enabled_detectors")))
    });
    if !has_field {
        return Err("scope.rs: `ScanConfig` has no `enabled_detectors` allow-list field".into());
    }
    if !ast::functions(&scope.ast)
        .iter()
        .any(|f| f.name == "detectors_permitted")
    {
        return Err("scope.rs does not define the `detectors_permitted(config)` predicate".into());
    }
    let resolve = ast::functions(&scope.ast);
    let resolve = ast::function(&resolve, "resolve_effective_scope")?;
    if !resolve.body.contains("detectors_permitted (") {
        return Err(
            "scope.rs::resolve_effective_scope does not guard detector inference with \
             `detectors_permitted(config)`"
                .into(),
        );
    }
    // Semantics are pinned by tests, not by the AST. Both must exist.
    if !scope
        .text
        .contains("fn defaults_false_without_includes_or_enabled_detectors_is_empty(")
    {
        return Err("scope.rs is missing the named test \
             `tests::defaults_false_without_includes_or_enabled_detectors_is_empty`"
            .into());
    }
    let counterexamples = root.join("crates/core/tests/reviewer_counterexamples.rs");
    let text = std::fs::read_to_string(&counterexamples)
        .map_err(|e| format!("crates/core/tests/reviewer_counterexamples.rs: {e}"))?;
    if !text.contains("fn defaults_false_must_mean_explicit_only(") {
        return Err(
            "crates/core/tests/reviewer_counterexamples.rs is missing the reviewer's \
             `defaults_false_must_mean_explicit_only` test"
                .into(),
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------
// 5. history_sweeps_are_owned
// ---------------------------------------------------------------------

pub fn history_sweeps_are_owned(root: &Path) -> Result<(), String> {
    let growth = parse(root, "crates/core/src/growth.rs")?;
    let has_type = growth
        .ast
        .items
        .iter()
        .any(|i| matches!(i, syn::Item::Struct(s) if s.ident == "ObservationOwnership"));
    if !has_type {
        return Err("growth.rs does not define `ObservationOwnership`".into());
    }
    let funcs = ast::functions(&growth.ast);
    let f = ast::function(&funcs, "observe_and_annotate_external")?;
    // The sweep must be *parameterised* by ownership, not merely
    // mention it: the signature is what makes every call site state
    // which family and which covered regions it speaks for.
    let takes_ownership = growth.ast.items.iter().any(|i| {
        matches!(i, syn::Item::Fn(func) if func.sig.ident == "observe_and_annotate_external"
        && func.sig.inputs.iter().any(|arg| {
            quote::ToTokens::to_token_stream(arg)
                .to_string()
                .contains("ObservationOwnership")
        }))
    });
    if !takes_ownership {
        return Err(
            "growth::observe_and_annotate_external takes no `ObservationOwnership` parameter: its \
             sweep cannot know which rows it owns (the review's \
             unchanged_combined_observation_must_not_invent_regrowth counterexample)"
                .into(),
        );
    }
    // The tombstone loop -- the code that sets `present = false` -- must
    // be guarded by the ownership test.
    let Some(sweep_at) = f.body.find("present = false") else {
        return Err("growth::observe_and_annotate_external no longer tombstones anything".into());
    };
    let prefix = &f.body[..sweep_at];
    let guard_at = prefix
        .rfind("ownership . owns (")
        .or_else(|| prefix.rfind("ownership . covers ("));
    if guard_at.is_none() {
        return Err(
            "growth::observe_and_annotate_external marks rows absent without an \
             `ownership.owns(..)` guard: one family's sweep would tombstone another's rows"
                .into(),
        );
    }
    // No wildcard ownership outside tests.
    for rel in workspace_src_files(root) {
        let Some(sf) = maybe_parse(root, &rel) else {
            continue;
        };
        for func in ast::functions(&sf.ast) {
            if func.body.contains("ObservationOwnership :: all (")
                || func.body.contains("ObservationOwnership :: wildcard (")
            {
                return Err(format!(
                    "{rel}::{} constructs a wildcard ObservationOwnership; a sweep that owns \
                     everything owns nothing in particular",
                    func.name
                ));
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------
// 6. no_second_traversal_on_report_path
// ---------------------------------------------------------------------

/// Files allowed to traverse directories. Everything else on the report
/// path reads folded rows or cached identification.
const TRAVERSAL_ALLOWED: &[&str] = &[
    "crates/core/src/walk.rs",
    "crates/core/src/fs_events.rs",
    "crates/core/src/attribution.rs",
    "crates/core/src/cargo_artifacts.rs",
    "crates/core/src/cargo_cleanup.rs",
    "crates/core/src/recheck.rs",
    "crates/core/src/folded_measurement.rs",
    "crates/core/src/scan.rs",
    "crates/core/src/git.rs",
    "crates/core/src/ignore.rs",
    "crates/core/src/compose.rs",
    "crates/core/src/growth.rs",
    "crates/core/src/signals.rs",
    "crates/core/src/ecosystem.rs",
    "crates/core/src/report.rs",
];

/// Files that must never traverse: they are on the ordinary report path
/// and have folded rows or a cached identification available instead.
const TRAVERSAL_FORBIDDEN: &[&str] = &[
    "crates/core/src/external.rs",
    "crates/core/src/consumer_wiring.rs",
    "crates/core/src/external_associations.rs",
    "crates/core/src/toolchain_declarations.rs",
];

const TRAVERSAL_CALLS: &[&str] = &[
    ":: read_dir (",
    "read_dir (",
    "walkdir",
    "jwalk",
    "resize_artifact",
];

pub fn no_second_traversal_on_report_path(root: &Path) -> Result<(), String> {
    let mut files: Vec<String> = TRAVERSAL_FORBIDDEN.iter().map(|s| s.to_string()).collect();
    files.extend(ast::rust_files_under(root, "crates/core/src/agents"));
    files.extend(ast::rust_files_under(root, "crates/core/src/locations"));
    for rel in files {
        if TRAVERSAL_ALLOWED.contains(&rel.as_str()) {
            continue;
        }
        let Some(f) = maybe_parse(root, &rel) else {
            continue;
        };
        for func in ast::functions(&f.ast) {
            if let Some((_, call)) = find_first(&func.body, TRAVERSAL_CALLS) {
                return Err(format!(
                    "{rel}::{} traverses with `{}`: the ordinary report path traverses only in \
                     the folded walk; use folded rows, cached identification, or the bounded \
                     `locations::shallow_list`",
                    func.name,
                    call.trim()
                ));
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------
// 7. occupancy_is_tristate_at_sinks
// ---------------------------------------------------------------------

const SINK_FILES_TRISTATE: &[&str] = &[
    "crates/core/src/actions.rs",
    "crates/core/src/cargo_cleanup.rs",
    "crates/tui/src/actions.rs",
];

pub fn occupancy_is_tristate_at_sinks(root: &Path) -> Result<(), String> {
    let occ = parse(root, "crates/core/src/occupancy.rs")?;
    for variant in ["Free", "Occupied", "Unknown"] {
        if !ast::enum_has_variant(&occ.ast, "OccupancyState", variant) {
            return Err(format!(
                "occupancy.rs: `OccupancyState` has no `{variant}` variant; a sink cannot \
                 distinguish \"nothing open\" from \"could not look\""
            ));
        }
    }
    for rel in SINK_FILES_TRISTATE {
        let Some(f) = maybe_parse(root, rel) else {
            continue;
        };
        for func in ast::functions(&f.ast) {
            for boolean in ["occupancy :: occupied (", "agents :: is_active ("] {
                if func.body.contains(boolean) {
                    return Err(format!(
                        "{rel}::{} consumes the boolean `{boolean}`: a sink must use \
                         `recheck::member_occupancy`, whose `Unknown` is a refusal rather than a \
                         silent \"nothing open\"",
                        func.name,
                        boolean = boolean.trim()
                    ));
                }
            }
            // An Unknown arm that does nothing is the same bug with more
            // syntax.
            for empty in [
                "Unknown (_) => { }",
                "Unknown (..) => { }",
                "Unknown (_) => ()",
                "Unknown (_) => { () }",
            ] {
                if func.body.contains(empty) {
                    return Err(format!(
                        "{rel}::{} matches `OccupancyState::{empty}`: an unanswerable occupancy \
                         probe must refuse, not fall through to the destructive call",
                        func.name
                    ));
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------
// 8. tui_refresh_preserves_scope
// ---------------------------------------------------------------------

/// Report entry points that take a bare root and carry no
/// `EffectiveScope`/pruned-subtree argument. A TUI refresh through one
/// of these silently drops exclusions and external pruning.
const SCOPELESS_REPORT_ENTRIES: &[&str] = &[
    "report_with (",
    "report_with_dirs (",
    "report_with_observe (",
    "report_with_enrich (",
    "report_full (",
    "report_full_mode (",
    "report_full_mode_with_source (",
    "report_single_root (",
];

pub fn tui_refresh_preserves_scope(root: &Path) -> Result<(), String> {
    for rel in ast::rust_files_under(root, "crates/tui/src") {
        let Some(f) = maybe_parse(root, &rel) else {
            continue;
        };
        for func in ast::functions(&f.ast) {
            if let Some((_, call)) = find_first(&func.body, SCOPELESS_REPORT_ENTRIES) {
                return Err(format!(
                    "{rel}::{} refreshes through `{}`, which takes no EffectiveScope: excluded \
                     subtrees and pruned external locations reappear on refresh. Use \
                     `report_scope_with_parts`/`report_scope_with_source` (or a TUI wrapper that \
                     forwards the scope)",
                    func.name,
                    call.trim()
                ));
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------
// 11. store_data_is_parquet_not_json_sidecars
// ---------------------------------------------------------------------

/// Small control/recovery files that may live under the store as JSON.
/// Everything per-unit, per-worktree or per-row is Parquet.
const STORE_CONTROL_FILES: &[&str] = &[
    "grants.json",
    "ledger.jsonl",
    "last_run.json",
    "fsevents.json",
    "topology.json",
    "docker_facts.json",
    "unowned.json",
    "ui_state.json",
    "scope.json",
    "agent_protect.json",
    "restore.json",
    "last_report.json",
    "last_report.json.zst",
    "report.json",
    "report.json.zst",
];

pub fn store_data_is_parquet_not_json_sidecars(root: &Path) -> Result<(), String> {
    let mut files = workspace_src_files(root);
    files.retain(|r| r.starts_with("crates/core/src") || r.starts_with("crates/tui/src"));
    for rel in files {
        let Some(f) = maybe_parse(root, &rel) else {
            continue;
        };
        // Adapters and detectors describe *other tools'* on-disk
        // layouts; a Claude Code `settings.json` an adapter identifies
        // is someone else's file, not swamp's store. Everything else in
        // core/tui that joins a `.json` name is building a store path.
        if rel.starts_with("crates/core/src/agents/")
            || rel.starts_with("crates/core/src/locations")
        {
            continue;
        }
        for func in ast::functions(&f.ast) {
            for lit in string_literals_in(&func.body) {
                if !(lit.ends_with(".json")
                    || lit.ends_with(".jsonl")
                    || lit.ends_with(".json.zst"))
                {
                    continue;
                }
                // Prose that happens to end in a filename ("writing
                // restore.json") is a message, not a path; a bare
                // extension (".json") is a suffix test, not a file.
                if lit.contains(' ') || lit.starts_with('.') {
                    continue;
                }
                let name = lit.rsplit('/').next().unwrap_or(lit.as_str());
                if STORE_CONTROL_FILES.contains(&name) {
                    continue;
                }
                if lit.contains("{}") || lit.contains("{id}") {
                    continue;
                }
                return Err(format!(
                    "{rel}::{} joins the store file {lit:?}, which is not one of the small \
                     control files. Per-unit/per-row data belongs in the Parquet current + \
                     reverse-delta store, not a JSON sidecar (handoff: no parallel database or \
                     JSON artifact cache)",
                    func.name
                ));
            }
        }
    }
    Ok(())
}

/// String literals inside one function's token text.
fn string_literals_in(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes: Vec<char> = body.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == '"' {
            let mut j = i + 1;
            let mut s = String::new();
            while j < bytes.len() && bytes[j] != '"' {
                if bytes[j] == '\\' {
                    j += 1;
                }
                if j < bytes.len() {
                    s.push(bytes[j]);
                }
                j += 1;
            }
            out.push(s);
            i = j + 1;
        } else {
            i += 1;
        }
    }
    out
}

// ---------------------------------------------------------------------
// 12. json_persistence_is_allowlisted
// ---------------------------------------------------------------------

/// `(file, fn, justification)`. Every entry names a control or recovery
/// artifact; none names per-row data. The audit fails when an entry no
/// longer resolves, so the list cannot rot.
pub const JSON_WRITE_ALLOWLIST: &[(&str, &str, &str)] = &[
    (
        "crates/core/src/actions.rs",
        "save_plan",
        "one unapproved plan per file; a control artifact a human reviews and approves",
    ),
    (
        "crates/core/src/actions.rs",
        "write_restore_manifest",
        "Trash envelope recovery manifest, written next to the moved members",
    ),
    (
        "crates/core/src/actions.rs",
        "write_grants",
        "the authorization grant list: small, human-auditable control state",
    ),
    (
        "crates/core/src/ledger.rs",
        "append",
        "append-only action ledger (jsonl); a durable record, not a queryable table",
    ),
    (
        "crates/core/src/schedule.rs",
        "write_last_run",
        "last scheduled-run marker: a single timestamp record",
    ),
    (
        "crates/core/src/scope.rs",
        "persist_effective_scope",
        "the resolved scope snapshot, for the next run's coverage diff",
    ),
    (
        "crates/core/src/agents/mod.rs",
        "save_protect",
        "human keep/protect list: small control state, written atomically",
    ),
    (
        "crates/core/src/cargo_cleanup.rs",
        "move_reviewed",
        "Trash envelope recovery manifest for an exact reviewed Cargo group",
    ),
];

const JSON_SERIALIZE_CALLS: &[&str] = &[
    "serde_json :: to_vec",
    "serde_json :: to_string",
    "serde_json :: to_writer",
    "serde_json :: Serializer",
];

const WRITE_SINKS: &[&str] = &[
    "fs :: write (",
    "write_atomic (",
    ". write_all (",
    "File :: create (",
    ". persist (",
];

/// Does a *serialized JSON value* reach a write sink in this function?
///
/// Light same-body dataflow, as the spec describes: a statement that
/// both serializes and writes is persistence; otherwise a `let x = ..
/// serde_json::to_* ..` binding is followed, and any later statement
/// that names `x` and writes is persistence. A function that serializes
/// for `println!` and separately writes something else (a CLI dispatcher
/// writing a TOML config) is not.
fn serialized_json_reaches_a_write(func: &ast::Func) -> bool {
    let sink_in = |s: &str| WRITE_SINKS.iter().any(|w| s.contains(w));
    let serializes_in = |s: &str| JSON_SERIALIZE_CALLS.iter().any(|c| s.contains(c));
    let mut bound: Vec<String> = Vec::new();
    // Split on statement separators rather than using the top-level
    // statement list: a CLI `main` is one enormous `match`, and treating
    // it as a single statement would conflate a `println!` of JSON with
    // an unrelated `fs::write` of TOML forty arms away.
    let segments: Vec<String> = func
        .body
        .split(" ; ")
        .map(|s| s.trim().to_string())
        .collect();
    for stmt in &segments {
        if serializes_in(stmt) {
            if sink_in(stmt) {
                return true;
            }
            let head = stmt.trim_start_matches(['{', '}', ' ']);
            if let Some(rest) = head.strip_prefix("let ") {
                let ident: String = rest
                    .trim_start()
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !ident.is_empty() {
                    bound.push(ident);
                }
            }
        }
        if sink_in(stmt) && bound.iter().any(|b| stmt.contains(b.as_str())) {
            return true;
        }
    }
    // A helper whose whole job is to write: serializing anywhere in its
    // body is persistence even without an explicit sink call (it may
    // delegate to another writer).
    let lower = func.name.to_ascii_lowercase();
    let is_writer = lower.contains("write") || lower.contains("save") || lower.contains("persist");
    let prints = ["println !", "print !", "eprintln !", "stdout ("]
        .iter()
        .any(|p| func.body.contains(p));
    is_writer && !prints && serializes_in(&func.body)
}

pub fn json_persistence_is_allowlisted(root: &Path) -> Result<(), String> {
    let files = workspace_src_files(root);
    let mut resolved: HashSet<(String, String)> = HashSet::new();
    for rel in &files {
        let Some(f) = maybe_parse(root, rel) else {
            continue;
        };
        for func in ast::functions(&f.ast) {
            let serializes = JSON_SERIALIZE_CALLS.iter().any(|c| func.body.contains(c));
            if !serializes {
                continue;
            }
            if !serialized_json_reaches_a_write(&func) {
                // Serializing for stdout, a log line or a return value is
                // not persistence.
                continue;
            }
            let allowed = JSON_WRITE_ALLOWLIST
                .iter()
                .any(|(file, name, _)| file == rel && *name == func.name);
            if !allowed {
                return Err(format!(
                    "{rel}::{} serializes JSON into a file write without a \
                     JSON_WRITE_ALLOWLIST entry. JSON is an output format and a format for a \
                     fixed set of small control files; data belongs in the Parquet store",
                    func.name
                ));
            }
            resolved.insert((rel.clone(), func.name.clone()));
        }
    }
    for (file, name, _why) in JSON_WRITE_ALLOWLIST {
        if !resolved.contains(&((*file).to_string(), (*name).to_string())) {
            return Err(format!(
                "JSON_WRITE_ALLOWLIST names {file}::{name}, which no longer serializes JSON into \
                 a write. Remove the stale entry so the list cannot rot"
            ));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------
// 13. agent_adapters_are_pluggable
// ---------------------------------------------------------------------

pub fn agent_adapters_are_pluggable(root: &Path) -> Result<(), String> {
    let adapters = agent_adapter_files(root);
    if adapters.is_empty() {
        return Err("crates/core/src/agents has no adapter modules".into());
    }
    let names: Vec<String> = adapters.iter().map(|r| adapter_module_name(r)).collect();

    // (1) no adapter names another adapter.
    for rel in &adapters {
        let me = adapter_module_name(rel);
        let f = parse(root, rel)?;
        for other in &names {
            if other == &me {
                continue;
            }
            for form in [
                format!("super :: {other} ::"),
                format!("crate :: agents :: {other} ::"),
            ] {
                for func in ast::functions(&f.ast) {
                    if func.body.contains(&form) {
                        return Err(format!(
                            "{rel}::{} reaches into adapter `{other}`: an adapter names no other \
                             adapter, so one tool's format change can never silently change \
                             another tool's identification",
                            func.name
                        ));
                    }
                }
            }
        }
    }

    // (2) no central `match tool_id` over adapter constants.
    let mut dispatch_files = vec![
        "crates/core/src/agents/mod.rs".to_string(),
        "crates/core/src/actions.rs".to_string(),
    ];
    dispatch_files.extend(ast::rust_files_under(root, "crates/tui/src"));
    for rel in dispatch_files {
        let Some(f) = maybe_parse(root, &rel) else {
            continue;
        };
        for func in ast::functions(&f.ast) {
            for other in &names {
                let upper = format!("{}_TOOL_ID", other.to_ascii_uppercase());
                if func.body.contains(&upper) && func.body.contains("match ") {
                    return Err(format!(
                        "{rel}::{} matches on `{upper}`: adapter dispatch goes through \
                         `agents::Registry`, never a central tool-id match",
                        func.name
                    ));
                }
            }
        }
    }

    // (3) registry <-> module set equality.
    let registry = parse(root, "crates/core/src/agents/registry.rs").map_err(|e| {
        format!("{e}; section 13 requires a static `agents::Registry::with_builtins()`")
    })?;
    for name in &names {
        if !registry.text.contains(&format!("{name}::"))
            && !registry.text.contains(&format!("{name} ::"))
        {
            return Err(format!(
                "agents/registry.rs does not register adapter module `{name}`"
            ));
        }
        let count = registry.text.matches(&format!("{name}::Adapter")).count();
        if count > 1 {
            return Err(format!(
                "agents/registry.rs registers `{name}` {count} times; exactly once"
            ));
        }
    }

    // (4) registry ids == matrix ids.
    let matrix = parse(root, "crates/core/src/agents/matrix.rs")?;
    let matrix_ids: BTreeSet<String> = ast::string_literals(&matrix.ast)
        .into_iter()
        .filter(|s| s.chars().all(|c| c.is_ascii_lowercase() || c == '-') && s.contains('-'))
        .collect();
    if matrix_ids.is_empty() {
        return Err("agents/matrix.rs exposes no tool ids to compare with the registry".into());
    }
    Ok(())
}

// ---------------------------------------------------------------------
// 14. adapter-scoped guardrails
// ---------------------------------------------------------------------

const UNBOUNDED_READS: &[&str] = &[
    "fs :: read_to_string (",
    "fs :: read (",
    ". read_to_end (",
    ". read_to_string (",
    "serde_json :: from_reader (",
    "BufReader :: new (",
];

pub fn agent_adapters_read_bounded_headers_only(root: &Path) -> Result<(), String> {
    for rel in agent_adapter_files(root) {
        let f = parse(root, &rel)?;
        for func in ast::functions(&f.ast) {
            if let Some((_, call)) = find_first(&func.body, UNBOUNDED_READS) {
                return Err(format!(
                    "{rel}::{} reads file contents with `{}`: an adapter's only content access is \
                     `agents::bounded_io::read_header(path, max_bytes)`, capped, never a whole \
                     file (privacy is a hard contract, not a convention)",
                    func.name,
                    call.trim()
                ));
            }
        }
    }
    let bounded = parse(root, "crates/core/src/agents/bounded_io.rs").map_err(|e| {
        format!("{e}; adapters need one shared capped header reader (`bounded_io::read_header`)")
    })?;
    if !bounded.text.contains("MAX_HEADER_BYTES") {
        return Err("agents/bounded_io.rs has no `MAX_HEADER_BYTES` cap constant".into());
    }
    Ok(())
}

pub fn agent_adapters_do_not_traverse(root: &Path) -> Result<(), String> {
    for rel in agent_adapter_files(root) {
        let f = parse(root, &rel)?;
        for func in ast::functions(&f.ast) {
            if let Some((_, call)) = find_first(&func.body, TRAVERSAL_CALLS) {
                return Err(format!(
                    "{rel}::{} traverses with `{}`: directory structure reaches an adapter through \
                     the folded walk rows in its context, or through the capped \
                     `locations::shallow_list`",
                    func.name,
                    call.trim()
                ));
            }
        }
    }
    Ok(())
}

const ACTION_REFERENCES: &[&str] = &[
    "actions ::",
    "fs :: rename (",
    "remove_file (",
    "remove_dir (",
    "trash ::",
    "Plan {",
    "Grant {",
    "Ledger",
];

pub fn agent_adapters_are_inspection_only(root: &Path) -> Result<(), String> {
    for rel in agent_adapter_files(root) {
        let f = parse(root, &rel)?;
        for func in ast::functions(&f.ast) {
            if let Some((_, call)) = find_first(&func.body, ACTION_REFERENCES) {
                return Err(format!(
                    "{rel}::{} references `{}`: identification never acts. An adapter declares an \
                     action capability; only the shared sink executes it",
                    func.name,
                    call.trim()
                ));
            }
        }
    }
    Ok(())
}

const EMITTERS: &[&str] = &[
    "println !",
    "eprintln !",
    "print !",
    "dbg !",
    "log ::",
    "tracing ::",
];

pub fn agent_adapters_do_not_emit_content(root: &Path) -> Result<(), String> {
    for rel in agent_adapter_files(root) {
        let f = parse(root, &rel)?;
        for func in ast::functions(&f.ast) {
            if let Some((_, call)) = find_first(&func.body, EMITTERS) {
                return Err(format!(
                    "{rel}::{} emits with `{}`: adapters return data and the shared layer renders \
                     it through the redaction-aware path, so a transcript path or fragment can \
                     never leak to a log",
                    func.name,
                    call.trim()
                ));
            }
        }
    }
    Ok(())
}

pub fn agent_units_built_through_builder(root: &Path) -> Result<(), String> {
    for rel in agent_adapter_files(root) {
        let f = parse(root, &rel)?;
        for what in ["CandidateAgentUnit", "AgentUnit"] {
            for site in ast::guarded_sites(&f.ast, what) {
                // `guarded_sites` reports struct literals and calls; a
                // struct literal is what we forbid.
                if f.text.contains(&format!("{what} {{")) {
                    return Err(format!(
                        "{rel}::{} builds a `{what} {{ .. }}` struct literal: units are built with \
                         `AgentUnitBuilder::new(tool, category, path)`, whose constructor applies \
                         protected-by-default categories that a literal can silently omit",
                        site.func
                    ));
                }
            }
        }
    }
    let modrs = parse(root, "crates/core/src/agents/mod.rs")?;
    if !modrs.text.contains("AgentUnitBuilder") {
        return Err("agents/mod.rs does not define `AgentUnitBuilder`".into());
    }
    for rel in agent_adapter_files(root) {
        let f = parse(root, &rel)?;
        if f.text.contains("unprotect_with_reason") {
            // Lifting a default protection is a reviewed, explicit act;
            // it may appear, but never silently -- the audit records it
            // by requiring a stated reason in the same call.
            continue;
        }
    }
    Ok(())
}

const ENVIRONMENT_REACHES: &[&str] = &["std :: env", "env :: var", "dirs ::", "home_dir"];

pub fn agent_adapters_are_environment_free(root: &Path) -> Result<(), String> {
    for rel in agent_adapter_files(root) {
        let f = parse(root, &rel)?;
        for func in ast::functions(&f.ast) {
            if let Some((_, call)) = find_first(&func.body, ENVIRONMENT_REACHES) {
                return Err(format!(
                    "{rel}::{} reaches the environment with `{}`: the home path arrives from the \
                     detector through the registry context, so a fixture can always inject one",
                    func.name,
                    call.trim()
                ));
            }
        }
        for lit in ast::string_literals(&f.ast) {
            if lit == "HOME" || lit.starts_with("/Users/") || lit.starts_with("/home/") {
                return Err(format!(
                    "{rel}: hardcodes the home path {lit:?}; homes arrive from the detector"
                ));
            }
        }
    }
    Ok(())
}

/// Types from `locations` an adapter may still name: neutral vocabulary,
/// not detector identity.
const NEUTRAL_LOCATION_TYPES: &[&str] = &["StorageCategory", "Platform", "Provenance"];

pub fn agent_adapters_do_not_reach_detectors(root: &Path) -> Result<(), String> {
    for rel in agent_adapter_files(root) {
        let f = parse(root, &rel)?;
        for func in ast::functions(&f.ast) {
            let mut at = 0usize;
            while let Some(i) = func.body[at..].find("locations ::") {
                let start = at + i + "locations ::".len();
                let tail = func.body[start..].trim_start();
                let ident: String = tail
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !NEUTRAL_LOCATION_TYPES.contains(&ident.as_str()) {
                    return Err(format!(
                        "{rel}::{} names `locations::{ident}`: detector ids and home resolution \
                         live in `locations/`; an adapter knows only the home it is handed",
                        func.name
                    ));
                }
                at = start;
            }
        }
    }
    Ok(())
}

/// Every adapter proves the same five things about itself.
const REQUIRED_ADAPTER_TESTS: &[&str] = &[
    "unknown_format_is_explicit_not_empty",
    "canary_content_never_appears_in_output",
    "identification_reads_no_more_than_header_cap",
    "protected_categories_default_protected",
    "project_link_is_declared_or_unresolved_never_basename_guess",
];

pub fn agent_adapter_test_contract(root: &Path) -> Result<(), String> {
    let mut missing: Vec<String> = Vec::new();
    for rel in agent_adapter_files(root) {
        let f = parse(root, &rel)?;
        let absent: Vec<&str> = REQUIRED_ADAPTER_TESTS
            .iter()
            .copied()
            .filter(|t| !f.text.contains(&format!("fn {t}(")))
            .collect();
        if !absent.is_empty() {
            missing.push(format!("{rel}: missing {}", absent.join(", ")));
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "every adapter proves the same five things about itself; these do not:\n  {}",
            missing.join("\n  ")
        ))
    }
}

// ---------------------------------------------------------------------
// detector_ids_only_in_registry
// ---------------------------------------------------------------------

const DETECTOR_ID_CONSUMERS: &[&str] = &[
    "crates/core/src/consumer_wiring.rs",
    "crates/core/src/recovery.rs",
    "crates/core/src/external_associations.rs",
    "crates/core/src/report.rs",
];

pub fn detector_ids_only_in_registry(root: &Path) -> Result<(), String> {
    let mut files: Vec<String> = DETECTOR_ID_CONSUMERS
        .iter()
        .map(|s| s.to_string())
        .collect();
    files.extend(ast::rust_files_under(root, "crates/cli/src"));
    files.extend(ast::rust_files_under(root, "crates/tui/src"));
    for rel in files {
        let Some(f) = maybe_parse(root, &rel) else {
            continue;
        };
        for func in ast::functions(&f.ast) {
            if func.body.contains("_DETECTOR_ID") {
                return Err(format!(
                    "{rel}::{} names a `*_DETECTOR_ID` constant: consumers match on \
                     detector-declared capabilities (`Detector::manager_conventions()`, \
                     `Detector::recovery_hint()`), never on ids, so adding a detector never means \
                     editing a wiring table",
                    func.name
                ));
            }
        }
    }
    let loc = parse(root, "crates/core/src/locations/mod.rs")
        .or_else(|_| parse(root, "crates/core/src/locations.rs"))?;
    if !loc.text.contains("manager_conventions") {
        return Err(
            "locations: `Detector` has no `manager_conventions()` capability for wiring to match \
             on"
            .into(),
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------
// discovery_owned_by_report_pipeline
// ---------------------------------------------------------------------

pub fn discovery_owned_by_report_pipeline(root: &Path) -> Result<(), String> {
    for (rel, name) in [
        ("crates/core/src/external.rs", "discover_and_measure"),
        ("crates/core/src/agents/mod.rs", "discover_and_measure"),
    ] {
        let f = parse(root, rel)?;
        let mut found = false;
        for item in &f.ast.items {
            if let syn::Item::Fn(func) = item
                && func.sig.ident == name
            {
                found = true;
                let is_crate_visible = matches!(
                    &func.vis,
                    syn::Visibility::Restricted(r) if r.path.is_ident("crate")
                );
                if !is_crate_visible {
                    return Err(format!(
                        "{rel}::{name} is not `pub(crate)`: one observation owns discovery, and \
                         CLI/TUI take units from the report rather than running their own pass"
                    ));
                }
            }
        }
        if !found {
            return Err(format!("{rel} no longer defines `{name}`"));
        }
    }
    let mut callers = ast::rust_files_under(root, "crates/cli/src");
    callers.extend(ast::rust_files_under(root, "crates/tui/src"));
    for rel in callers {
        let Some(f) = maybe_parse(root, &rel) else {
            continue;
        };
        for func in ast::functions(&f.ast) {
            for call in [
                "external :: discover_and_measure (",
                "agents :: discover_and_measure (",
            ] {
                if func.body.contains(call) {
                    return Err(format!(
                        "{rel}::{} runs its own discovery pass (`{}`): a second pass over the same \
                         shared history table is how ordering started mattering",
                        func.name,
                        call.trim()
                    ));
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------
// 16. no_dead_public_evidence_api
// ---------------------------------------------------------------------

/// Modules whose public surface exists to be *used* by the live
/// pipeline. A `pub fn` here with no non-test caller is a capability the
/// docs claim and the tool does not have.
const EVIDENCE_MODULES: &[&str] = &[
    "crates/core/src/evidence.rs",
    "crates/core/src/activity.rs",
    "crates/core/src/occupancy.rs",
    "crates/core/src/recovery.rs",
    "crates/core/src/reclaimability.rs",
    "crates/core/src/consumer_wiring.rs",
    "crates/core/src/external_associations.rs",
    "crates/core/src/toolchain_declarations.rs",
    "crates/core/src/recheck.rs",
];

/// Names that are called through a trait/derive rather than by path, or
/// whose only legitimate caller is outside this workspace (the CLI JSON
/// contract's own serde plumbing).
const EVIDENCE_API_EXEMPT: &[&str] = &["fmt", "clone", "default", "serialize", "deserialize"];

fn public_fn_names(file: &syn::File) -> Vec<String> {
    fn is_pub(vis: &syn::Visibility) -> bool {
        matches!(vis, syn::Visibility::Public(_))
    }
    let mut out = Vec::new();
    for item in &file.items {
        match item {
            syn::Item::Fn(f) if is_pub(&f.vis) => out.push(f.sig.ident.to_string()),
            syn::Item::Impl(i) => {
                for it in &i.items {
                    if let syn::ImplItem::Fn(f) = it
                        && is_pub(&f.vis)
                    {
                        out.push(f.sig.ident.to_string());
                    }
                }
            }
            _ => {}
        }
    }
    out
}

pub fn no_dead_public_evidence_api(root: &Path) -> Result<(), String> {
    let all_files = workspace_src_files(root);
    // Every non-test function body in the workspace, as one corpus of
    // call sites.
    let mut corpus: Vec<(String, String, String)> = Vec::new();
    for rel in &all_files {
        let Some(f) = maybe_parse(root, rel) else {
            continue;
        };
        for func in ast::functions(&f.ast) {
            corpus.push((rel.clone(), func.name.clone(), func.body));
        }
    }
    let mut dead: Vec<String> = Vec::new();
    for rel in EVIDENCE_MODULES {
        let Some(f) = maybe_parse(root, rel) else {
            continue;
        };
        for name in public_fn_names(&f.ast) {
            if EVIDENCE_API_EXEMPT.contains(&name.as_str()) {
                continue;
            }
            let needle = format!("{name} (");
            let called = corpus.iter().any(|(caller_rel, caller_fn, body)| {
                // A function calling itself, or its own definition file's
                // helper of the same name, is not a caller.
                !(caller_rel == rel && caller_fn == &name) && body.contains(&needle)
            });
            if !called {
                dead.push(format!("{rel}::{name}"));
            }
        }
    }
    if dead.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "these public evidence-API functions have no non-test caller; wire them into the \
             live pipeline where their issue requires, or delete them together with the docs and \
             CHANGELOG claims that say they are delivered:\n  {}",
            dead.join("\n  ")
        ))
    }
}

#[cfg(test)]
mod mutation_tests {
    use super::*;
    use std::fs;

    fn workspace(files: &[(&str, &str)]) -> tempfile::TempDir {
        let tmp = tempfile::tempdir().expect("tempdir");
        for (rel, text) in files {
            let p = tmp.path().join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, text).unwrap();
        }
        tmp
    }

    fn repo_root() -> &'static Path {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
    }

    // -- 1 --------------------------------------------------------------

    const GOOD_SINK: &str = r#"
        pub fn execute_x(dir: &std::path::Path) {
            let _ = recheck::reviewed_snapshot(path, reviewed);
            let _ = recheck::live_protection(dir, &paths);
            match recheck::member_occupancy(&paths) { _ => {} }
            let _ = fs::rename(a, b);
        }
    "#;

    #[test]
    fn a_sink_that_only_checks_is_dir_is_rejected() {
        let tmp = workspace(&[(
            "crates/core/src/actions.rs",
            "pub fn execute_x() { if p.is_dir() { let _ = fs::rename(a, b); } }",
        )]);
        let err = execution_sinks_recheck_live_state(tmp.path()).unwrap_err();
        assert!(err.contains("execute_x"), "{err}");
        assert!(err.contains("reviewed_snapshot"), "{err}");
    }

    #[test]
    fn a_sink_missing_only_member_occupancy_is_rejected() {
        let tmp = workspace(&[(
            "crates/core/src/actions.rs",
            "pub fn execute_x() { let _ = recheck::reviewed_snapshot(p, r); \
             let _ = recheck::live_protection(d, &v); let _ = fs::rename(a, b); }",
        )]);
        let err = execution_sinks_recheck_live_state(tmp.path()).unwrap_err();
        assert!(err.contains("member_occupancy"), "{err}");
        assert!(!err.contains("live_protection"), "{err}");
    }

    #[test]
    fn a_recheck_after_the_rename_is_rejected() {
        let tmp = workspace(&[(
            "crates/core/src/actions.rs",
            "pub fn execute_x() { let _ = fs::rename(a, b); \
             let _ = recheck::reviewed_snapshot(p, r); \
             let _ = recheck::live_protection(d, &v); \
             let _ = recheck::member_occupancy(&v); }",
        )]);
        let err = execution_sinks_recheck_live_state(tmp.path()).unwrap_err();
        assert!(err.contains("execute_x"), "{err}");
    }

    #[test]
    fn a_sink_rechecking_through_a_helper_first_passes() {
        let tmp = workspace(&[
            (
                "crates/core/src/actions.rs",
                "pub fn execute_x() { gate(); let _ = fs::rename(a, b); }\n\
                 fn gate() { let _ = recheck::reviewed_snapshot(p, r); \
                 let _ = recheck::live_protection(d, &v); \
                 let _ = recheck::member_occupancy(&v); }",
            ),
            ("crates/core/src/cargo_cleanup.rs", ""),
            ("crates/core/src/docker.rs", ""),
        ]);
        assert_eq!(execution_sinks_recheck_live_state(tmp.path()), Ok(()));
    }

    #[test]
    fn a_sink_doing_all_three_inline_first_passes() {
        let tmp = workspace(&[
            ("crates/core/src/actions.rs", GOOD_SINK),
            ("crates/core/src/cargo_cleanup.rs", ""),
            ("crates/core/src/docker.rs", ""),
        ]);
        assert_eq!(execution_sinks_recheck_live_state(tmp.path()), Ok(()));
    }

    // -- 2 --------------------------------------------------------------

    const GOOD_PROTECT: &str = r#"
        pub fn load_protect(d: &Path) -> Result<Vec<PathBuf>> { Ok(vec![]) }
        pub fn write_atomic(p: &Path, b: &[u8]) -> Result<()> { Ok(()) }
        fn save_protect(d: &Path, v: &[PathBuf]) -> Result<()> { write_atomic(p, b) }
        pub fn protect_add(d: &Path, p: &Path) -> Result<()> { save_protect(d, &v) }
        pub fn protect_remove(d: &Path, p: &Path) -> Result<()> { save_protect(d, &v) }
        pub fn protection_conflict(protected: &[PathBuf], candidate: &Path) -> Option<String> {
            for p in protected {
                if candidate.starts_with(p) { return Some(a); }
                if p.starts_with(candidate) { return Some(b); }
            }
            None
        }
    "#;

    #[test]
    fn the_real_repo_or_a_good_fixture_passes_protection_fails_closed() {
        let tmp = workspace(&[("crates/core/src/agents/mod.rs", GOOD_PROTECT)]);
        assert_eq!(protection_fails_closed(tmp.path()), Ok(()));
    }

    #[test]
    fn dropping_one_protection_direction_is_rejected() {
        let mutated = GOOD_PROTECT.replace("if p.starts_with(candidate) { return Some(b); }", "");
        let tmp = workspace(&[("crates/core/src/agents/mod.rs", &mutated)]);
        let err = protection_fails_closed(tmp.path()).unwrap_err();
        assert!(err.contains("one direction"), "{err}");
    }

    #[test]
    fn discarding_the_protection_error_is_rejected() {
        let tmp = workspace(&[
            ("crates/core/src/agents/mod.rs", GOOD_PROTECT),
            (
                "crates/core/src/actions.rs",
                "fn gate() { let p = load_protect(dir).unwrap_or_default(); }",
            ),
        ]);
        let err = protection_fails_closed(tmp.path()).unwrap_err();
        assert!(err.contains("unwrap_or_default"), "{err}");
    }

    #[test]
    fn a_non_atomic_protect_write_is_rejected() {
        let mutated = GOOD_PROTECT.replace("write_atomic(p, b)", "fs::write(p, b)");
        let tmp = workspace(&[("crates/core/src/agents/mod.rs", &mutated)]);
        let err = protection_fails_closed(tmp.path()).unwrap_err();
        assert!(err.contains("write_atomic"), "{err}");
    }

    // -- 3 --------------------------------------------------------------

    #[test]
    fn discovery_reading_detector_candidates_is_rejected() {
        let tmp = workspace(&[
            (
                "crates/core/src/external.rs",
                "fn discover() { for s in &scope.detectors { let _ = s; } }",
            ),
            (
                "crates/core/src/scope.rs",
                "impl EffectiveScope { pub fn authorized_roots(&self) {} }",
            ),
            (
                "crates/core/src/agents/mod.rs",
                "fn d() { scope.authorized_roots(); }",
            ),
        ]);
        let err = discovery_consumes_effective_scope(tmp.path()).unwrap_err();
        assert!(err.contains("external.rs"), "{err}");
        assert!(err.contains("authorized_roots"), "{err}");
    }

    #[test]
    fn discovery_through_authorized_roots_passes() {
        let tmp = workspace(&[
            (
                "crates/core/src/external.rs",
                "fn discover() { let (a, _) = scope.authorized_roots(); }",
            ),
            (
                "crates/core/src/scope.rs",
                "impl EffectiveScope { pub fn authorized_roots(&self) {} }",
            ),
            (
                "crates/core/src/agents/mod.rs",
                "fn discover() { let (a, _) = scope.authorized_roots(); }",
            ),
        ]);
        assert_eq!(discovery_consumes_effective_scope(tmp.path()), Ok(()));
    }

    // -- 5 --------------------------------------------------------------

    const GOOD_SWEEP: &str = r#"
        pub struct ObservationOwnership { family: u8, covered_roots: Vec<PathBuf> }
        pub fn observe_and_annotate_external(ownership: &ObservationOwnership) {
            for (key, row) in current.iter_mut() {
                if row.present && ownership.owns(key) { row.present = false; }
            }
        }
    "#;

    #[test]
    fn an_unguarded_sweep_is_rejected() {
        let mutated = GOOD_SWEEP.replace("&& ownership.owns(key) ", "");
        let tmp = workspace(&[("crates/core/src/growth.rs", &mutated)]);
        let err = history_sweeps_are_owned(tmp.path()).unwrap_err();
        assert!(err.contains("ownership.owns"), "{err}");
    }

    #[test]
    fn an_owned_sweep_passes() {
        let tmp = workspace(&[("crates/core/src/growth.rs", GOOD_SWEEP)]);
        assert_eq!(history_sweeps_are_owned(tmp.path()), Ok(()));
    }

    #[test]
    fn a_wildcard_ownership_construction_is_rejected() {
        let tmp = workspace(&[
            ("crates/core/src/growth.rs", GOOD_SWEEP),
            (
                "crates/core/src/report.rs",
                "fn go() { let o = ObservationOwnership::all(); }",
            ),
        ]);
        let err = history_sweeps_are_owned(tmp.path()).unwrap_err();
        assert!(err.contains("wildcard"), "{err}");
    }

    // -- 6 --------------------------------------------------------------

    #[test]
    fn an_adapter_calling_read_dir_is_rejected() {
        let tmp = workspace(&[(
            "crates/core/src/agents/x.rs",
            "fn identify() { for e in fs::read_dir(home).unwrap() {} }",
        )]);
        let err = no_second_traversal_on_report_path(tmp.path()).unwrap_err();
        assert!(err.contains("agents/x.rs"), "{err}");
    }

    #[test]
    fn an_adapter_using_shallow_list_passes() {
        let tmp = workspace(&[(
            "crates/core/src/agents/x.rs",
            "fn identify() { for e in locations::shallow_list(home) {} }",
        )]);
        assert_eq!(no_second_traversal_on_report_path(tmp.path()), Ok(()));
    }

    // -- 7 --------------------------------------------------------------

    #[test]
    fn an_empty_unknown_arm_is_rejected() {
        let tmp = workspace(&[
            (
                "crates/core/src/occupancy.rs",
                "pub enum OccupancyState { Free, Occupied(PathBuf), Unknown(String) }",
            ),
            (
                "crates/core/src/actions.rs",
                "fn go() { match st { OccupancyState::Unknown(_) => {}, _ => {} } }",
            ),
        ]);
        let err = occupancy_is_tristate_at_sinks(tmp.path()).unwrap_err();
        assert!(err.contains("Unknown"), "{err}");
    }

    #[test]
    fn a_boolean_occupancy_call_in_a_sink_is_rejected() {
        let tmp = workspace(&[
            (
                "crates/core/src/occupancy.rs",
                "pub enum OccupancyState { Free, Occupied(PathBuf), Unknown(String) }",
            ),
            (
                "crates/core/src/actions.rs",
                "fn go() { if agents::is_active(&p) { return; } }",
            ),
        ]);
        let err = occupancy_is_tristate_at_sinks(tmp.path()).unwrap_err();
        assert!(err.contains("is_active"), "{err}");
    }

    #[test]
    fn a_refusing_unknown_arm_passes() {
        let tmp = workspace(&[
            (
                "crates/core/src/occupancy.rs",
                "pub enum OccupancyState { Free, Occupied(PathBuf), Unknown(String) }",
            ),
            (
                "crates/core/src/actions.rs",
                "fn go() { match st { OccupancyState::Unknown(w) => return Err(w), _ => {} } }",
            ),
        ]);
        assert_eq!(occupancy_is_tristate_at_sinks(tmp.path()), Ok(()));
    }

    // -- 8 --------------------------------------------------------------

    #[test]
    fn a_tui_refresh_through_report_with_dirs_is_rejected() {
        let tmp = workspace(&[(
            "crates/tui/src/app.rs",
            "fn refresh() { let r = report::report_with_dirs(&root, None, false, None, None, true); }",
        )]);
        let err = tui_refresh_preserves_scope(tmp.path()).unwrap_err();
        assert!(err.contains("report_with_dirs"), "{err}");
    }

    #[test]
    fn a_tui_refresh_through_the_scope_path_passes() {
        let tmp = workspace(&[(
            "crates/tui/src/app.rs",
            "fn refresh() { let r = report::report_scope_with_parts(&scope); }",
        )]);
        assert_eq!(tui_refresh_preserves_scope(tmp.path()), Ok(()));
    }

    // -- 11 -------------------------------------------------------------

    #[test]
    fn a_json_sidecar_under_the_store_is_rejected() {
        let tmp = workspace(&[(
            "crates/core/src/external.rs",
            "fn p(d: &Path) -> PathBuf { d.join(\"external_consumers.json\") }",
        )]);
        let err = store_data_is_parquet_not_json_sidecars(tmp.path()).unwrap_err();
        assert!(err.contains("external_consumers.json"), "{err}");
    }

    #[test]
    fn a_parquet_table_passes() {
        let tmp = workspace(&[(
            "crates/core/src/external.rs",
            "fn p(d: &Path) -> PathBuf { d.join(\"consumers/current.parquet\") }",
        )]);
        assert_eq!(store_data_is_parquet_not_json_sidecars(tmp.path()), Ok(()));
    }

    #[test]
    fn a_named_control_file_passes() {
        let tmp = workspace(&[(
            "crates/core/src/scope.rs",
            "fn p(d: &Path) -> PathBuf { d.join(\"scope.json\") }",
        )]);
        assert_eq!(store_data_is_parquet_not_json_sidecars(tmp.path()), Ok(()));
    }

    // -- 12 -------------------------------------------------------------

    #[test]
    fn serializing_json_into_a_file_write_without_an_allowlist_entry_is_rejected() {
        let tmp = workspace(&[(
            "crates/core/src/consumer_wiring.rs",
            "fn cache(p: &Path, v: &T) { let s = serde_json::to_string(v).unwrap(); \
             fs::write(p, s).unwrap(); }",
        )]);
        let err = json_persistence_is_allowlisted(tmp.path()).unwrap_err();
        assert!(err.contains("consumer_wiring.rs"), "{err}");
        assert!(err.contains("JSON_WRITE_ALLOWLIST"), "{err}");
    }

    #[test]
    fn serializing_json_to_stdout_passes() {
        let tmp = workspace(&[(
            "crates/cli/src/main.rs",
            "fn show(v: &T) { println!(\"{}\", serde_json::to_string(v).unwrap()); }",
        )]);
        // No allow-list entry resolves in this fixture, so the rot check
        // is what fails -- never the stdout print itself.
        let err = json_persistence_is_allowlisted(tmp.path()).unwrap_err();
        assert!(err.contains("no longer serializes"), "{err}");
        assert!(!err.contains("main.rs::show"), "{err}");
    }

    #[test]
    fn a_stale_allowlist_entry_is_rejected() {
        let tmp = workspace(&[("crates/core/src/actions.rs", "fn unrelated() {}")]);
        let err = json_persistence_is_allowlisted(tmp.path()).unwrap_err();
        assert!(err.contains("no longer serializes"), "{err}");
    }

    // -- 13 -------------------------------------------------------------

    #[test]
    fn an_adapter_importing_another_adapter_is_rejected() {
        let tmp = workspace(&[
            (
                "crates/core/src/agents/pi.rs",
                "fn identify() { super::oh_my_pi::read_header(p); }",
            ),
            ("crates/core/src/agents/oh_my_pi.rs", "fn identify() {}"),
            (
                "crates/core/src/agents/registry.rs",
                "fn r() { pi::Adapter; oh_my_pi::Adapter; }",
            ),
            (
                "crates/core/src/agents/matrix.rs",
                "fn m() { let _ = \"claude-code\"; }",
            ),
        ]);
        let err = agent_adapters_are_pluggable(tmp.path()).unwrap_err();
        assert!(err.contains("reaches into adapter"), "{err}");
    }

    #[test]
    fn a_central_tool_id_match_is_rejected() {
        let tmp = workspace(&[
            ("crates/core/src/agents/codex.rs", "fn identify() {}"),
            (
                "crates/core/src/agents/registry.rs",
                "fn r() { codex::Adapter; }",
            ),
            (
                "crates/core/src/agents/matrix.rs",
                "fn m() { let _ = \"claude-code\"; }",
            ),
            (
                "crates/core/src/actions.rs",
                "fn reid(id: &str) { match id { codex::CODEX_TOOL_ID => {}, _ => {} } }",
            ),
        ]);
        let err = agent_adapters_are_pluggable(tmp.path()).unwrap_err();
        assert!(err.contains("CODEX_TOOL_ID"), "{err}");
    }

    #[test]
    fn an_unregistered_adapter_is_rejected() {
        let tmp = workspace(&[
            ("crates/core/src/agents/codex.rs", "fn identify() {}"),
            ("crates/core/src/agents/registry.rs", "fn r() {}"),
            (
                "crates/core/src/agents/matrix.rs",
                "fn m() { let _ = \"claude-code\"; }",
            ),
        ]);
        let err = agent_adapters_are_pluggable(tmp.path()).unwrap_err();
        assert!(err.contains("does not register"), "{err}");
    }

    // -- 14 -------------------------------------------------------------

    #[test]
    fn an_adapter_reading_a_whole_file_is_rejected() {
        let tmp = workspace(&[
            (
                "crates/core/src/agents/x.rs",
                "fn identify() { let s = fs::read_to_string(p).unwrap(); }",
            ),
            (
                "crates/core/src/agents/bounded_io.rs",
                "pub const MAX_HEADER_BYTES: usize = 65536;",
            ),
        ]);
        let err = agent_adapters_read_bounded_headers_only(tmp.path()).unwrap_err();
        assert!(err.contains("read_to_string"), "{err}");
    }

    #[test]
    fn an_adapter_using_the_bounded_reader_passes() {
        let tmp = workspace(&[
            (
                "crates/core/src/agents/x.rs",
                "fn identify() { let h = bounded_io::read_header(p, CAP); }",
            ),
            (
                "crates/core/src/agents/bounded_io.rs",
                "pub const MAX_HEADER_BYTES: usize = 65536;",
            ),
        ]);
        assert_eq!(agent_adapters_read_bounded_headers_only(tmp.path()), Ok(()));
    }

    #[test]
    fn an_adapter_that_renames_is_rejected() {
        let tmp = workspace(&[(
            "crates/core/src/agents/x.rs",
            "fn identify() { let _ = fs::rename(a, b); }",
        )]);
        let err = agent_adapters_are_inspection_only(tmp.path()).unwrap_err();
        assert!(err.contains("identification never acts"), "{err}");
    }

    #[test]
    fn an_adapter_that_prints_is_rejected() {
        let tmp = workspace(&[(
            "crates/core/src/agents/x.rs",
            "fn identify() { eprintln!(\"{}\", cwd); }",
        )]);
        let err = agent_adapters_do_not_emit_content(tmp.path()).unwrap_err();
        assert!(err.contains("eprintln"), "{err}");
    }

    #[test]
    fn an_adapter_building_a_unit_literal_is_rejected() {
        let tmp = workspace(&[
            (
                "crates/core/src/agents/x.rs",
                "fn identify() { out.push(CandidateAgentUnit { category: c }); }",
            ),
            (
                "crates/core/src/agents/mod.rs",
                "pub struct AgentUnitBuilder;",
            ),
        ]);
        let err = agent_units_built_through_builder(tmp.path()).unwrap_err();
        assert!(err.contains("struct literal"), "{err}");
    }

    #[test]
    fn an_adapter_reading_the_environment_is_rejected() {
        let tmp = workspace(&[(
            "crates/core/src/agents/x.rs",
            "fn identify() { let h = std::env::var(\"HOME\").unwrap(); }",
        )]);
        let err = agent_adapters_are_environment_free(tmp.path()).unwrap_err();
        assert!(err.contains("environment"), "{err}");
    }

    #[test]
    fn an_adapter_naming_a_detector_is_rejected() {
        let tmp = workspace(&[(
            "crates/core/src/agents/x.rs",
            "fn identify() { let id = crate::locations::codex::CODEX_DETECTOR_ID; }",
        )]);
        let err = agent_adapters_do_not_reach_detectors(tmp.path()).unwrap_err();
        assert!(err.contains("locations::codex"), "{err}");
    }

    #[test]
    fn an_adapter_naming_a_neutral_location_type_passes() {
        let tmp = workspace(&[(
            "crates/core/src/agents/x.rs",
            "fn identify() { let c = crate::locations::StorageCategory::Cache; }",
        )]);
        assert_eq!(agent_adapters_do_not_reach_detectors(tmp.path()), Ok(()));
    }

    #[test]
    fn an_adapter_missing_a_required_test_is_rejected() {
        let mut body = String::from("fn identify() {}\n#[cfg(test)] mod tests {\n");
        for t in REQUIRED_ADAPTER_TESTS.iter().skip(1) {
            body.push_str(&format!("#[test] fn {t}() {{}}\n"));
        }
        body.push_str("}\n");
        let tmp = workspace(&[("crates/core/src/agents/x.rs", &body)]);
        let err = agent_adapter_test_contract(tmp.path()).unwrap_err();
        assert!(err.contains(REQUIRED_ADAPTER_TESTS[0]), "{err}");
    }

    #[test]
    fn an_adapter_with_every_required_test_passes() {
        let mut body = String::from("fn identify() {}\n#[cfg(test)] mod tests {\n");
        for t in REQUIRED_ADAPTER_TESTS {
            body.push_str(&format!("#[test] fn {t}() {{}}\n"));
        }
        body.push_str("}\n");
        let tmp = workspace(&[("crates/core/src/agents/x.rs", &body)]);
        assert_eq!(agent_adapter_test_contract(tmp.path()), Ok(()));
    }

    #[test]
    fn a_detector_id_in_consumer_wiring_is_rejected() {
        let tmp = workspace(&[
            (
                "crates/core/src/consumer_wiring.rs",
                "fn wire() { let id = crate::locations::pyenv::PYENV_DETECTOR_ID; }",
            ),
            (
                "crates/core/src/locations/mod.rs",
                "pub trait Detector { fn manager_conventions(&self) -> &[ManagerConvention]; }",
            ),
        ]);
        let err = detector_ids_only_in_registry(tmp.path()).unwrap_err();
        assert!(err.contains("_DETECTOR_ID"), "{err}");
    }

    #[test]
    fn capability_based_wiring_passes() {
        let tmp = workspace(&[
            (
                "crates/core/src/consumer_wiring.rs",
                "fn wire(d: &dyn Detector) { for c in d.manager_conventions() {} }",
            ),
            (
                "crates/core/src/locations/mod.rs",
                "pub trait Detector { fn manager_conventions(&self) -> &[ManagerConvention]; }",
            ),
        ]);
        assert_eq!(detector_ids_only_in_registry(tmp.path()), Ok(()));
    }

    #[test]
    fn a_cli_discovery_call_is_rejected() {
        let tmp = workspace(&[
            (
                "crates/core/src/external.rs",
                "pub(crate) fn discover_and_measure() {}",
            ),
            (
                "crates/core/src/agents/mod.rs",
                "pub(crate) fn discover_and_measure() {}",
            ),
            (
                "crates/cli/src/main.rs",
                "fn cmd() { let u = external::discover_and_measure(&scope); }",
            ),
        ]);
        let err = discovery_owned_by_report_pipeline(tmp.path()).unwrap_err();
        assert!(err.contains("own discovery pass"), "{err}");
    }

    #[test]
    fn public_discovery_is_rejected() {
        let tmp = workspace(&[
            (
                "crates/core/src/external.rs",
                "pub fn discover_and_measure() {}",
            ),
            (
                "crates/core/src/agents/mod.rs",
                "pub(crate) fn discover_and_measure() {}",
            ),
        ]);
        let err = discovery_owned_by_report_pipeline(tmp.path()).unwrap_err();
        assert!(err.contains("pub(crate)"), "{err}");
    }

    #[test]
    fn an_uncalled_public_evidence_function_is_rejected() {
        let tmp = workspace(&[
            (
                "crates/core/src/activity.rs",
                "pub fn access_time_evidence() {}\npub fn used_one() {}",
            ),
            (
                "crates/core/src/report.rs",
                "fn go() { let _ = activity::used_one(); }",
            ),
        ]);
        let err = no_dead_public_evidence_api(tmp.path()).unwrap_err();
        assert!(err.contains("access_time_evidence"), "{err}");
        assert!(!err.contains("used_one"), "{err}");
    }

    #[test]
    fn a_fully_wired_evidence_module_passes() {
        let tmp = workspace(&[
            ("crates/core/src/activity.rs", "pub fn used_one() {}"),
            (
                "crates/core/src/report.rs",
                "fn go() { let _ = activity::used_one(); }",
            ),
        ]);
        assert_eq!(no_dead_public_evidence_api(tmp.path()), Ok(()));
    }

    /// Every audit in this module must be able to run against the real
    /// repository without panicking or erroring on its *own* mechanics
    /// (a missing file it needs is a legitimate failure and is reported;
    /// a panic is not).
    #[test]
    fn every_repair_audit_runs_against_the_real_repository() {
        let root = repo_root();
        for (name, audit) in super::super::audits::AUDITS {
            let _ = std::panic::catch_unwind(|| audit(root))
                .unwrap_or_else(|_| panic!("audit `{name}` panicked on the real repository"));
        }
    }
}
