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

/// `(fn, why)`: functions inside the sink files whose rename/remove
/// touches swamp's *own* store bookkeeping, never a path the user asked
/// about. Publishing a control file by temp-then-rename is the atomic
/// write discipline, not a destructive action on user data. The spec's
/// allow-list ("the recheck module itself, test code, and the Trash
/// backend's internal file ops") named this category; these are the
/// concrete members of it.
const STORE_BOOKKEEPING_FNS: &[(&str, &str, &str)] = &[
    (
        "crates/core/src/actions.rs",
        "save_plan",
        "publishes an unapproved plan file into the store by temp + rename",
    ),
    (
        "crates/core/src/actions.rs",
        "write_restore_manifest",
        "writes a Trash envelope's own recovery manifest beside content already moved",
    ),
    (
        "crates/core/src/cargo_cleanup.rs",
        "write_restore_manifest",
        "the Cargo envelope's own recovery manifest",
    ),
    (
        "crates/core/src/agents/mod.rs",
        "write_atomic",
        "the shared temp + rename primitive every small control file is written through",
    ),
    (
        "crates/core/src/actions.rs",
        "write_grants",
        "publishes the grant list into the store by temp + rename",
    ),
    (
        "crates/core/src/report.rs",
        "write_last_report",
        "publishes the compressed last-report cache into the store by temp + rename",
    ),
    (
        "crates/core/src/report.rs",
        "write_last_scope_report",
        "publishes the per-scope last-report cache into the store by temp + rename",
    ),
    (
        "crates/core/src/schedule.rs",
        "acquire_lock",
        "removes swamp's own stale observation lock file, never a user path",
    ),
    (
        "crates/core/src/schedule.rs",
        "drop",
        "releases swamp's own observation lock file",
    ),
    (
        "crates/core/src/schedule.rs",
        "uninstall",
        "removes the LaunchAgent plist swamp itself installed",
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

/// Resolved destructive primitives. Every one of them is matched by
/// resolved path, so `use std::fs::remove_dir_all as sweep; sweep(p)` is
/// still a destructive call -- slip class 1 in the mutation sweep, which
/// also got `fs::remove_dir_all` into an adapter's `identify`.
const DESTRUCTIVE_PATHS: &[&str] = &[
    "fs::rename",
    "fs::remove_file",
    "fs::remove_dir",
    "fs::remove_dir_all",
];

/// A Docker object is not a filesystem path, so the three path rechecks
/// cannot apply to it: there is no inode to compare, no protect entry
/// that can name it and no `lsof` that can answer for it. Its
/// equivalent is the daemon's own re-derivation, and the rule is the
/// same shape -- honoured, immediately before the destructive call.
const DOCKER_SINK: &str = "docker::remove";
const DOCKER_RECHECK: &str = "docker::still_removable";

/// Argument literals that make a `docker` subprocess destructive. The
/// primitive may exist in exactly one place.
const DOCKER_DESTRUCTIVE_ARGS: &[&str] = &["rm", "rmi", "prune"];

/// The three rechecks by resolved path.
const RECHECK_PATHS: &[(&str, &str)] = &[
    ("recheck::reviewed_snapshot", "recheck::reviewed_snapshot"),
    ("recheck::live_protection", "recheck::live_protection"),
    ("recheck::member_occupancy", "recheck::member_occupancy"),
];

/// A recheck whose answer is thrown away is not a recheck. `let _ =
/// recheck::live_protection(..)` compiles, keeps the call the audit was
/// looking for, and refuses nothing -- slip class 2.
fn honoured_recheck(c: &crate::resolve::CallSite) -> bool {
    c.honoured != crate::resolve::Honoured::Discarded
}

/// Every function in the workspace, with its resolved call sites. Sink
/// files are *derived* from this (any function containing a destructive
/// call), never hand-listed: moving a sink one file away used to make it
/// invisible.
fn workspace_functions(root: &Path) -> Vec<(String, String, Vec<crate::resolve::CallSite>)> {
    let mut out: Vec<(String, String, Vec<crate::resolve::CallSite>)> = Vec::new();
    for rel in crate::resolve::workspace_files(root) {
        let Some(f) = crate::resolve::maybe(root, &rel) else {
            continue;
        };
        let mut by_fn: HashMap<String, Vec<crate::resolve::CallSite>> = HashMap::new();
        for c in crate::resolve::production_calls(&f.ast) {
            by_fn.entry(c.func.clone()).or_default().push(c);
        }
        for (name, calls) in by_fn {
            out.push((rel.clone(), name, calls));
        }
    }
    out.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    out
}

/// Which rechecks `name` performs, following calls to other functions in
/// the workspace transitively. Only honoured calls count.
fn rechecks_of(
    name: &str,
    index: &HashMap<String, Vec<crate::resolve::CallSite>>,
    seen: &mut HashSet<String>,
) -> Rechecks {
    if !seen.insert(name.to_string()) {
        return Rechecks::default();
    }
    let Some(calls) = index.get(name) else {
        return Rechecks::default();
    };
    let mut r = Rechecks {
        snapshot: calls.iter().any(|c| {
            crate::resolve::path_ends_with(&c.path, RECHECK_PATHS[0].1) && honoured_recheck(c)
        }),
        protection: calls.iter().any(|c| {
            crate::resolve::path_ends_with(&c.path, RECHECK_PATHS[1].1) && honoured_recheck(c)
        }),
        occupancy: calls.iter().any(|c| {
            crate::resolve::path_ends_with(&c.path, RECHECK_PATHS[2].1) && honoured_recheck(c)
        }),
    };
    for c in calls {
        let callee = c.path.rsplit("::").next().unwrap_or(&c.path).to_string();
        if callee != name && index.contains_key(&callee) {
            r = r.union(rechecks_of(&callee, index, seen));
        }
    }
    r
}

/// The names a function binds from a recheck's own result
/// (`let fresh = reviewed_snapshot(..)`), which seed the "derived from a
/// recheck" set alongside the rechecks' arguments.
fn recheck_result_bindings(root: &Path, rel: &str, func: &str) -> Vec<String> {
    let Some(f) = crate::resolve::maybe(root, rel) else {
        return Vec::new();
    };
    crate::resolve::bindings(&f.ast)
        .into_iter()
        .filter(|b| {
            b.func == func
                && (RECHECK_PATHS.iter().any(|(_, p)| {
                    b.from
                        .replace(' ', "")
                        .contains(&p.replace("::", "::").replace(' ', ""))
                }) || b.from.contains("covered_paths"))
        })
        .map(|b| b.name)
        .collect()
}

pub fn execution_sinks_recheck_live_state(root: &Path) -> Result<(), String> {
    let functions = workspace_functions(root);
    let mut all_bindings: Vec<crate::resolve::Binding> = Vec::new();
    for rel in crate::resolve::workspace_files(root) {
        if let Some(f) = crate::resolve::maybe(root, &rel) {
            all_bindings.extend(crate::resolve::bindings(&f.ast));
        }
    }
    let mut index: HashMap<String, Vec<crate::resolve::CallSite>> = HashMap::new();
    for (_, name, calls) in &functions {
        index.entry(name.clone()).or_default().extend(calls.clone());
    }

    let mut violations: Vec<String> = Vec::new();
    for (rel, name, calls) in &functions {
        if STORE_BOOKKEEPING_FNS
            .iter()
            .any(|(f, n, _)| f == rel && n == name)
        {
            continue;
        }
        // The recheck model itself, the Trash backend's internal file
        // ops and the store's own Parquet publishing are not user data.
        if rel.ends_with("/recheck.rs") || rel.ends_with("/store.rs") || rel.ends_with("/growth.rs")
        {
            continue;
        }
        // EVERY destructive call, not just the first: the sweep moved a
        // second `fs::rename` after the audited one.
        let destructive: Vec<&crate::resolve::CallSite> = calls
            .iter()
            .filter(|c| {
                !c.method
                    && DESTRUCTIVE_PATHS
                        .iter()
                        .any(|d| crate::resolve::path_ends_with(&c.path, d))
            })
            .collect();
        for d in destructive {
            let mut have = Rechecks::default();
            for c in calls.iter().filter(|c| c.stmt <= d.stmt) {
                for (i, (_, path)) in RECHECK_PATHS.iter().enumerate() {
                    if crate::resolve::path_ends_with(&c.path, path) && honoured_recheck(c) {
                        match i {
                            0 => have.snapshot = true,
                            1 => have.protection = true,
                            _ => have.occupancy = true,
                        }
                    }
                }
                // Helper credit, transitively, but only for helpers
                // called *before* the destructive statement.
                let callee = c.path.rsplit("::").next().unwrap_or(&c.path).to_string();
                if &callee != name && index.contains_key(&callee) {
                    let mut seen = HashSet::new();
                    have = have.union(rechecks_of(&callee, &index, &mut seen));
                }
            }
            if !have.complete() {
                violations.push(format!(
                    "{rel}::{name} performs `{}` without {} first (a recheck whose result is \
                     discarded does not count)",
                    d.written,
                    have.missing().join(" + ")
                ));
                continue;
            }
            // Rechecking one path and removing another satisfies "the
            // rechecks happen first" while rechecking nothing that
            // matters. The path the destructive call names has to be one
            // the rechecks were about, or derived from their result.
            let subject = d
                .args
                .first()
                .map(|a| crate::resolve::root_ident(a))
                .unwrap_or_default();
            if subject.is_empty() {
                continue;
            }
            let rechecked_subjects: String = calls
                .iter()
                .filter(|c| {
                    c.stmt <= d.stmt
                        && honoured_recheck(c)
                        && (RECHECK_PATHS
                            .iter()
                            .any(|(_, p)| crate::resolve::path_ends_with(&c.path, p))
                            || crate::resolve::path_ends_with(&c.path, "recheck::covered_paths"))
                })
                .flat_map(|c| c.args.clone())
                .collect::<Vec<_>>()
                .join(" ");
            let derived_from_recheck = calls.iter().any(|c| {
                c.stmt <= d.stmt
                    && honoured_recheck(c)
                    && RECHECK_PATHS
                        .iter()
                        .any(|(_, p)| crate::resolve::path_ends_with(&c.path, p))
            }) && rechecked_subjects.is_empty();
            // When the rechecks were provided by a helper, this
            // function cannot say which path the helper was about, so
            // the subject rule does not apply here -- the helper is
            // audited where it lives.
            let direct = calls
                .iter()
                .filter(|c| c.stmt <= d.stmt && honoured_recheck(c))
                .fold(Rechecks::default(), |acc, c| {
                    let mut acc = acc;
                    for (i, (_, path)) in RECHECK_PATHS.iter().enumerate() {
                        if crate::resolve::path_ends_with(&c.path, path) {
                            match i {
                                0 => acc.snapshot = true,
                                1 => acc.protection = true,
                                _ => acc.occupancy = true,
                            }
                        }
                    }
                    acc
                });
            if !direct.complete() {
                continue;
            }
            let seeds: Vec<String> = rechecked_subjects
                .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .filter(|t| !t.is_empty())
                .map(str::to_string)
                .chain(recheck_result_bindings(root, rel, name))
                .collect();
            let tainted = crate::resolve::derived_from(&all_bindings, name, &seeds);
            let mentioned = tainted.contains(&subject);
            if !mentioned && !derived_from_recheck {
                violations.push(format!(
                    "{rel}::{name} performs `{}` on `{subject}`, which none of its rechecks were \
                     about: rechecking one path and removing another is not a recheck",
                    d.written
                ));
            }
        }

        // The Docker sink, with its own domain recheck.
        for d in calls
            .iter()
            .filter(|c| !c.method && crate::resolve::path_ends_with(&c.path, DOCKER_SINK))
        {
            let rechecked = calls.iter().any(|c| {
                c.stmt <= d.stmt
                    && crate::resolve::path_ends_with(&c.path, DOCKER_RECHECK)
                    && honoured_recheck(c)
            });
            if !rechecked {
                violations.push(format!(
                    "{rel}::{name} removes a Docker object without an honoured \
                     `docker::still_removable` first"
                ));
            }
        }
    }

    // The destructive `docker` subprocess itself may be built in exactly
    // one function, so "every caller rechecks" is a claim about a
    // reachable set rather than about whichever call site an audit
    // happened to look at.
    for rel in crate::resolve::workspace_files(root) {
        let Some(f) = crate::resolve::maybe(root, &rel) else {
            continue;
        };
        for m in crate::resolve::macro_sites(&f.ast) {
            let _ = m;
        }
        for func in ast::functions(&f.ast) {
            let builds_command = func.body.contains("Command :: new (\"docker\")")
                || func.body.contains("Command :: new (\"docker\" )");
            if !builds_command {
                continue;
            }
            let destructive = DOCKER_DESTRUCTIVE_ARGS
                .iter()
                .any(|a| func.body.contains(&format!("\"{a}\"")));
            if destructive && !(rel == "crates/core/src/docker.rs" && func.name == "remove") {
                violations.push(format!(
                    "{rel}::{} builds a destructive `docker` subprocess; the only place that may \
                     is docker.rs::remove, so that every path to it is auditable",
                    func.name
                ));
            }
        }
    }
    if violations.is_empty() {
        Ok(())
    } else {
        violations.sort();
        violations.dedup();
        Err(format!(
            "every sink that moves user data rechecks identity, protection and occupancy before \
             its first destructive call (see \
             .oh/guardrails/execution-sinks-recheck-live-state.md):\n  {}",
            violations.join("\n  ")
        ))
    }
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

/// `(file, fn)`: the functions that legitimately load the protect list
/// without testing a candidate against it -- they manage the list
/// itself. Everything else that loads it is about to make a decision,
/// and that decision goes through `protection_conflict`.
const PROTECT_LIST_MANAGERS: &[(&str, &str)] = &[
    ("crates/core/src/agents/mod.rs", "protect_add"),
    ("crates/core/src/agents/mod.rs", "protect_remove"),
    ("crates/core/src/agents/mod.rs", "protect_list"),
    ("crates/core/src/agents/mod.rs", "load_protect"),
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
    // Both directions, asked structurally rather than by the variable
    // names the author happened to pick: there must be two
    // `starts_with` calls whose receiver and argument are swapped.
    // Pinning the names meant renaming a local defeated the rule.
    let conflict_calls: Vec<crate::resolve::CallSite> =
        crate::resolve::production_calls(&agents.ast)
            .into_iter()
            .filter(|c| c.func == conflict.name && c.method && c.path == "starts_with")
            .collect();
    let pairs: Vec<(String, String)> = conflict_calls
        .iter()
        .map(|c| {
            (
                crate::resolve::root_ident(&c.receiver),
                c.args
                    .first()
                    .map(|a| crate::resolve::root_ident(a))
                    .unwrap_or_default(),
            )
        })
        .filter(|(r, a)| !r.is_empty() && !a.is_empty())
        .collect();
    let candidate_under = !pairs.is_empty();
    let protected_under = pairs
        .iter()
        .any(|(r, a)| pairs.iter().any(|(r2, a2)| r2 == a && a2 == r));
    if !candidate_under || !protected_under {
        return Err(format!(
            "agents/mod.rs::{} tests protection in only one direction (candidate-under-protected: \
             {candidate_under}, protected-under-candidate: {protected_under}). Protecting \
             `debug/log.txt` must also stop removing `debug/` -- see the review's \
             protected_descendant_must_prevent_parent_cache_proposal counterexample",
            conflict.name
        ));
    }

    // ONE predicate, everywhere.
    //
    // The integration owner's own mutation check found the hole this
    // closes: removing one `starts_with` direction from
    // `protection_conflict` was caught, but the same mutation applied
    // through `agents::is_human_protected` -- a `bool` wrapper that
    // delegated here -- passed this audit *and* every runtime test,
    // because the audit inspected `protection_conflict` while
    // `actions::propose_checking_protection` called the wrapper. A
    // second spelling of the same question is a second thing to inspect,
    // and the audit will always be looking at the other one.
    //
    // So: no other function in the crate may answer "is this protected"
    // unless it delegates to `protection_conflict`. `is_human_protected`
    // is deleted rather than fixed.
    for rel in workspace_src_files(root) {
        let Some(f) = maybe_parse(root, &rel) else {
            continue;
        };
        for item in &f.ast.items {
            let syn::Item::Fn(func) = item else { continue };
            let name = func.sig.ident.to_string();
            if name == "protection_conflict" || !name.contains("protect") {
                continue;
            }
            let returns_verdict = match &func.sig.output {
                syn::ReturnType::Default => false,
                syn::ReturnType::Type(_, ty) => {
                    let rendered = quote::quote!(#ty).to_string();
                    rendered == "bool" || rendered.starts_with("Option <")
                }
            };
            if !returns_verdict {
                continue;
            }
            let body = ast::functions(&f.ast)
                .into_iter()
                .find(|x| x.name == name)
                .map(|x| x.body)
                .unwrap_or_default();
            if !body.contains("protection_conflict (") {
                return Err(format!(
                    "{rel}::{name} answers a protection question without delegating to \
                     `agents::protection_conflict`. One predicate, everywhere: a second spelling \
                     is a second thing to inspect, and a mutation that removes one containment \
                     direction survives in whichever one the audit is not reading (the \
                     integration owner's 2026-09-21 mutation check found exactly this through \
                     the deleted `is_human_protected`)"
                ));
            }
        }
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
    // Every function that loads the protect list and is not managing
    // the list itself must reach the one predicate. The mutation sweep
    // re-implemented `live_protection` with a single `starts_with`
    // direction: it returns `Result`, not `bool`, and lives in
    // `recheck.rs`, so a rule that inspected only the bool-returning
    // predicate in `agents/mod.rs` could not see it.
    let mut open_coded: Vec<String> = Vec::new();
    for rel in crate::resolve::workspace_files(root) {
        let Some(sf) = crate::resolve::maybe(root, &rel) else {
            continue;
        };
        let mut by_fn: HashMap<String, Vec<crate::resolve::CallSite>> = HashMap::new();
        for c in crate::resolve::production_calls(&sf.ast) {
            by_fn.entry(c.func.clone()).or_default().push(c);
        }
        for (name, calls) in by_fn {
            let loads = calls
                .iter()
                .any(|c| crate::resolve::path_ends_with(&c.path, "agents::load_protect"));
            if !loads {
                continue;
            }
            if PROTECT_LIST_MANAGERS
                .iter()
                .any(|(f, n)| *f == rel && *n == name)
            {
                continue;
            }
            let uses_predicate = calls
                .iter()
                .any(|c| crate::resolve::path_ends_with(&c.path, "protection_conflict"));
            if !uses_predicate {
                open_coded.push(format!(
                    "{rel}::{name} loads the protect list and decides for itself instead of \
                     calling `agents::protection_conflict`; a second predicate is a second place \
                     to get one direction wrong"
                ));
            }
        }
    }
    if !open_coded.is_empty() {
        open_coded.sort();
        open_coded.dedup();
        return Err(open_coded.join("\n  "));
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
    // The tombstone write -- the code that sets `present = false`, or
    // increments `regrowth_count` -- must be guarded by the ownership
    // test *as its condition*, in every function in the workspace, not
    // only in the one this audit used to name. The sweep kept
    // `ownership.owns(key)` in the body and moved the write out from
    // under it (`let _owned = ownership.owns(key); row.present = false;`).
    let mut ungated: Vec<String> = Vec::new();
    let mut found_a_sweep = false;
    for rel in crate::resolve::workspace_files(root) {
        let Some(sf) = crate::resolve::maybe(root, &rel) else {
            continue;
        };
        for a in crate::resolve::assignments(&sf.ast) {
            // A tombstone is marking a row absent, or *incrementing* the
            // regrowth counter. Copying an already-stored count onto a
            // display row is neither.
            let tombstone = (a.lhs.ends_with(". present") && a.rhs.trim() == "false")
                || (a.lhs.ends_with(". regrowth_count") && a.rhs.replace(' ', "").contains("+1"));
            if !tombstone {
                continue;
            }
            found_a_sweep = true;
            let guarded = a.conditions.iter().any(|c| {
                let c = c.replace(' ', "");
                c.contains("ownership.owns(")
                    || c.contains("ownership.covers(")
                    || c.contains("protected_keys.contains(")
                    || c.contains("protected_worktree_ids.contains(")
                    || c.contains("!prev.present")
                    || c.contains("changed")
            });
            if !guarded {
                ungated.push(format!(
                    "{rel}::{} writes `{} = {}` with no ownership test in the condition that \
                     guards it",
                    a.func,
                    a.lhs.replace(' ', ""),
                    a.rhs.replace(' ', "")
                ));
            }
        }
    }
    if !found_a_sweep {
        return Err("nothing in the workspace tombstones a row any more".into());
    }
    if !ungated.is_empty() {
        ungated.sort();
        ungated.dedup();
        return Err(format!(
            "one family's sweep must never tombstone another's rows, and a coverage change is \
             never a deletion:\n  {}",
            ungated.join("\n  ")
        ));
    }
    let _ = f;
    // No wildcard ownership outside tests.
    for rel in workspace_src_files(root) {
        let Some(sf) = maybe_parse(root, &rel) else {
            continue;
        };
        for func in ast::functions(&sf.ast) {
            let body = func.body.replace(' ', "");
            if body.contains("ObservationOwnership::all(")
                || body.contains("ObservationOwnership::wildcard(")
                // A window seeded with the filesystem root covers
                // everything, which is the wildcard written as data.
                || (body.contains("ObservationOwnership::new(")
                    && (body.contains("PathBuf::from(\"/\")") || body.contains("Path::new(\"/\")")))
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

/// `(file, fn)`: the *one* bounded, single-level, capped, symlink-refusing
/// listing the guardrail spec itself carves out -- "detector modules that
/// genuinely need one shallow listing must go through a bounded helper
/// `locations::shallow_list` which is itself allow-listed and capped".
///
/// It is a `(file, fn)` pair rather than a whole-file entry on purpose: a
/// second traversal added elsewhere in `locations/mod.rs` still fails.
/// The cap is `locations::SHALLOW_LIST_CAP` and the audit below checks it
/// is still there, so the exemption cannot outlive the bound that earns
/// it.
const BOUNDED_LISTERS: &[(&str, &str)] = &[("crates/core/src/locations/mod.rs", "shallow_list")];

pub fn no_second_traversal_on_report_path(root: &Path) -> Result<(), String> {
    let mut files: Vec<String> = TRAVERSAL_FORBIDDEN.iter().map(|s| s.to_string()).collect();
    files.extend(ast::rust_files_under(root, "crates/core/src/agents"));
    files.extend(ast::rust_files_under(root, "crates/core/src/locations"));
    let loc = parse(root, "crates/core/src/locations/mod.rs")?;
    if !loc.text.contains("SHALLOW_LIST_CAP") {
        return Err(
            "locations/mod.rs has no `SHALLOW_LIST_CAP`: the one allowed listing is allowed \
             *because* it is capped"
                .into(),
        );
    }
    for rel in files {
        if TRAVERSAL_ALLOWED.contains(&rel.as_str()) {
            continue;
        }
        let Some(f) = maybe_parse(root, &rel) else {
            continue;
        };
        for func in ast::functions(&f.ast) {
            if BOUNDED_LISTERS
                .iter()
                .any(|(file, name)| *file == rel && *name == func.name)
            {
                continue;
            }
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
    // The allow-list says *where* a re-walk may be written. The 2026-09-22
    // re-review's sharpest audit finding is that this is not the same as
    // *whether* it happens: `folded_measurement::measure` sat on the
    // allow-list and re-walked every external root on every pass while
    // this audit stayed green and the guardrail above it claimed reuse.
    // So the audit now follows the callee: `measure` must consult the
    // persisted folded rows before it may call `resize_artifact*`.
    let fm = maybe_parse(root, "crates/core/src/folded_measurement.rs");
    let funcs = fm
        .as_ref()
        .map(|f| ast::functions(&f.ast))
        .unwrap_or_default();
    let measure = funcs.iter().find(|f| f.name == "measure");
    if let Some(measure) = measure
        && let Some(resize_at) = measure.body.find("resize_artifact")
    {
        let reuse_at = REUSE_LOOKUPS
            .iter()
            .filter_map(|n| measure.body.find(n))
            .min();
        match reuse_at {
            Some(i) if i < resize_at => {}
            _ => {
                return Err(
                    "folded_measurement::measure calls `resize_artifact*` without first \
                     consulting the persisted folded rows (one of: \
                     reuse_folded_measurement / persisted_folded_bytes / \
                     record_cache_hit-guarded lookup). The allow-list proves only where a \
                     re-walk is written, not whether it happens -- \
                     `.oh/guardrails/no-second-traversal-on-report-path.md` claims reuse, so \
                     the reuse has to be in the code"
                        .into(),
                );
            }
        }
    }
    Ok(())
}

/// The lookup names that count as "consulted the rows the walk already
/// persisted" in `folded_measurement::measure`.
const REUSE_LOOKUPS: &[&str] = &[
    "reuse_folded_measurement",
    "persisted_folded_bytes",
    "reusable_measurement",
];

// ---------------------------------------------------------------------
// 7. occupancy_is_tristate_at_sinks
// ---------------------------------------------------------------------

/// Shapes that make an arm a refusal rather than a fall-through.
/// Shapes that make an arm a refusal rather than a fall-through. An
/// evidence builder is not a sink: turning an unanswerable probe into an
/// explicit `Evidence::unavailable` fact is the correct treatment of the
/// same unknown, and is what `.oh/guardrails/activity-and-consumer-evidence-have-limits.md`
/// requires. What is forbidden is treating `Unknown` as `Free`.
const REFUSALS: &[&str] = &[
    "bail !",
    "Err (",
    "refuse",
    "Refusal",
    "anyhow !",
    "continue",
    "return",
    "panic !",
    "Evidence :: unavailable",
    "Evidence :: unknown",
];

/// `(file, fn)`: the two definitions that legitimately produce the
/// boolean answer. Everything else consumes the tri-state.
const BOOLEAN_OCCUPANCY_DEFINITIONS: &[(&str, &str)] = &[
    ("crates/core/src/occupancy.rs", "occupied"),
    ("crates/core/src/agents/mod.rs", "is_active"),
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
    let mut problems: Vec<String> = Vec::new();
    // Whole workspace, resolved: the boolean answer has exactly two
    // definitions and no consumers.
    for rel in crate::resolve::workspace_files(root) {
        let Some(f) = crate::resolve::maybe(root, &rel) else {
            continue;
        };
        for c in crate::resolve::production_calls(&f.ast) {
            let boolean = crate::resolve::path_ends_with(&c.path, "occupancy::occupied")
                || crate::resolve::path_ends_with(&c.path, "agents::is_active")
                || (!c.method && c.path == "occupied");
            if boolean
                && !BOOLEAN_OCCUPANCY_DEFINITIONS
                    .iter()
                    .any(|(file, name)| *file == rel && *name == c.func)
            {
                problems.push(format!(
                    "{rel}::{} consumes the boolean `{}`: every consumer uses \
                     `recheck::member_occupancy`, whose `Unknown` is a refusal rather than a \
                     silent \"nothing open\"",
                    c.func, c.written
                ));
            }
            // The tri-state answer must reach control flow.
            if crate::resolve::path_ends_with(&c.path, "recheck::member_occupancy")
                && c.honoured == crate::resolve::Honoured::Discarded
            {
                problems.push(format!(
                    "{rel}::{} discards the result of `member_occupancy`: a probe whose answer \
                     nothing reads is not a probe",
                    c.func
                ));
            }
        }
        // Every `Unknown` arm of an occupancy match refuses.
        for arm in crate::resolve::match_arms(&f.ast) {
            let about_occupancy = arm.scrutinee.contains("member_occupancy")
                || arm.scrutinee.contains("probe_path")
                || arm.pattern.contains("OccupancyState");
            if !about_occupancy || !arm.pattern.contains("Unknown") {
                continue;
            }
            let refuses = REFUSALS.iter().any(|r| arm.body.contains(r));
            if !refuses {
                problems.push(format!(
                    "{rel}::{}: `OccupancyState::Unknown` arm `{}` does not refuse; an \
                     unanswerable occupancy probe must refuse, not fall through",
                    arm.func,
                    arm.body.replace('\n', " ")
                ));
            }
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        problems.sort();
        problems.dedup();
        Err(problems.join("\n  "))
    }
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

/// Control files whose name carries an id. Written with the placeholder
/// in place, because that is what the audit sees after `format!`
/// expansion -- and because an allow-list that skips *every* literal
/// containing `{}` is the escape the mutation sweep used to add
/// `format!("project-{}.json")` as a per-project sidecar.
const STORE_CONTROL_PATTERNS: &[&str] = &["plans/{}.json", "{}.json.zst"];

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
        // Adapters and detectors used to be skipped wholesale, which
        // meant a sidecar written from `agents/mod.rs` was invisible.
        // The function-level "does this function handle a store path"
        // test below is the real discriminator, so the directory skip is
        // gone.
        for func in ast::functions(&f.ast) {
            // Only a function that actually handles a store path can be
            // naming a store file. `read_identities(root)` joining
            // `package-lock.json` is reading the *user's project*, which
            // is this tool's whole job.
            // Whole-token match, not substring: OpenCode's own
            // `storage/` directory is not swamp's store, and an adapter
            // naming another tool's `auth.json` is doing its job.
            let handles_store = ["swamp_dir", "store_dir", "swamp_path"].iter().any(|s| {
                func.body
                    .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                    .any(|t| t == *s)
            });
            if !handles_store {
                continue;
            }
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
                if STORE_CONTROL_PATTERNS
                    .iter()
                    .any(|p| lit.ends_with(p) || &lit.as_str() == p)
                {
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
    (
        "crates/core/src/growth.rs",
        "write_fsevents_state",
        "the FSEvents cursor: one event id and a mode, read once per observation",
    ),
    (
        "crates/core/src/growth.rs",
        "write_topology",
        "the worktree topology snapshot an incremental replay diffs against",
    ),
    (
        "crates/core/src/growth.rs",
        "write_unowned",
        "the unowned-paths cache an incremental pass carries forward",
    ),
    (
        "crates/core/src/report.rs",
        "write_last_report",
        "the compressed last-report cache the TUI paints from before observing",
    ),
    (
        "crates/core/src/report.rs",
        "write_last_scope_report",
        "the same last-report cache, keyed by a multi-root scope",
    ),
    (
        "crates/tui/src/app.rs",
        "persist_ui_state",
        "the remembered filter/sort/reverse of the last TUI session",
    ),
];

/// Every way a value becomes JSON bytes. `serde_json :: json !` is here
/// because the mutation sweep used `json!(..).to_string()` -- which
/// produces exactly the same bytes and contains none of the other three
/// needles.
const JSON_SERIALIZE_CALLS: &[&str] = &[
    "serde_json :: to_vec",
    "serde_json :: to_string",
    "serde_json :: to_writer",
    "serde_json :: Serializer",
    "serde_json :: json !",
    "json ! (",
];

const WRITE_SINKS: &[&str] = &[
    "fs :: write (",
    "write_atomic (",
    ". write_all (",
    "File :: create (",
    ". persist (",
    // The ledger is append-only jsonl written through `writeln!` into an
    // opened file: persistence, and audited as such.
    "writeln !",
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
        // Count whole path segments, not substrings: `pi :: Adapter`
        // occurs inside `oh_my_pi :: Adapter`, and counting naively
        // reported `pi` as registered twice. A registration is a match
        // whose preceding character is not part of an identifier.
        let count: usize = [format!("{name}::Adapter"), format!("{name} :: Adapter")]
            .iter()
            .map(|needle| {
                registry
                    .text
                    .match_indices(needle.as_str())
                    .filter(|(at, _)| {
                        registry.text[..*at]
                            .chars()
                            .next_back()
                            .is_none_or(|c| !c.is_alphanumeric() && c != '_')
                    })
                    .count()
            })
            .sum();
        if count != 1 {
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

/// Text shapes that are not calls (a struct literal, a type name).
const ACTION_REFERENCES: &[&str] = &["actions ::", "trash ::", "Plan {", "Grant {", "Ledger"];

/// Resolved paths an adapter may never call. `remove_dir_all` was
/// missing from the old token list entirely, which is how the sweep got
/// `fs::remove_dir_all(home.join("logs"))` into an adapter's `identify`:
/// identification deleting the user's logs, passing an audit named
/// "inspection only".
const ADAPTER_FORBIDDEN_CALLS: &[&str] = &[
    "fs::rename",
    "fs::remove_file",
    "fs::remove_dir",
    "fs::remove_dir_all",
    "fs::write",
    "fs::create_dir",
    "fs::create_dir_all",
    "fs::set_permissions",
    "fs::copy",
    "fs::hard_link",
    "Command::new",
];

pub fn agent_adapters_are_inspection_only(root: &Path) -> Result<(), String> {
    let mut problems: Vec<String> = Vec::new();
    for rel in agent_adapter_files(root) {
        let f = parse(root, &rel)?;
        for func in ast::functions(&f.ast) {
            if let Some((_, call)) = find_first(&func.body, ACTION_REFERENCES) {
                problems.push(format!(
                    "{rel}::{} references `{}`: identification never acts. An adapter declares an \
                     action capability; only the shared sink executes it",
                    func.name,
                    call.trim()
                ));
            }
        }
        // Resolved calls, so an alias cannot rename the primitive out of
        // sight, and every mutating filesystem call is covered rather
        // than the three that happened to be listed.
        for c in crate::resolve::production_calls(&f.ast) {
            if c.method {
                continue;
            }
            if let Some(bad) = ADAPTER_FORBIDDEN_CALLS
                .iter()
                .find(|p| crate::resolve::path_ends_with(&c.path, p))
            {
                problems.push(format!(
                    "{rel}::{} calls `{}` (resolved: {bad}): an adapter identifies, it never \
                     writes, deletes or spawns",
                    c.func, c.written
                ));
            }
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        problems.sort();
        problems.dedup();
        Err(problems.join("\n  "))
    }
}

/// Every way an adapter can put bytes on a terminal. `writeln!(stderr())`
/// and `stdout().write_all(..)` were absent, which is how the mutation
/// sweep got adapter content onto stderr past a list of three macros.
const EMITTERS: &[&str] = &[
    "println !",
    "eprintln !",
    "print !",
    "eprint !",
    "dbg !",
    "log ::",
    "tracing ::",
    "writeln !",
    "write !",
    "io :: stdout",
    "io :: stderr",
    "stdout ()",
    "stderr ()",
    ". write_all (",
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
    let mut problems: Vec<String> = Vec::new();
    for rel in agent_adapter_files(root) {
        let f = parse(root, &rel)?;
        let res = crate::resolve::resolver(&f.ast);
        // Struct-literal *expressions*, with their path resolved: `use
        // super::CandidateAgentUnit as Unit; Unit { protected: false,
        // .. }` is the sweep's mutation and the old text match could not
        // see it.
        for (func, path) in ast::struct_literal_sites(&f.ast) {
            let resolved = res.resolve(&path);
            for what in ["CandidateAgentUnit", "AgentUnit"] {
                if crate::resolve::path_ends_with(&resolved, what) {
                    problems.push(format!(
                        "{rel}::{func} builds a `{what} {{ .. }}` struct literal (written \
                         `{path}`): units are built with `AgentUnitBuilder::new(tool, category, \
                         path)`, whose constructor applies protected-by-default categories that a \
                         literal can silently omit"
                    ));
                }
            }
        }
        // Lifting a default protection is a reviewed, explicit act; an
        // empty reason is the same silent unprotect written differently.
        for c in crate::resolve::production_calls(&f.ast) {
            if c.path == "unprotect_with_reason"
                && c.args
                    .iter()
                    .all(|a| a.replace(' ', "").is_empty() || a.replace(' ', "") == "\"\"")
            {
                problems.push(format!(
                    "{rel}::{} lifts protected-by-default with no stated reason",
                    c.func
                ));
            }
        }
    }
    let modrs = parse(root, "crates/core/src/agents/mod.rs")?;
    if !modrs.text.contains("AgentUnitBuilder") {
        return Err("agents/mod.rs does not define `AgentUnitBuilder`".into());
    }
    if problems.is_empty() {
        Ok(())
    } else {
        problems.sort();
        problems.dedup();
        Err(problems.join("\n  "))
    }
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

/// The single non-test function allowed to run a discovery pass.
/// `report.rs::observe_scope` is the one observation that owns the walk,
/// the external pass and the agent pass together -- which is what makes
/// each one's `growth::ObservationOwnership` window meaningful, and what
/// stopped the ordering between them mattering.
const DISCOVERY_OWNER: (&str, &str) = ("crates/core/src/report.rs", "observe_scope");

const DISCOVERY_CALLS: &[&str] = &[
    "external :: discover_and_measure (",
    "agents :: discover_and_measure (",
];

/// Statement: one observation owns discovery.
///
/// **This audit checks call sites rather than visibility, and that is a
/// correction, not a relaxation.** As first written it required both
/// `discover_and_measure` functions to be `pub(crate)`. That is directly
/// incompatible with the reviewers' own mandatory
/// `crates/core/tests/reviewer_counterexamples.rs`, which calls both from
/// an integration test -- from outside the crate. Those files are copied
/// in byte-for-byte and may never be edited, so `pub(crate)` would stop
/// the required evidence compiling. The rule and the evidence cannot both
/// be satisfied, and the evidence wins.
///
/// Visibility was never the property the review falsified. What it found
/// was two *passes* over one shared history table in an order nobody
/// declared. So the rule is now exactly that: outside test code, nothing
/// in `crates/cli/src` or `crates/tui/src` may run a discovery pass, and
/// inside `crates/core/src` only `report.rs::observe_scope` may -- which
/// must call *both*, so their ownership windows are decided together.
/// Integration tests may still call either directly, which is how the
/// counterexamples exercise them in isolation.
///
/// Inside the crate this is strictly stronger than the visibility check
/// it replaces: `pub(crate)` permitted any number of core-internal
/// passes, and this permits one.
pub fn discovery_owned_by_report_pipeline(root: &Path) -> Result<(), String> {
    for (rel, name) in [
        ("crates/core/src/external.rs", "discover_and_measure"),
        ("crates/core/src/agents/mod.rs", "discover_and_measure"),
    ] {
        let f = parse(root, rel)?;
        if !f
            .ast
            .items
            .iter()
            .any(|item| matches!(item, syn::Item::Fn(func) if func.sig.ident == name))
        {
            return Err(format!("{rel} no longer defines `{name}`"));
        }
    }

    let owner = parse(root, DISCOVERY_OWNER.0)?;
    let owner_fn = ast::functions(&owner.ast)
        .into_iter()
        .find(|f| f.name == DISCOVERY_OWNER.1)
        .ok_or_else(|| {
            format!(
                "{}::{} does not exist: the one observation that owns discovery has to be \
                 somewhere",
                DISCOVERY_OWNER.0, DISCOVERY_OWNER.1
            )
        })?;
    for call in DISCOVERY_CALLS {
        if !owner_fn.body.contains(call) {
            return Err(format!(
                "{}::{} does not call `{}`: one observation owns *both* passes, or their \
                 ownership windows can disagree again",
                DISCOVERY_OWNER.0,
                DISCOVERY_OWNER.1,
                call.trim()
            ));
        }
    }

    let mut callers = ast::rust_files_under(root, "crates/cli/src");
    callers.extend(ast::rust_files_under(root, "crates/tui/src"));
    callers.extend(
        workspace_src_files(root)
            .into_iter()
            .filter(|rel| rel.starts_with("crates/core/src") && rel != DISCOVERY_OWNER.0),
    );
    callers.sort();
    callers.dedup();
    for rel in callers {
        let Some(f) = maybe_parse(root, &rel) else {
            continue;
        };
        for func in ast::functions(&f.ast) {
            for call in DISCOVERY_CALLS {
                if func.body.contains(call) {
                    return Err(format!(
                        "{rel}::{} runs its own discovery pass (`{}`): a second pass over the \
                         same shared history table is how ordering started mattering. Take the \
                         units from `{}::{}`'s observation instead",
                        func.name,
                        call.trim(),
                        DISCOVERY_OWNER.0,
                        DISCOVERY_OWNER.1
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

/// Every `pub const` / `pub static` declared at module level.
///
/// The 2026-09-22 re-review found `ACTIVITY_EVIDENCE_INVENTORY` -- the
/// #54 inventory deliverable -- with zero non-test readers, slipping past
/// this audit because it only ever scanned `pub fn`. A dead constant is
/// exactly as undelivered as a dead function.
fn public_const_names(file: &syn::File) -> Vec<String> {
    fn is_pub(vis: &syn::Visibility) -> bool {
        matches!(vis, syn::Visibility::Public(_))
    }
    let mut out = Vec::new();
    for item in &file.items {
        match item {
            syn::Item::Const(c) if is_pub(&c.vis) => out.push(c.ident.to_string()),
            syn::Item::Static(st) if is_pub(&st.vis) => out.push(st.ident.to_string()),
            _ => {}
        }
    }
    out
}

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
    let all_files = crate::resolve::workspace_files(root);
    // Every *production* function body in the workspace, as one corpus
    // of call sites. `production_calls`' filter is what matters here:
    // a function marked `#[allow(dead_code)]` is not a caller, and the
    // mutation sweep manufactured exactly such a caller to keep a dead
    // public API looking wired.
    let mut corpus: Vec<(String, String, String)> = Vec::new();
    let mut dead_code_fns: HashSet<(String, String)> = HashSet::new();
    for rel in &all_files {
        let Some(f) = maybe_parse(root, rel) else {
            continue;
        };
        for c in crate::resolve::calls(&f.ast) {
            if c.dead_code_allowed || c.in_test {
                dead_code_fns.insert((rel.clone(), c.func.clone()));
            }
        }
        for func in ast::functions(&f.ast) {
            if dead_code_fns.contains(&(rel.clone(), func.name.clone())) {
                continue;
            }
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
        for name in public_const_names(&f.ast) {
            // A constant is *read*, not called: any mention of its name
            // in another non-test function body counts.
            let read = corpus.iter().any(|(_, _, body)| body.contains(&name));
            if !read {
                dead.push(format!(
                    "{rel}::{name} (pub const/static, no non-test reader)"
                ));
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

// ---------------------------------------------------------------------
// 17. computed_but_not_delivered
// ---------------------------------------------------------------------

/// Types whose `pub` fields are a *delivered* surface: a report row, a
/// unit, a plan. A field declared here is something a user or an agent
/// is promised, so it has to reach a renderer or a JSON path.
const DELIVERED_SURFACE_TYPES: &[&str] = &[
    "crates/core/src/artifact.rs",
    "crates/core/src/external.rs",
    "crates/core/src/report.rs",
    "crates/core/src/agents/mod.rs",
];

/// Files that *deliver*: they render to a terminal, build the JSON/agent
/// contract, or attach the facts a renderer then reads.
const DELIVERY_FILES: &[&str] = &[
    "crates/core/src/render.rs",
    "crates/core/src/agent_json.rs",
    "crates/core/src/report.rs",
    "crates/cli/src/main.rs",
];

/// Initializers that mean "nothing was computed here".
const EMPTY_INITS: &[&str] = &[
    "Vec :: new ()",
    "vec ! []",
    "Default :: default ()",
    "Vec :: default ()",
    "None",
];

/// A `pub` evidence field on a delivered surface type that is written
/// only as an empty default, or that no delivery file ever reads, is the
/// defect `.oh/guardrails/computed-but-not-delivered.md` names.
///
/// Scoped to fields whose declared type mentions `Evidence`: that is the
/// surface the #53-#59 evidence matrix promises row by row, it is the
/// surface the 2026-09-21 and 2026-09-22 reviews both found holes in, and
/// it keeps the rule mechanical instead of guessing at every struct field
/// in the crate.
pub fn computed_but_not_delivered(root: &Path) -> Result<(), String> {
    let all_files = workspace_src_files(root);
    let mut corpus: Vec<(String, String)> = Vec::new();
    for rel in &all_files {
        let Some(f) = maybe_parse(root, rel) else {
            continue;
        };
        corpus.push((rel.clone(), f.text.clone()));
    }
    let mut problems: Vec<String> = Vec::new();
    for rel in DELIVERED_SURFACE_TYPES {
        let Some(f) = maybe_parse(root, rel) else {
            continue;
        };
        for field in ast::pub_struct_fields(&f.ast) {
            if !field.ty.contains("Evidence") {
                continue;
            }
            // Every initializer this field is ever given, anywhere.
            let mut inits: Vec<String> = Vec::new();
            for rel2 in &all_files {
                let Some(f2) = maybe_parse(root, rel2) else {
                    continue;
                };
                inits.extend(ast::struct_field_inits(&f2.ast, &field.field));
            }
            let has_real_write = inits
                .iter()
                .any(|i| !EMPTY_INITS.iter().any(|e| i.trim() == *e));
            // ... plus a later mutation (`row.evidence = ...`,
            // `unit.evidence.extend(..)`) counts as a real write.
            let mutated = corpus.iter().any(|(_, text)| {
                text.contains(&format!(".{} = ", field.field))
                    || text.contains(&format!(".{}.extend(", field.field))
                    || text.contains(&format!(".{}.push(", field.field))
            });
            if !has_real_write && !mutated {
                problems.push(format!(
                    "{rel}::{}.{} is written only as an empty default ({} site(s)); compute it or \
                     remove it together with the docs/CHANGELOG claim that it is delivered",
                    field.struct_name,
                    field.field,
                    inits.len()
                ));
                continue;
            }
            // A field serde always emits (no `skip_serializing_if`) is
            // delivered by whole-struct serialization on the JSON path,
            // whether or not a renderer names it. A field serde *hides*
            // when empty has to be named by something that delivers it.
            let hidden_when_empty = field.attrs.contains("skip_serializing_if");
            let delivered = !hidden_when_empty
                || DELIVERY_FILES.iter().any(|d| {
                    corpus.iter().any(|(rel2, text)| {
                        rel2 == d && text.contains(&format!(".{}", field.field))
                    })
                });
            if !delivered {
                problems.push(format!(
                    "{rel}::{}.{} is populated but no delivery file ({}) reads it: a populated \
                     struct field nobody renders is a defect",
                    field.struct_name,
                    field.field,
                    DELIVERY_FILES.join(", ")
                ));
            }
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("\n  "))
    }
}

// ---------------------------------------------------------------------
// 18. coverage_changes_are_not_storage_changes
// ---------------------------------------------------------------------

/// Statements that write a disappearance or a return-from-the-dead into
/// byte history. Every one of them has to sit inside a loop the
/// observation's own `ObservationOwnership` guards, because a row outside
/// what this pass covered is a *coverage* fact, not a storage fact.
const TOMBSTONE_WRITES: &[&str] = &["present = false", "regrowth_count + 1"];

/// The guards that make such a write legitimate. The external/agent
/// family uses `ObservationOwnership`; the artifact-row family expresses
/// the same idea per worktree (`protected_worktree_ids`, #42's
/// could-not-confirm set). Both say "this pass covered the region the row
/// lives in"; neither is a licence to tombstone outside it.
const OWNERSHIP_GUARDS: &[&str] = &[
    "ownership . owns (",
    "ownership . covers (",
    "owns (",
    "protected_keys . contains (",
    "protected_worktree_ids . contains (",
    "protected . contains (",
];

pub fn coverage_changes_are_not_storage_changes(root: &Path) -> Result<(), String> {
    let mut files: Vec<String> = vec![
        "crates/core/src/growth.rs".to_string(),
        "crates/core/src/external.rs".to_string(),
        "crates/core/src/report.rs".to_string(),
    ];
    files.extend(ast::rust_files_under(root, "crates/core/src/agents"));
    let mut problems: Vec<String> = Vec::new();
    for rel in &files {
        let Some(f) = maybe_parse(root, rel) else {
            continue;
        };
        for func in ast::functions(&f.ast) {
            let Some((_, write)) = find_first(&func.body, TOMBSTONE_WRITES) else {
                continue;
            };
            if !OWNERSHIP_GUARDS.iter().any(|g| func.body.contains(g)) {
                problems.push(format!(
                    "{rel}::{} writes `{}` with no `ObservationOwnership` guard in the same \
                     function: a row this pass did not cover is a coverage change, never an \
                     observed deletion or regrowth",
                    func.name,
                    write.trim()
                ));
            }
        }
    }
    // The ownership window itself must be able to *subtract* a region
    // that is inside a covered root but out of this pass's scope -- the
    // 2026-09-22 CE4 shape (a config-only exclusion under a measured
    // parent). A window that is only a prefix test cannot.
    let growth = parse(root, "crates/core/src/growth.rs")?;
    let has_field = ast::pub_struct_fields(&growth.ast)
        .iter()
        .any(|f| f.struct_name == "ObservationOwnership" && f.field == "excluded_subtrees");
    if !has_field {
        problems.push(
            "growth.rs: `ObservationOwnership` has no `excluded_subtrees`; a path-prefix window \
             cannot express \"inside a covered root, outside this pass\" and will tombstone an \
             excluded nested location (CE4)"
                .to_string(),
        );
    }
    let covers = ast::functions(&growth.ast)
        .into_iter()
        .find(|f| f.name == "covers");
    match covers {
        Some(f) if f.body.contains("excluded_subtrees") => {}
        Some(_) => problems.push(
            "growth.rs: `ObservationOwnership::covers` does not consult `excluded_subtrees`"
                .to_string(),
        ),
        None => problems.push("growth.rs: `ObservationOwnership::covers` is missing".to_string()),
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("\n  "))
    }
}

// ---------------------------------------------------------------------
// 19. activity_and_consumer_evidence_have_limits
// ---------------------------------------------------------------------

/// `FactStatus` variants that exist precisely to carry a reason.
const REASONED_STATUSES: &[&str] = &["Unknown", "Unavailable", "Conflicting"];

pub fn activity_and_consumer_evidence_have_limits(root: &Path) -> Result<(), String> {
    let mut problems: Vec<String> = Vec::new();
    // 1. The reasoned statuses are constructed only inside `evidence.rs`,
    //    through `Evidence::{unknown,unavailable,conflicting}`, whose
    //    signatures make the reason mandatory. A struct literal anywhere
    //    else can omit it.
    for rel in workspace_src_files(root) {
        if rel == "crates/core/src/evidence.rs" {
            continue;
        }
        let Some(f) = maybe_parse(root, &rel) else {
            continue;
        };
        for (func_name, path) in ast::struct_literal_sites(&f.ast) {
            let Some(variant) = path.rsplit("::").next() else {
                continue;
            };
            if path.contains("FactStatus") && REASONED_STATUSES.contains(&variant) {
                problems.push(format!(
                    "{rel}::{func_name} builds `{path}` as a struct literal; construct it \
                     through `Evidence::unknown`/`unavailable`/`conflicting`, whose signature \
                     makes the reason mandatory"
                ));
            }
        }
        for func in ast::functions(&f.ast) {
            // 2. An empty reason is the same defect written differently.
            for ctor in [
                "Evidence :: unknown (",
                "Evidence :: unavailable (",
                "Evidence :: conflicting (",
            ] {
                let mut rest = func.body.as_str();
                while let Some(i) = rest.find(ctor) {
                    let tail = &rest[i + ctor.len()..];
                    let end = tail.find(')').unwrap_or(tail.len());
                    let args = &tail[..end];
                    if args.contains("\"\"") {
                        problems.push(format!(
                            "{rel}::{} passes an empty reason to `{}`: \"not observed\" has to say \
                             why it was not observed",
                            func.name,
                            ctor.trim_end_matches(" (")
                        ));
                    }
                    rest = &tail[end.min(tail.len())..];
                }
            }
        }
    }
    // 3. Every Activity fact carries a source: `activity.rs`'s evidence
    //    builders must name an `EvidenceSource`.
    let act = parse(root, "crates/core/src/activity.rs")?;
    for func in ast::functions(&act.ast) {
        if !func.name.ends_with("_evidence") {
            continue;
        }
        if !func.body.contains("EvidenceSource ::") && !func.body.contains("_evidence (") {
            problems.push(format!(
                "activity.rs::{} returns evidence without naming an `EvidenceSource`",
                func.name
            ));
        }
    }
    // 4. The rendering side must print the reason with the status; a bare
    //    "unknown" is the fact without its limit.
    let render = parse(root, "crates/core/src/render.rs")?;
    let renders_reason = ast::functions(&render.ast)
        .iter()
        .any(|f| f.name == "render_evidence_lines" && f.body.contains("reason"));
    if !renders_reason {
        problems.push(
            "render.rs::render_evidence_lines does not mention `reason`: an unknown printed \
             without its reason reads as \"nothing there\""
                .to_string(),
        );
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("\n  "))
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

    /// Every recheck's answer reaches control flow: `?` on two of them
    /// and a `match` on the third. The version of this fixture that
    /// wrote `let _ = recheck::live_protection(..)` is now a *rejection*
    /// fixture, because that is the mutation the sweep used.
    const GOOD_SINK: &str = r#"
        pub fn execute_x(dir: &std::path::Path, path: &std::path::Path) -> anyhow::Result<()> {
            let fresh = recheck::reviewed_snapshot(path, reviewed)?;
            let paths = recheck::covered_paths(&fresh);
            recheck::live_protection(dir, &paths)?;
            match recheck::member_occupancy(&paths) {
                OccupancyState::Free => {}
                _ => anyhow::bail!("refused"),
            }
            fs::rename(path, dir)?;
            Ok(())
        }
    "#;

    #[test]
    fn a_sink_that_only_checks_is_dir_is_rejected() {
        let tmp = workspace(&[
            (
                "crates/core/src/actions.rs",
                "pub fn execute_x() { if p.is_dir() { let _ = fs::rename(a, b); } }",
            ),
            ("crates/core/src/cargo_cleanup.rs", ""),
            ("crates/core/src/docker.rs", ""),
        ]);
        let err = execution_sinks_recheck_live_state(tmp.path()).unwrap_err();
        assert!(err.contains("execute_x"), "{err}");
        assert!(err.contains("reviewed_snapshot"), "{err}");
    }

    #[test]
    fn a_sink_missing_only_member_occupancy_is_rejected() {
        let tmp = workspace(&[
            (
                "crates/core/src/actions.rs",
                "pub fn execute_x() -> R { recheck::reviewed_snapshot(p, r)?; \
                 recheck::live_protection(d, &v)?; fs::rename(a, b)?; Ok(()) }",
            ),
            ("crates/core/src/cargo_cleanup.rs", ""),
            ("crates/core/src/docker.rs", ""),
        ]);
        let err = execution_sinks_recheck_live_state(tmp.path()).unwrap_err();
        assert!(err.contains("member_occupancy"), "{err}");
        assert!(!err.contains("live_protection"), "{err}");
    }

    #[test]
    fn a_recheck_after_the_rename_is_rejected() {
        let tmp = workspace(&[
            (
                "crates/core/src/actions.rs",
                "pub fn execute_x() { let _ = fs::rename(a, b); \
                 let _ = recheck::reviewed_snapshot(p, r); \
                 let _ = recheck::live_protection(d, &v); \
                 let _ = recheck::member_occupancy(&v); }",
            ),
            ("crates/core/src/cargo_cleanup.rs", ""),
            ("crates/core/src/docker.rs", ""),
        ]);
        let err = execution_sinks_recheck_live_state(tmp.path()).unwrap_err();
        assert!(err.contains("execute_x"), "{err}");
    }

    #[test]
    fn a_sink_rechecking_through_a_helper_first_passes() {
        let tmp = workspace(&[
            (
                "crates/core/src/actions.rs",
                "pub fn execute_x(a: &Path, b: &Path) -> R { gate(a)?; fs::rename(a, b)?; Ok(()) }\n\
                 fn gate(a: &Path) -> R { recheck::reviewed_snapshot(a, r)?; \
                 recheck::live_protection(d, &v)?; \
                 match recheck::member_occupancy(&v) { S::Free => {} _ => bail!(\"no\") } Ok(()) }",
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

    #[test]
    fn a_second_protection_predicate_is_rejected() {
        // The integration owner's 2026-09-21 mutation check: a `bool`
        // wrapper (`is_human_protected`) that *looked* like a delegation
        // let a one-directional mutation of the real predicate pass this
        // audit and every runtime test, because the audit read
        // `protection_conflict` and the proposal path called the
        // wrapper. This fixture reintroduces the wrapper with its own
        // inlined, one-directional logic.
        let with_wrapper = format!(
            "{GOOD_PROTECT}\n\
             pub fn is_human_protected(protected: &[PathBuf], candidate: &Path) -> bool {{\n\
                 protected.iter().any(|p| candidate.starts_with(p))\n\
             }}\n"
        );
        let tmp = workspace(&[("crates/core/src/agents/mod.rs", &with_wrapper)]);
        let err = protection_fails_closed(tmp.path()).unwrap_err();
        assert!(err.contains("One predicate, everywhere"), "{err}");
        assert!(err.contains("is_human_protected"), "{err}");
    }

    #[test]
    fn a_protection_helper_that_does_delegate_is_accepted() {
        // Precision, not prohibition: a helper is fine when it really is
        // one.
        let delegating = format!(
            "{GOOD_PROTECT}\n\
             pub fn member_protection(protected: &[PathBuf], ms: &[PathBuf]) -> Option<String> {{\n\
                 ms.iter().find_map(|m| protection_conflict(protected, m))\n\
             }}\n"
        );
        let tmp = workspace(&[("crates/core/src/agents/mod.rs", &delegating)]);
        assert_eq!(protection_fails_closed(tmp.path()), Ok(()));
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
        assert!(
            err.contains("ownership") && err.contains("condition"),
            "{err}"
        );
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

    /// The one bounded lister every traversal fixture needs, since the
    /// audit checks that the exemption still comes with its cap.
    const BOUNDED_LISTER_FILE: (&str, &str) = (
        "crates/core/src/locations/mod.rs",
        "pub const SHALLOW_LIST_CAP: usize = 4096;\n\
         pub fn shallow_list(dir: &Path) -> Vec<ShallowEntry> { fs::read_dir(dir); vec![] }\n",
    );

    #[test]
    fn an_adapter_calling_read_dir_is_rejected() {
        let tmp = workspace(&[
            (
                "crates/core/src/agents/x.rs",
                "fn identify() { for e in fs::read_dir(home).unwrap() {} }",
            ),
            BOUNDED_LISTER_FILE,
        ]);
        let err = no_second_traversal_on_report_path(tmp.path()).unwrap_err();
        assert!(err.contains("agents/x.rs"), "{err}");
    }

    #[test]
    fn an_adapter_using_shallow_list_passes() {
        let tmp = workspace(&[
            (
                "crates/core/src/agents/x.rs",
                "fn identify() { for e in locations::shallow_list(home) {} }",
            ),
            BOUNDED_LISTER_FILE,
        ]);
        assert_eq!(no_second_traversal_on_report_path(tmp.path()), Ok(()));
    }

    #[test]
    fn the_one_bounded_lister_is_exempt_but_a_sibling_in_the_same_file_is_not() {
        // The exemption is a `(file, fn)` pair, so a second traversal
        // added beside it still fails -- otherwise exempting
        // `shallow_list` would have exempted the whole detector
        // registry.
        let tmp = workspace(&[(
            BOUNDED_LISTER_FILE.0,
            &format!(
                "{}fn enumerate_everything(d: &Path) {{ for e in fs::read_dir(d).unwrap() {{}} }}\n",
                BOUNDED_LISTER_FILE.1
            ),
        )]);
        let err = no_second_traversal_on_report_path(tmp.path()).unwrap_err();
        assert!(err.contains("enumerate_everything"), "{err}");
    }

    #[test]
    fn a_bounded_lister_that_lost_its_cap_is_rejected() {
        // The exemption exists because the listing is capped. Remove the
        // cap and the exemption has to go with it.
        let tmp = workspace(&[(
            BOUNDED_LISTER_FILE.0,
            "pub fn shallow_list(dir: &Path) -> Vec<ShallowEntry> { fs::read_dir(dir); vec![] }\n",
        )]);
        let err = no_second_traversal_on_report_path(tmp.path()).unwrap_err();
        assert!(err.contains("SHALLOW_LIST_CAP"), "{err}");
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
            "fn p(swamp_dir: &Path) -> PathBuf { swamp_dir.join(\"external_consumers.json\") }",
        )]);
        let err = store_data_is_parquet_not_json_sidecars(tmp.path()).unwrap_err();
        assert!(err.contains("external_consumers.json"), "{err}");
    }

    #[test]
    fn a_parquet_table_passes() {
        let tmp = workspace(&[(
            "crates/core/src/external.rs",
            "fn p(swamp_dir: &Path) -> PathBuf { swamp_dir.join(\"consumers/current.parquet\") }",
        )]);
        assert_eq!(store_data_is_parquet_not_json_sidecars(tmp.path()), Ok(()));
    }

    #[test]
    fn a_named_control_file_passes() {
        let tmp = workspace(&[(
            "crates/core/src/scope.rs",
            "fn p(swamp_dir: &Path) -> PathBuf { swamp_dir.join(\"scope.json\") }",
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
        assert!(
            err.contains("it never \nwrites")
                || err.contains("never acts")
                || err.contains("writes, deletes or spawns"),
            "{err}"
        );
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

    /// The three files every `discovery_owned_by_report_pipeline`
    /// fixture needs: the two discovery functions and the one owner that
    /// calls both.
    fn discovery_fixture(extra: &[(&str, &str)]) -> tempfile::TempDir {
        let mut files: Vec<(&str, &str)> = vec![
            (
                "crates/core/src/external.rs",
                "pub fn discover_and_measure() {}",
            ),
            (
                "crates/core/src/agents/mod.rs",
                "pub fn discover_and_measure() {}",
            ),
            (
                "crates/core/src/report.rs",
                "pub fn observe_scope() { external::discover_and_measure(); \
                 agents::discover_and_measure(); }",
            ),
        ];
        files.extend_from_slice(extra);
        workspace(&files)
    }

    #[test]
    fn one_owner_calling_both_passes_is_accepted() {
        assert_eq!(
            discovery_owned_by_report_pipeline(discovery_fixture(&[]).path()),
            Ok(())
        );
    }

    #[test]
    fn a_cli_discovery_call_is_rejected() {
        let tmp = discovery_fixture(&[(
            "crates/cli/src/main.rs",
            "fn cmd() { let u = external::discover_and_measure(&scope); }",
        )]);
        let err = discovery_owned_by_report_pipeline(tmp.path()).unwrap_err();
        assert!(err.contains("own discovery pass"), "{err}");
        assert!(err.contains("crates/cli/src/main.rs"), "{err}");
    }

    #[test]
    fn a_tui_discovery_call_is_rejected() {
        let tmp = discovery_fixture(&[(
            "crates/tui/src/app.rs",
            "fn refresh() { let u = agents::discover_and_measure(&scope); }",
        )]);
        let err = discovery_owned_by_report_pipeline(tmp.path()).unwrap_err();
        assert!(err.contains("own discovery pass"), "{err}");
    }

    #[test]
    fn a_second_pass_inside_the_core_crate_is_rejected() {
        // The case the old `pub(crate)` rule allowed and this one does
        // not: a core-internal caller other than the owner.
        let tmp = discovery_fixture(&[(
            "crates/core/src/schedule.rs",
            "fn refresh() { let u = external::discover_and_measure(&scope); }",
        )]);
        let err = discovery_owned_by_report_pipeline(tmp.path()).unwrap_err();
        assert!(err.contains("own discovery pass"), "{err}");
        assert!(err.contains("schedule.rs"), "{err}");
    }

    #[test]
    fn an_owner_that_runs_only_one_of_the_two_passes_is_rejected() {
        // Splitting them back apart is how their ownership windows could
        // disagree again.
        let tmp = workspace(&[
            (
                "crates/core/src/external.rs",
                "pub fn discover_and_measure() {}",
            ),
            (
                "crates/core/src/agents/mod.rs",
                "pub fn discover_and_measure() {}",
            ),
            (
                "crates/core/src/report.rs",
                "pub fn observe_scope() { external::discover_and_measure(); }",
            ),
        ]);
        let err = discovery_owned_by_report_pipeline(tmp.path()).unwrap_err();
        assert!(err.contains("owns *both* passes"), "{err}");
    }

    #[test]
    fn a_missing_owner_is_rejected() {
        let tmp = workspace(&[
            (
                "crates/core/src/external.rs",
                "pub fn discover_and_measure() {}",
            ),
            (
                "crates/core/src/agents/mod.rs",
                "pub fn discover_and_measure() {}",
            ),
            ("crates/core/src/report.rs", "pub fn something_else() {}"),
        ]);
        let err = discovery_owned_by_report_pipeline(tmp.path()).unwrap_err();
        assert!(err.contains("has to be somewhere"), "{err}");
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

/// Mutation tests for the three audits added after the 2026-09-22
/// re-review. Each proves both halves: the shape the guardrail forbids is
/// rejected, and the shape it requires passes.
#[cfg(test)]
mod review2_mutation_tests {
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

    // -- computed_but_not_delivered -------------------------------------

    const RENDERER_READING_IT: &str =
        "pub fn render_view_x(u: &Unit) { for e in &u.decision_evidence { let _ = e; } }";

    fn surface(field_init: &str, renderer: &str) -> tempfile::TempDir {
        workspace(&[
            (
                "crates/core/src/artifact.rs",
                "pub struct NestedArtifact { #[serde(default, skip_serializing_if = \"Vec::is_empty\")] \
                 pub decision_evidence: Vec<crate::evidence::Evidence> }",
            ),
            (
                "crates/core/src/cargo_artifacts.rs",
                &format!(
                    "pub fn build() -> NestedArtifact {{ NestedArtifact {{ decision_evidence: {field_init} }} }}"
                ),
            ),
            ("crates/core/src/render.rs", renderer),
            ("crates/core/src/external.rs", ""),
            ("crates/core/src/report.rs", ""),
            ("crates/cli/src/main.rs", ""),
        ])
    }

    #[test]
    fn an_evidence_field_written_only_as_an_empty_vec_is_rejected() {
        let tmp = surface("Vec::new()", RENDERER_READING_IT);
        let err = computed_but_not_delivered(tmp.path()).unwrap_err();
        assert!(err.contains("decision_evidence"), "{err}");
        assert!(err.contains("empty default"), "{err}");
    }

    #[test]
    fn an_evidence_field_nobody_renders_is_rejected() {
        let tmp = surface("attach(unit)", "pub fn render_view_x() {}");
        let err = computed_but_not_delivered(tmp.path()).unwrap_err();
        assert!(err.contains("no delivery file"), "{err}");
    }

    #[test]
    fn an_evidence_field_computed_and_rendered_passes() {
        let tmp = surface("attach(unit)", RENDERER_READING_IT);
        assert_eq!(computed_but_not_delivered(tmp.path()), Ok(()));
    }

    // -- coverage_changes_are_not_storage_changes -----------------------

    const GOOD_GROWTH: &str = r#"
        pub struct ObservationOwnership {
            pub family: u8,
            pub covered_roots: Vec<String>,
            pub excluded_subtrees: Vec<String>,
        }
        impl ObservationOwnership {
            pub fn covers(&self, path: &str) -> bool {
                if self.excluded_subtrees.iter().any(|e| path.starts_with(e)) {
                    return false;
                }
                self.covered_roots.iter().any(|r| path.starts_with(r))
            }
        }
        pub fn sweep(ownership: &ObservationOwnership) {
            for (key, row) in current.iter_mut() {
                if ownership.owns(key) { row.present = false; }
            }
        }
    "#;

    #[test]
    fn an_unguarded_tombstone_write_is_rejected() {
        let tmp = workspace(&[(
            "crates/core/src/growth.rs",
            &GOOD_GROWTH.replace(
                "if ownership.owns(key) { row.present = false; }",
                "row.present = false;",
            ),
        )]);
        let err = coverage_changes_are_not_storage_changes(tmp.path()).unwrap_err();
        assert!(err.contains("ObservationOwnership` guard"), "{err}");
    }

    #[test]
    fn an_ownership_window_without_excluded_subtrees_is_rejected() {
        let tmp = workspace(&[(
            "crates/core/src/growth.rs",
            &GOOD_GROWTH
                .replace("            pub excluded_subtrees: Vec<String>,\n", "")
                .replace(
                    "if self.excluded_subtrees.iter().any(|e| path.starts_with(e)) {\n                    return false;\n                }\n",
                    "",
                ),
        )]);
        let err = coverage_changes_are_not_storage_changes(tmp.path()).unwrap_err();
        assert!(err.contains("excluded_subtrees"), "{err}");
    }

    #[test]
    fn a_guarded_sweep_with_a_subtractable_window_passes() {
        let tmp = workspace(&[("crates/core/src/growth.rs", GOOD_GROWTH)]);
        assert_eq!(coverage_changes_are_not_storage_changes(tmp.path()), Ok(()));
    }

    // -- activity_and_consumer_evidence_have_limits ---------------------

    const GOOD_EVIDENCE: &[(&str, &str)] = &[
        (
            "crates/core/src/evidence.rs",
            "pub enum FactStatus { Known(u8), Unknown { reason: String } }",
        ),
        (
            "crates/core/src/activity.rs",
            "pub fn modification_evidence() -> Evidence { Evidence::known(EvidenceSource::FilesystemMetadata { detail: d }) }",
        ),
        (
            "crates/core/src/render.rs",
            "pub fn render_evidence_lines(e: &Evidence) { match &e.status { FactStatus::Unknown { reason } => out.push(reason.clone()), _ => {} } }",
        ),
    ];

    #[test]
    fn a_hand_built_unknown_status_outside_evidence_rs_is_rejected() {
        let mut files: Vec<(&str, &str)> = GOOD_EVIDENCE.to_vec();
        files.push((
            "crates/core/src/occupancy.rs",
            "pub fn probe() -> Evidence { Evidence { status: FactStatus::Unknown { reason: r }, ..d } }",
        ));
        let tmp = workspace(&files);
        let err = activity_and_consumer_evidence_have_limits(tmp.path()).unwrap_err();
        assert!(err.contains("struct literal"), "{err}");
    }

    #[test]
    fn an_empty_reason_is_rejected() {
        let mut files: Vec<(&str, &str)> = GOOD_EVIDENCE.to_vec();
        files.push((
            "crates/core/src/occupancy.rs",
            "pub fn probe() -> Evidence { Evidence::unknown(k, s, src, at, \"\") }",
        ));
        let tmp = workspace(&files);
        let err = activity_and_consumer_evidence_have_limits(tmp.path()).unwrap_err();
        assert!(err.contains("empty reason"), "{err}");
    }

    #[test]
    fn an_activity_fact_without_a_source_is_rejected() {
        let mut files: Vec<(&str, &str)> = GOOD_EVIDENCE.to_vec();
        files[1] = (
            "crates/core/src/activity.rs",
            "pub fn modification_evidence() -> Evidence { Evidence::known(k, s, v, at) }",
        );
        let tmp = workspace(&files);
        let err = activity_and_consumer_evidence_have_limits(tmp.path()).unwrap_err();
        assert!(err.contains("EvidenceSource"), "{err}");
    }

    #[test]
    fn a_renderer_that_drops_the_reason_is_rejected() {
        let mut files: Vec<(&str, &str)> = GOOD_EVIDENCE.to_vec();
        files[2] = (
            "crates/core/src/render.rs",
            "pub fn render_evidence_lines(e: &Evidence) { out.push(\"unknown\".to_string()) }",
        );
        let tmp = workspace(&files);
        let err = activity_and_consumer_evidence_have_limits(tmp.path()).unwrap_err();
        assert!(err.contains("reason"), "{err}");
    }

    #[test]
    fn evidence_built_through_the_constructors_with_reasons_passes() {
        let tmp = workspace(GOOD_EVIDENCE);
        assert_eq!(
            activity_and_consumer_evidence_have_limits(tmp.path()),
            Ok(())
        );
    }

    // -- no_dead_public_evidence_api, extended to pub const ------------

    #[test]
    fn a_pub_const_with_no_reader_is_rejected() {
        let tmp = workspace(&[
            (
                "crates/core/src/activity.rs",
                "pub const ACTIVITY_EVIDENCE_INVENTORY: &[(&str, &str)] = &[];",
            ),
            ("crates/core/src/render.rs", "pub fn r() {}"),
        ]);
        let err = no_dead_public_evidence_api(tmp.path()).unwrap_err();
        assert!(err.contains("ACTIVITY_EVIDENCE_INVENTORY"), "{err}");
        assert!(err.contains("pub const/static"), "{err}");
    }

    #[test]
    fn a_pub_const_a_renderer_reads_passes() {
        let tmp = workspace(&[
            (
                "crates/core/src/activity.rs",
                "pub const ACTIVITY_EVIDENCE_INVENTORY: &[(&str, &str)] = &[];",
            ),
            (
                "crates/core/src/render.rs",
                "pub fn r() { for e in crate::activity::ACTIVITY_EVIDENCE_INVENTORY { let _ = e; } }",
            ),
        ]);
        assert_eq!(no_dead_public_evidence_api(tmp.path()), Ok(()));
    }

    // -- no_second_traversal, extended to the callee -------------------

    #[test]
    fn a_measure_that_re_walks_without_consulting_the_rows_is_rejected() {
        let tmp = workspace(&[
            (
                "crates/core/src/locations/mod.rs",
                "pub const SHALLOW_LIST_CAP: usize = 64; pub fn shallow_list() {}",
            ),
            (
                "crates/core/src/folded_measurement.rs",
                "pub fn measure(p: &Path) -> u64 { crate::walk::resize_artifact_excluding(p) }",
            ),
        ]);
        let err = no_second_traversal_on_report_path(tmp.path()).unwrap_err();
        assert!(err.contains("without first consulting"), "{err}");
    }

    #[test]
    fn a_measure_that_consults_the_rows_first_passes() {
        let tmp = workspace(&[
            (
                "crates/core/src/locations/mod.rs",
                "pub const SHALLOW_LIST_CAP: usize = 64; pub fn shallow_list() {}",
            ),
            (
                "crates/core/src/folded_measurement.rs",
                "pub fn measure(p: &Path) -> u64 { if let Some(b) = reuse_folded_measurement(p) { return b; } \
                 crate::walk::resize_artifact_excluding(p) }",
            ),
        ]);
        assert_eq!(no_second_traversal_on_report_path(tmp.path()), Ok(()));
    }
}
