//! Store guardrails: data is Parquet, JSON is a fixed set of small
//! control files written by named writers, and history is changed only
//! where this observation owns the row.

use super::{load, verdict};
use crate::program::{self, Fun, PCall, Program, contains_token};
use crate::resolve;
use std::collections::HashSet;
use std::path::Path;

/// Small control/recovery files that may live under the store as JSON.
/// Everything per-unit, per-worktree or per-row is Parquet.
pub const STORE_CONTROL_FILES: &[&str] = &[
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

/// Control files whose name carries an id, with the placeholder in place.
pub const STORE_CONTROL_PATTERNS: &[&str] = &["plans/{}.json", "{}.json.zst"];

/// `(file, fn, justification)`: every function allowed to persist JSON.
/// Each names a control or recovery artifact; none names per-row data.
/// An entry that no longer serializes JSON into a write fails, so the
/// list cannot rot.
pub const JSON_WRITE_ALLOWLIST: &[(&str, &str, &str)] = &[
    ("crates/core/src/actions.rs", "save_plan", "one unapproved plan per file; a control artifact a human reviews and approves"),
    ("crates/core/src/actions.rs", "write_restore_manifest", "Trash envelope recovery manifest, written next to the moved members"),
    ("crates/core/src/actions.rs", "write_grants", "the authorization grant list: small, human-auditable control state"),
    ("crates/core/src/ledger.rs", "append", "append-only action ledger (jsonl); a durable record, not a queryable table"),
    ("crates/core/src/schedule.rs", "write_last_run", "last scheduled-run marker: a single timestamp record"),
    ("crates/core/src/scope.rs", "persist_effective_scope", "the resolved scope snapshot, for the next run's coverage diff"),
    ("crates/core/src/agents/mod.rs", "save_protect", "human keep/protect list: small control state, written atomically"),
    ("crates/core/src/cargo_cleanup.rs", "move_reviewed", "Trash envelope recovery manifest for an exact reviewed Cargo group"),
    ("crates/core/src/growth.rs", "write_fsevents_state", "the FSEvents cursor: one event id and a mode, read once per observation"),
    ("crates/core/src/growth.rs", "write_unit_root_cursor", "the unit-root half of the same FSEvents cursor file"),
    ("crates/core/src/growth.rs", "write_topology", "the worktree topology snapshot an incremental replay diffs against"),
    ("crates/core/src/growth.rs", "write_unowned", "the unowned-paths cache an incremental pass carries forward"),
    ("crates/core/src/report.rs", "write_last_report", "the compressed last-report cache the TUI paints from before observing"),
    ("crates/core/src/report.rs", "write_last_scope_report", "the same last-report cache, keyed by a multi-root scope"),
    ("crates/tui/src/app.rs", "persist_ui_state", "the remembered filter/sort/reverse of the last TUI session"),
    // Found by the program-model dataflow on 2026-09-22: `load_cached`
    // serialized the Docker facts inside an `if let` and wrote them in the
    // next statement, which the old statement-split heuristic could not
    // connect. It is the `docker_facts.json` control file with a TTL.
    ("crates/core/src/docker.rs", "load_cached", "the Docker facts cache: one small document with a TTL, re-probed when stale"),
];

/// A call that turns a value into JSON bytes: any `serde_json::to_*`
/// except `to_value` (which builds a value, not bytes), or `.to_string()`
/// / `.to_vec()` on a `json!` value.
fn serializes(c: &PCall) -> bool {
    (program::serializes_json(c) && !c.is("serde_json::to_value"))
        || (c.method && ["to_string", "to_vec"].contains(&c.path.as_str()) && c.receiver.contains("json !"))
}

/// Does a serialized value reach a write in `f`?
fn json_reaches_write(p: &Program, i: usize) -> bool {
    let f = &p.funs[i];
    let writes = p.destructive();
    let ser: Vec<&PCall> = f.calls.iter().filter(|c| serializes(c)).collect();
    let macro_ser = f.macros.iter().any(|m| m.tokens.contains("serde_json :: to_") || m.tokens.contains("json !"));
    if ser.is_empty() && !macro_ser {
        return false;
    }
    // Names bound from a serializer: `let x = serde_json::to_*(..)`, and
    // `if let Ok(x) = serde_json::to_*(..)`.
    let mut seeds: Vec<String> = f
        .bindings
        .iter()
        .filter(|b| b.from.contains("serde_json :: to_") || b.from.contains("json !"))
        .map(|b| b.name.clone())
        .collect();
    let body = &f.body;
    let mut rest = body.as_str();
    while let Some(at) = rest.find("if let Ok (") {
        let tail = &rest[at + "if let Ok (".len()..];
        let name: String = tail.trim_start().chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        let init = tail.split('{').next().unwrap_or("");
        if !name.is_empty() && (init.contains("serde_json :: to_") || init.contains("json !")) {
            seeds.push(name);
        }
        rest = tail;
    }
    let tainted = resolve::derived_from(&f.bindings, &f.name, &seeds);
    for (ci, c) in f.calls.iter().enumerate() {
        let is_write = program::destructive_call(f, c)
            || (!p.target(i, ci).possible && p.target(i, ci).local.iter().any(|g| writes.contains(g)));
        if !is_write {
            continue;
        }
        let args = c.args.join(" , ");
        if args.contains("serde_json :: to_")
            || args.contains("json !")
            || args.split(|ch: char| !(ch.is_alphanumeric() || ch == '_')).any(|t| tainted.contains(t))
        {
            return true;
        }
    }
    // `to_writer(file, ..)` writes as it serializes.
    if f.calls.iter().any(|c| c.is("serde_json::to_writer") || c.is("serde_json::to_writer_pretty"))
        && !f.calls.iter().any(|c| c.is("io::stdout") || c.is("io::stderr"))
    {
        return true;
    }
    // `writeln!(file, "{}", serde_json::to_string(..)?)`: a write macro
    // into anything but a terminal or a string buffer.
    f.macros.iter().any(|m| {
        ["write", "writeln"].contains(&m.name.as_str())
            && m.tokens.contains("serde_json :: to_")
            && {
                let target = resolve::root_ident(m.tokens.split(',').next().unwrap_or(""));
                let param_ty = f.params.iter().find(|(n, _)| resolve::root_ident(n) == target).map(|(_, t)| t.clone()).unwrap_or_default();
                !(target.contains("stdout") || target.contains("stderr") || contains_token(&param_ty, "String") || contains_token(&param_ty, "Formatter"))
            }
    })
}

pub fn json_persistence_is_allowlisted(root: &Path) -> Result<(), String> {
    let p = load(root);
    let mut problems = Vec::new();
    let mut found: HashSet<(String, String)> = HashSet::new();
    for (i, f) in p.funs.iter().enumerate() {
        if !json_reaches_write(&p, i) {
            continue;
        }
        found.insert((f.rel.clone(), f.name.clone()));
        let allowed = JSON_WRITE_ALLOWLIST.iter().any(|(rel, name, _)| *rel == f.rel && *name == f.name);
        if !allowed {
            problems.push(format!(
                "{} serializes JSON into a file write without a JSON_WRITE_ALLOWLIST entry: JSON \
                 is an output format and a format for a fixed set of small control files",
                f.display()
            ));
        }
    }
    for (rel, name, _) in JSON_WRITE_ALLOWLIST {
        if !found.contains(&(rel.to_string(), name.to_string())) {
            problems.push(format!(
                "JSON_WRITE_ALLOWLIST names {rel}::{name}, which no longer serializes JSON into a \
                 write; remove the stale entry"
            ));
        }
    }
    verdict("JSON persistence is allow-listed", problems)
}

/// Every `*.json`/`*.jsonl`/`*.json.zst` file name a function can write:
/// its own literals, the constants it names, and the literals of the
/// local path helpers it calls.
fn json_names(p: &Program, i: usize) -> Vec<String> {
    let f = &p.funs[i];
    let mut lits: Vec<String> = f.literals.clone();
    for d in p.consts_reached(f) {
        lits.extend(d.literals.iter().cloned());
    }
    for g in p.callees(i) {
        let gf = &p.funs[*g];
        if contains_token(&gf.ret, "PathBuf") {
            lits.extend(gf.literals.iter().cloned());
        }
    }
    lits.into_iter()
        .map(|l| resolve::strip_placeholders(&l))
        .filter(|l| (l.ends_with(".json") || l.ends_with(".jsonl") || l.ends_with(".json.zst")) && !l.contains(' ') && !l.starts_with('.'))
        .collect()
}

pub fn store_data_is_parquet_not_json_sidecars(root: &Path) -> Result<(), String> {
    let p = load(root);
    let mut problems = Vec::new();
    let writes = p.destructive();
    for (i, f) in p.funs.iter().enumerate() {
        // Only a function that writes can create a sidecar; an adapter
        // naming another tool's `settings.json` is reading it.
        let writes_here = f.calls.iter().enumerate().any(|(ci, c)| {
            (program::destructive_call(f, c) && !program::spawn_call(c))
                || (!p.target(i, ci).possible && p.target(i, ci).local.iter().any(|g| writes.contains(g)))
        });
        if !writes_here {
            continue;
        }
        for name in json_names(&p, i) {
            let file = name.rsplit('/').next().unwrap_or(&name);
            if STORE_CONTROL_FILES.contains(&file) || STORE_CONTROL_PATTERNS.iter().any(|pat| name.ends_with(pat) || name == *pat) {
                continue;
            }
            problems.push(format!(
                "{} writes the JSON file {name:?}, which is not one of the small control files: \
                 per-unit/per-row data belongs in the Parquet current + reverse-delta store",
                f.display()
            ));
        }
    }
    verdict("store data is Parquet, never JSON sidecars", problems)
}

// ---------------------------------------------------------------------
// History writes: tombstones and regrowth
// ---------------------------------------------------------------------

/// The ownership predicates: methods of the ownership window type.
fn ownership_methods(p: &Program) -> Vec<String> {
    p.funs
        .iter()
        .filter(|f| f.self_ty.as_deref() == Some("ObservationOwnership") && f.returns_verdict())
        .map(|f| f.name.clone())
        .collect()
}

/// Whether a condition is an ownership test: it calls an ownership
/// method, or it excludes a region the *caller* declared this pass could
/// not confirm (a negated membership test on a parameter).
fn ownership_condition(f: &Fun, cond: &str, methods: &[String]) -> bool {
    let c = cond.replace(' ', "");
    if methods.iter().any(|m| c.contains(&format!(".{m}("))) {
        return true;
    }
    f.params.iter().any(|(pat, _)| {
        let n = resolve::root_ident(pat);
        !n.is_empty() && n != "self" && c.contains(&format!("!{n}.contains("))
    })
}

/// Whether a row was *found by an observation's key* rather than taken
/// from a wholesale iteration: a keyed lookup whose key comes from a
/// loop over something other than the table being written.
fn observed_lookup(f: &Fun, cond: &str) -> bool {
    let c = cond.replace(' ', "");
    for m in [".get_mut(", ".get("] {
        let Some(at) = c.find(m) else { continue };
        let map = resolve::root_ident(&c[..at]);
        let arg = resolve::root_ident(&c[at + m.len()..]);
        if let Some(b) = f.bindings.iter().find(|b| b.name == arg) {
            if resolve::root_ident(&b.from) != map {
                return true;
            }
        }
    }
    false
}

/// The value an assignment's right-hand side evaluates to, when it is a
/// literal or a constant.
fn falsy(p: &Program, rhs: &str) -> bool {
    match p.eval_literal(rhs) {
        Some(v) => v == "false",
        // A value this audit cannot evaluate might be false.
        None => rhs.trim() != "true",
    }
}

pub(crate) struct HistoryWrite {
    pub who: String,
    pub what: String,
    pub tombstone: bool,
}

/// Every tombstone (`.present = <false>`) and regrowth bump
/// (`.regrowth_count += n`, `= .. + n`, or a binding computed that way)
/// that is not inside a condition that makes it this observation's.
pub(crate) fn unowned_history_writes(p: &Program) -> (Vec<HistoryWrite>, usize) {
    let methods = ownership_methods(p);
    let mut out = Vec::new();
    let mut seen = 0usize;
    for f in p.funs.iter() {
        for a in &f.assigns {
            let lhs = a.lhs.replace(' ', "");
            let rhs = a.rhs.replace(' ', "");
            let tombstone = lhs.ends_with(".present") && a.op == "=" && falsy(p, &a.rhs);
            let bump = lhs.ends_with(".regrowth_count")
                && (a.op == "+=" || rhs.contains("regrowth_count+") || {
                    let r = resolve::root_ident(&a.rhs);
                    f.bindings.iter().any(|b| b.name == r && b.from.replace(' ', "").contains("regrowth_count+"))
                });
            if !tombstone && !bump {
                continue;
            }
            seen += 1;
            let owned = a.conditions.iter().any(|c| ownership_condition(f, c, &methods))
                || (bump && a.conditions.iter().any(|c| observed_lookup(f, c)));
            if !owned {
                out.push(HistoryWrite {
                    who: f.display(),
                    what: format!("{lhs} {} {rhs}", a.op),
                    tombstone,
                });
            }
        }
    }
    (out, seen)
}

pub fn history_sweeps_are_owned(root: &Path) -> Result<(), String> {
    let p = load(root);
    let mut problems = Vec::new();
    if !p.types.iter().any(|t| t.name == "ObservationOwnership") {
        problems.push("`ObservationOwnership` is not defined".into());
    }
    let (writes, seen) = unowned_history_writes(&p);
    if seen == 0 {
        problems.push("nothing tombstones a row any more".into());
    }
    for w in writes.iter().filter(|w| w.tombstone) {
        problems.push(format!(
            "{} writes `{}` with no ownership test in the condition that guards it",
            w.who, w.what
        ));
    }
    // A sweep guarded by the ownership window is handed it: the window
    // comes from the caller, which states what it covered.
    let methods = ownership_methods(&p);
    for f in p.funs.iter() {
        let uses_window = f.assigns.iter().any(|a| a.conditions.iter().any(|c| methods.iter().any(|m| c.replace(' ', "").contains(&format!(".{m}(")))));
        if uses_window && !f.params.iter().any(|(_, t)| contains_token(t, "ObservationOwnership")) && f.self_ty.as_deref() != Some("ObservationOwnership") {
            problems.push(format!("{} sweeps with an ownership window it was not handed", f.display()));
        }
        // No wildcard window.
        let b = f.body.replace(' ', "");
        if b.contains("ObservationOwnership::all(")
            || b.contains("ObservationOwnership::wildcard(")
            || (b.contains("ObservationOwnership::new(") && (b.contains("PathBuf::from(\"/\")") || b.contains("Path::new(\"/\")")))
        {
            problems.push(format!("{} constructs a wildcard ObservationOwnership", f.display()));
        }
    }
    verdict(
        "one family's sweep never tombstones another's rows: every tombstone is inside an \
         ownership test",
        problems,
    )
}

pub fn coverage_changes_are_not_storage_changes(root: &Path) -> Result<(), String> {
    let p = load(root);
    let mut problems = Vec::new();
    let (writes, _) = unowned_history_writes(&p);
    for w in &writes {
        problems.push(format!(
            "{} writes `{}` outside a condition that makes the row this observation's: a row this \
             pass did not cover is a coverage change, never an observed deletion or regrowth",
            w.who, w.what
        ));
    }
    let has_field = p.types.iter().any(|t| t.name == "ObservationOwnership" && t.fields.iter().any(|(n, _, _, _)| n == "excluded_subtrees"));
    if !has_field {
        problems.push("`ObservationOwnership` has no `excluded_subtrees`: a prefix window cannot subtract an excluded nested location (CE4)".into());
    }
    let covers = p.defs("ObservationOwnership::covers");
    if covers.is_empty() {
        problems.push("`ObservationOwnership::covers` is missing".into());
    }
    for c in covers {
        if !p.funs[c].fields.iter().any(|x| x == "excluded_subtrees") {
            problems.push("`ObservationOwnership::covers` does not consult `excluded_subtrees`".into());
        }
    }
    verdict("coverage changes are not storage changes", problems)
}
