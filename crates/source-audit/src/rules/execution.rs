//! Execution-time guardrails: sinks recheck live state, protection has
//! one bidirectional predicate and fails closed, occupancy stays
//! tri-state, authorization is minted only where a human confirms, and
//! every subprocess is counted.

use super::{anchors, load, verdict};
use crate::program::{Fun, PCall, Program, contains_token, split_top_level};
use crate::resolve::{self, Honoured};
use std::collections::{HashMap, HashSet};
use std::path::Path;

// ---------------------------------------------------------------------
// execution_sinks_recheck_live_state
// ---------------------------------------------------------------------

/// The three live rechecks, and the helper that names what they covered.
const RECHECKS: [&str; 3] = [
    "recheck::reviewed_snapshot",
    "recheck::live_protection",
    "recheck::member_occupancy",
];
const COVERED: &str = "recheck::covered_paths";

/// Removing or moving an existing path: the calls that can lose user
/// data. (Writing a new file is `destructive` too, but a sink is about a
/// path that already exists.)
fn removal(c: &PCall) -> bool {
    !c.method
        && (c.is("fs::rename")
            || c.is("fs::remove_file")
            || c.is("fs::remove_dir")
            || c.is("fs::remove_dir_all")
            || c.path.starts_with("trash::"))
}

#[derive(Default, Clone, Copy)]
struct Have([bool; 3]);

impl Have {
    fn union(self, o: Have) -> Have {
        Have([
            self.0[0] || o.0[0],
            self.0[1] || o.0[1],
            self.0[2] || o.0[2],
        ])
    }
    fn complete(self) -> bool {
        self.0.iter().all(|b| *b)
    }
    fn missing(self) -> String {
        RECHECKS
            .iter()
            .zip(self.0)
            .filter(|(_, h)| !h)
            .map(|(r, _)| *r)
            .collect::<Vec<_>>()
            .join(" + ")
    }
}

fn honoured(c: &PCall) -> bool {
    c.honoured != Honoured::Discarded
}

/// Which rechecks a definition performs, honoured, transitively.
fn rechecks_of(
    p: &Program,
    g: usize,
    memo: &mut HashMap<usize, Have>,
    seen: &mut HashSet<usize>,
) -> Have {
    if let Some(h) = memo.get(&g) {
        return *h;
    }
    if !seen.insert(g) {
        return Have::default();
    }
    let f = &p.funs[g];
    let mut h = Have::default();
    for (ci, c) in f.calls.iter().enumerate() {
        for (k, r) in RECHECKS.iter().enumerate() {
            if honoured(c) && p.call_reaches_path(g, ci, r) {
                h.0[k] = true;
            }
        }
        if honoured(c) {
            for t in &p.target(g, ci).local {
                if *t != g {
                    h = h.union(rechecks_of(p, *t, memo, seen));
                }
            }
        }
    }
    memo.insert(g, h);
    h
}

/// Whether the path a removal names was handed to this function whole
/// (a parameter, a field of one, or plain bindings/iteration over
/// those) rather than constructed by it (joined from a literal,
/// computed by a local path helper that names one, or created here).
///
/// A caller-supplied path is user data and the removal is a sink. A
/// constructed path is swamp's own bookkeeping -- a temp file published
/// by rename, an expired delta, its own lock file. The whole-file skips
/// of `recheck.rs`, `store.rs` and `growth.rs` are gone: re-review 3 put
/// a bare `remove_dir_all(path)` in `growth.rs`.
pub(crate) fn caller_supplied(p: &Program, f: &Fun, expr: &str, depth: usize) -> bool {
    if depth > 8 {
        return true;
    }
    let text = expr.trim();
    if text.contains('"') || text.contains("format !") {
        return false;
    }
    // A call to a local path helper that constructs a path from a name
    // it chose (directly or through the helpers it calls).
    for c in &f.calls {
        if !c.method && text.contains(&format!("{} (", crate::program::spaced(&c.written))) {
            let local = p.resolve_path(f, &c.path).local;
            let helpers = p.reachable_exact(&local, &HashSet::new());
            if helpers.iter().any(|g| names_a_path(&p.funs[*g])) {
                return false;
            }
        }
    }
    let root = resolve::root_ident(text);
    if root.is_empty() {
        return true;
    }
    if root == "self" {
        let field = text
            .trim_start_matches('&')
            .trim()
            .strip_prefix("self")
            .unwrap_or("")
            .trim_start_matches([' ', '.'])
            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .next()
            .unwrap_or("")
            .to_string();
        return self_field_supplied(p, f, &field);
    }
    if f.params
        .iter()
        .any(|(pat, _)| resolve::root_ident(pat) == root)
    {
        return true;
    }
    match f.bindings.iter().rev().find(|b| b.name == root) {
        Some(b) => caller_supplied(p, f, &b.from, depth + 1),
        // A closure parameter or a destructured name this model cannot
        // trace: treat it as handed in.
        None => true,
    }
}

/// A function that builds a path from a name it chose: a literal joined,
/// formatted, given as an extension, or used to select directory entries.
fn names_a_path(f: &Fun) -> bool {
    let b = &f.body;
    [
        "join (\"",
        "with_extension (\"",
        "format !",
        "ends_with (\"",
        "starts_with (\"",
        "extension () == Some (\"",
    ]
    .iter()
    .any(|s| b.contains(s))
        || b.contains("== \"") && b.contains("extension")
}

/// `self.field`: owned when every struct literal of `Self` in the
/// program constructs that field from a path its own function built.
fn self_field_supplied(p: &Program, f: &Fun, field: &str) -> bool {
    let Some(ty) = &f.self_ty else { return true };
    let mut seen = false;
    for g in p.funs.iter() {
        for l in &g.struct_lits {
            if !resolve::path_ends_with(&l.path, ty)
                && !(l.path == "Self" && g.self_ty.as_deref() == Some(ty))
            {
                continue;
            }
            if let Some((_, init)) = l.fields.iter().find(|(n, _)| n == field) {
                seen = true;
                let init = if init.trim().is_empty() {
                    field
                } else {
                    init.as_str()
                };
                if caller_supplied(p, g, init, 1) {
                    return true;
                }
            }
        }
    }
    !seen
}

pub fn execution_sinks_recheck_live_state(root: &Path) -> Result<(), String> {
    let p = load(root);
    let mut problems = Vec::new();
    let mut all = RECHECKS.to_vec();
    all.push(COVERED);
    anchors(&p, &all, &mut problems);
    let mut memo: HashMap<usize, Have> = HashMap::new();
    for (i, f) in p.funs.iter().enumerate() {
        for (di, d) in f.calls.iter().enumerate() {
            if !removal(d) {
                continue;
            }
            let subject = d.args.first().cloned().unwrap_or_default();
            if !caller_supplied(&p, f, &subject, 0) {
                continue;
            }
            // Every recheck, honoured, before the removal: in this body or
            // in a helper called before it.
            let mut have = Have::default();
            let mut direct = Have::default();
            let mut subjects: Vec<String> = Vec::new();
            for (ci, c) in f.calls.iter().enumerate() {
                if c.stmt > d.stmt || ci == di || !honoured(c) {
                    continue;
                }
                for (k, r) in RECHECKS.iter().enumerate() {
                    if p.call_reaches_path(i, ci, r) {
                        have.0[k] = true;
                        direct.0[k] = true;
                        subjects.extend(c.args.iter().cloned());
                    }
                }
                if p.call_reaches_path(i, ci, COVERED) {
                    subjects.extend(c.args.iter().cloned());
                }
                for t in &p.target(i, ci).local {
                    if *t != i {
                        have = have.union(rechecks_of(&p, *t, &mut memo, &mut HashSet::new()));
                    }
                }
            }
            if !have.complete() {
                problems.push(format!(
                    "{} performs `{}` on caller-supplied `{}` without {} first (a recheck whose \
                     answer is discarded does not count)",
                    f.display(),
                    d.written,
                    subject.replace(' ', ""),
                    have.missing()
                ));
                continue;
            }
            // Rechecking one path and removing another is not a recheck.
            // When a helper did the rechecks, the helper is audited where
            // it lives.
            if !direct.complete() {
                continue;
            }
            let root_s = resolve::root_ident(&subject);
            let mut seeds: Vec<String> = subjects
                .join(" ")
                .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .filter(|t| !t.is_empty())
                .map(str::to_string)
                .collect();
            seeds.extend(
                f.bindings
                    .iter()
                    .filter(|b| {
                        all.iter()
                            .any(|r| b.from.replace(' ', "").contains(&r.replace(' ', "")))
                    })
                    .map(|b| b.name.clone()),
            );
            let tainted = resolve::derived_from(&f.bindings, &f.name, &seeds);
            if !root_s.is_empty() && !tainted.contains(&root_s) {
                problems.push(format!(
                    "{} performs `{}` on `{root_s}`, which none of its rechecks were about",
                    f.display(),
                    d.written
                ));
            }
        }
    }
    // Docker objects: the daemon's own re-derivation, honoured, before
    // the one destructive docker call.
    let sinks = p.defs("docker::remove");
    let recheck = p.defs("docker::still_removable");
    if sinks.is_empty() || recheck.is_empty() {
        problems.push("docker::remove / docker::still_removable are not defined".into());
    }
    for (i, f) in p.funs.iter().enumerate() {
        for (ci, c) in f.calls.iter().enumerate() {
            if c.method || !p.target(i, ci).local.iter().any(|g| sinks.contains(g)) {
                continue;
            }
            let ok = f.calls.iter().enumerate().any(|(ri, r)| {
                r.stmt <= c.stmt
                    && honoured(r)
                    && p.target(i, ri).local.iter().any(|g| recheck.contains(g))
            });
            if !ok {
                problems.push(format!(
                    "{} removes a Docker object without an honoured `docker::still_removable` first",
                    f.display()
                ));
            }
        }
    }
    // The destructive docker subprocess is built in exactly the sink.
    for (i, f) in p.funs.iter().enumerate() {
        let spawns_docker = f
            .calls
            .iter()
            .any(|c| spawn_site(c) && first_literal(&p, c).as_deref() == Some("docker"));
        let destructive = ["rm", "rmi", "prune"]
            .iter()
            .any(|v| f.literals.iter().any(|l| l == v));
        if spawns_docker && destructive && !sinks.contains(&i) {
            problems.push(format!(
                "{} builds a destructive `docker` subprocess; only `docker::remove` may, so every \
                 path to it is auditable",
                f.display()
            ));
        }
    }
    verdict(
        "every removal of a caller-supplied path rechecks identity, protection and occupancy, \
         honoured, before it (.oh/guardrails/execution-sinks-recheck-live-state.md)",
        problems,
    )
}

// ---------------------------------------------------------------------
// protection_fails_closed
// ---------------------------------------------------------------------

/// The shapes that turn a protection lookup's error into "nothing is
/// protected": `Result` combinators that substitute a value.
fn discards_error(method: &str) -> bool {
    matches!(
        method,
        "unwrap_or_default" | "unwrap_or" | "unwrap_or_else" | "ok"
    )
}

pub fn protection_fails_closed(root: &Path) -> Result<(), String> {
    let p = load(root);
    let mut problems = Vec::new();
    let predicate = anchors(&p, &["agents::protection_conflict"], &mut problems);
    let loaders = anchors(&p, &["agents::load_protect"], &mut problems);
    let writer = anchors(&p, &["agents::write_atomic"], &mut problems);

    // 1. The predicate tests containment in both directions: two
    //    `starts_with` calls whose receiver and argument are swapped.
    for i in &predicate {
        let f = &p.funs[*i];
        let pairs: Vec<(String, String)> = f
            .calls
            .iter()
            .filter(|c| c.method && c.path == "starts_with")
            .map(|c| {
                (
                    resolve::root_ident(&c.receiver),
                    c.args
                        .first()
                        .map(|a| resolve::root_ident(a))
                        .unwrap_or_default(),
                )
            })
            .filter(|(r, a)| !r.is_empty() && !a.is_empty())
            .collect();
        let both = pairs
            .iter()
            .any(|(r, a)| pairs.iter().any(|(r2, a2)| r2 == a && a2 == r));
        if !both {
            problems.push(format!(
                "{} tests protection in only one direction: protecting `debug/log.txt` must also \
                 stop removing `debug/`",
                f.display()
            ));
        }
    }

    // What returns the protect list, and what persists it (a function
    // taking the list that reaches a write).
    let list_type: Vec<String> = loaders
        .iter()
        .map(|i| p.funs[*i].ret.replace(' ', ""))
        .collect();
    let mutating = p.mutating();
    // A persister: something a function that loads the list calls, in the
    // list's own module, handing it a path list, that writes.
    let load_callers: HashSet<usize> = loaders.iter().flat_map(|l| p.callers(*l)).collect();
    let persisters: HashSet<usize> = p
        .funs
        .iter()
        .enumerate()
        .filter(|(i, f)| {
            loaders.iter().any(|l| p.funs[*l].rel == f.rel)
                && load_callers.iter().any(|c| p.callees(*c).contains(i))
                && f.params
                    .iter()
                    .any(|(_, t)| t.replace(' ', "").contains("PathBuf]"))
                && mutating.contains(i)
        })
        .map(|(i, _)| i)
        .collect();
    // 2. Every persister writes through the atomic temp+rename writer.
    for i in &persisters {
        let reaches_writer = !p.reachable(&[*i], &HashSet::new()).is_disjoint(&writer);
        let bare = p.funs[*i]
            .calls
            .iter()
            .any(|c| !c.method && (c.is("fs::write") || c.is("File::create")));
        if !reaches_writer || bare {
            problems.push(format!(
                "{} persists the protect list without `write_atomic` (temp file + rename): a crash \
                 mid-write would publish an empty keep list",
                p.funs[*i].display()
            ));
        }
    }
    // 3. One predicate, everywhere: whatever loads the list and does not
    //    manage it (return it, or persist it) delegates to the predicate.
    for (i, f) in p.funs.iter().enumerate() {
        if loaders.contains(&i) || predicate.contains(&i) {
            continue;
        }
        let loads = f
            .calls
            .iter()
            .enumerate()
            .any(|(ci, _)| p.target(i, ci).local.iter().any(|g| loaders.contains(g)));
        if !loads {
            continue;
        }
        let returns_list = list_type.iter().any(|t| f.ret.replace(' ', "") == *t);
        let manages = returns_list || p.callees(i).iter().any(|g| persisters.contains(g));
        let delegates = !p.reachable(&[i], &HashSet::new()).is_disjoint(&predicate);
        if !manages && !delegates {
            problems.push(format!(
                "{} loads the protect list and decides for itself instead of calling \
                 `agents::protection_conflict`",
                f.display()
            ));
        }
        // 4. No caller turns the lookup's error into an empty list.
        for c in &f.calls {
            if c.method
                && discards_error(&c.path)
                && (c.receiver.contains("load_protect")
                    || c.receiver.contains("protect_list")
                    || f.calls.iter().any(|l| {
                        !l.method
                            && l.callee() == resolve::root_ident(&c.receiver)
                            && l.is("agents::load_protect")
                    }))
            {
                problems.push(format!(
                    "{} discards the protection lookup's error with `.{}()`: unreadable protection \
                     state is unknown, never an empty keep list",
                    f.display(),
                    c.path
                ));
            }
        }
    }
    // Aliased loaders (`use agents::load_protect as read_keep_list`) are
    // resolved calls too: check every call that lands on a loader.
    for (i, f) in p.funs.iter().enumerate() {
        for (ci, c) in f.calls.iter().enumerate() {
            if !p.target(i, ci).local.iter().any(|g| loaders.contains(g)) {
                continue;
            }
            let spelled = format!("{} (", crate::program::spaced(&c.written));
            for m in &f.calls {
                if m.method
                    && discards_error(&m.path)
                    && m.receiver.contains(spelled.trim_end_matches(" ("))
                {
                    problems.push(format!(
                        "{} discards the protection lookup's error with `.{}()`",
                        f.display(),
                        m.path
                    ));
                }
            }
            if c.honoured == Honoured::Discarded {
                problems.push(format!(
                    "{} loads the protect list and ignores the answer",
                    f.display()
                ));
            }
        }
    }
    // 5. In the modules that decide actions (they define an
    //    `execute*`/`propose*` entry point), no second containment
    //    predicate: a verdict-returning function that tests path
    //    containment must be, or delegate to, the one predicate.
    let decision_files: HashSet<String> = p.family(
        &p.funs
            .iter()
            .filter(|f| {
                f.is_pub
                    && f.self_ty.is_none()
                    && (f.name.starts_with("execute") || f.name.starts_with("propose"))
            })
            .map(|f| f.rel.clone())
            .collect(),
    );
    for (i, f) in p.funs.iter().enumerate() {
        if !decision_files.contains(&f.rel) || predicate.contains(&i) || !f.returns_verdict() {
            continue;
        }
        let contains_test = f.calls.iter().any(|c| {
            c.method && c.path == "starts_with" && c.args.first().is_some_and(|a| !a.contains('"'))
        });
        let delegates = !p.reachable(&[i], &HashSet::new()).is_disjoint(&predicate);
        if contains_test && !delegates {
            problems.push(format!(
                "{} answers a path-containment question in an action-deciding module without \
                 delegating to `agents::protection_conflict`: one predicate, everywhere",
                f.display()
            ));
        }
    }
    verdict(
        "protection has one bidirectional predicate, is written atomically, and fails closed",
        problems,
    )
}

// ---------------------------------------------------------------------
// occupancy_is_tristate_at_sinks
// ---------------------------------------------------------------------

/// An arm body that refuses: it diverges, returns an error, or turns the
/// unknown into an explicit unknown/unavailable fact.
fn refuses(body: &str) -> bool {
    let b = body.replace(' ', "");
    [
        "bail!", "Err(", "return", "continue", "break", "panic!", "anyhow!",
    ]
    .iter()
    .any(|r| b.contains(r))
        || b.contains("Evidence::unavailable")
        || b.contains("Evidence::unknown")
        || b.to_ascii_lowercase().contains("refus")
}

pub fn occupancy_is_tristate_at_sinks(root: &Path) -> Result<(), String> {
    let p = load(root);
    let mut problems = Vec::new();
    let state = p.types.iter().find(|t| t.name == "OccupancyState");
    match state {
        Some(t) => {
            for v in ["Free", "Occupied", "Unknown"] {
                if !t.variants.iter().any(|x| x == v) {
                    problems.push(format!("`OccupancyState` has no `{v}` variant"));
                }
            }
        }
        None => problems.push("`OccupancyState` is not defined".into()),
    }
    let tri = anchors(&p, &["recheck::member_occupancy"], &mut problems);
    // Boolean occupancy: a `bool` answer derived from the tri-state.
    let boolean: HashSet<usize> = p
        .funs
        .iter()
        .enumerate()
        .filter(|(_, f)| {
            f.ret.trim() == "bool"
                && (contains_token(&f.body, "OccupancyState")
                    || f.calls.iter().any(|c| c.is("occupancy::probe_path")))
        })
        .map(|(i, _)| i)
        .collect();
    for (i, f) in p.funs.iter().enumerate() {
        for (ci, c) in f.calls.iter().enumerate() {
            let t = p.target(i, ci);
            if !t.possible && !boolean.contains(&i) && t.local.iter().any(|g| boolean.contains(g)) {
                problems.push(format!(
                    "{} consumes the boolean occupancy answer `{}`: every consumer uses \
                     `recheck::member_occupancy`, whose `Unknown` refuses",
                    f.display(),
                    c.written
                ));
            }
            if c.honoured == Honoured::Discarded && t.local.iter().any(|g| tri.contains(g)) {
                problems.push(format!(
                    "{} discards the result of `member_occupancy`",
                    f.display()
                ));
            }
        }
        // Match arms: an `Unknown` arm refuses; a wildcard that covers an
        // unnamed `Unknown` refuses too.
        let mut by_scrutinee: HashMap<&str, Vec<&resolve::MatchArm>> = HashMap::new();
        for a in &f.arms {
            by_scrutinee
                .entry(a.scrutinee.as_str())
                .or_default()
                .push(a);
        }
        for (_, arms) in by_scrutinee {
            let about = arms.iter().any(|a| a.pattern.contains("OccupancyState"))
                || arms.iter().any(|a| {
                    a.scrutinee.contains("member_occupancy") || a.scrutinee.contains("probe_path")
                });
            if !about {
                continue;
            }
            let names_unknown = arms.iter().any(|a| a.pattern.contains("Unknown"));
            for a in &arms {
                let covers_unknown = a.pattern.contains("Unknown")
                    || (!names_unknown
                        && !a.pattern.contains("Free")
                        && !a.pattern.contains("Occupied"));
                if covers_unknown && !refuses(&a.body) {
                    problems.push(format!(
                        "{}: the arm `{} => {}` lets an unanswerable occupancy probe through",
                        f.display(),
                        a.pattern,
                        a.body.replace('\n', " ")
                    ));
                }
            }
        }
        // `matches!`/`==`/`if let` that name `Occupied` and not `Unknown`
        // put `Unknown` on the `Free` side.
        for m in &f.macros {
            if m.name == "matches" {
                let parts = split_top_level(&m.tokens);
                let pattern = parts.get(1..).map(|x| x.join(",")).unwrap_or_default();
                if pattern.contains("Occupied") && !pattern.contains("Unknown") {
                    problems.push(format!(
                        "{}: `matches!({})` collapses the tri-state -- `Unknown` is treated as free",
                        f.display(),
                        m.tokens
                    ));
                }
            }
        }
        let body = f.body.replace(' ', "");
        if body.contains("==OccupancyState::Occupied")
            || body.contains("!=OccupancyState::Occupied")
            || body.contains("::OccupancyState::Occupied(_)==")
        {
            problems.push(format!(
                "{} compares against `Occupied`, collapsing `Unknown` into free",
                f.display()
            ));
        }
        let mut rest = body.as_str();
        while let Some(at) = rest.find("iflet") {
            let tail = &rest[at + 5..];
            let pat = tail.split('=').next().unwrap_or("");
            if pat.contains("Occupied") && !pat.contains("Unknown") {
                problems.push(format!(
                    "{}: `if let ..Occupied.. = ..` proceeds on `Unknown`",
                    f.display()
                ));
            }
            rest = tail;
        }
    }
    verdict(
        "occupancy stays tri-state at every consumer: an unanswerable probe refuses, never reads \
         as free",
        problems,
    )
}

// ---------------------------------------------------------------------
// human_only_authorization
// ---------------------------------------------------------------------

/// The three functions that mint or change authorization.
const AUTH_SINKS: &[&str] = &[
    "actions::approve",
    "actions::add_standing_grant",
    "actions::revoke_grant",
];

/// `(file, function)`: the reviewed call sites. The CLI's own
/// approve/grant subcommands are factored into these so the list can
/// name them; the TUI's confirmed-execution path is `execute_one`,
/// reached only after its own confirm prompt.
pub const AUTH_ALLOWED_CALLERS: &[(&str, &str)] = &[
    ("crates/cli/src/main.rs", "cmd_approve"),
    ("crates/cli/src/main.rs", "cmd_grant_add"),
    ("crates/cli/src/main.rs", "cmd_grant_revoke"),
    ("crates/tui/src/actions.rs", "execute_one"),
];

pub fn human_only_authorization(root: &Path) -> Result<(), String> {
    let p = load(root);
    let mut problems = Vec::new();
    let sinks = anchors(&p, AUTH_SINKS, &mut problems);
    let sink_files: HashSet<String> = sinks.iter().map(|i| p.funs[*i].rel.clone()).collect();
    for (i, f) in p.funs.iter().enumerate() {
        // The sinks' own module: `approve` calling its internal grant
        // writer is wiring, not a new minting site.
        if sink_files.contains(&f.rel) {
            continue;
        }
        let hits: Vec<String> = p
            .callees(i)
            .iter()
            .filter(|g| sinks.contains(g))
            .map(|g| p.funs[*g].name.clone())
            .collect();
        if hits.is_empty() {
            continue;
        }
        let allowed = AUTH_ALLOWED_CALLERS
            .iter()
            .any(|(rel, name)| *rel == f.rel && *name == f.name);
        if !allowed {
            problems.push(format!(
                "{} reaches authorization-minting {hits:?} outside the reviewed CLI approve/grant \
                 handlers and the TUI confirmation path",
                f.display()
            ));
        }
    }
    for (rel, name) in AUTH_ALLOWED_CALLERS {
        if !p.funs.iter().any(|f| f.rel == *rel && f.name == *name) {
            problems.push(format!(
                "the reviewed caller {rel}::{name} no longer exists; remove it from the list"
            ));
        }
    }
    verdict(
        "authorization is minted only from the reviewed human-confirmation call sites \
         (.oh/guardrails/human-only-authorization.md)",
        problems,
    )
}

// ---------------------------------------------------------------------
// every_spawn_is_counted
// ---------------------------------------------------------------------

/// A subprocess constructor: the standard library's, or the one wrapper.
pub fn spawn_site(c: &PCall) -> bool {
    !c.method && (c.is("Command::new") || c.is("spawn::command"))
}

/// The program a spawn site names, when it is a literal or a constant.
pub fn first_literal(p: &Program, c: &PCall) -> Option<String> {
    c.args.first().and_then(|a| p.eval_literal(a))
}

pub fn every_spawn_is_counted(root: &Path) -> Result<(), String> {
    let p = load(root);
    let mut problems = Vec::new();
    let wrapper = anchors(&p, &["spawn::command"], &mut problems);
    for w in &wrapper {
        let f = &p.funs[*w];
        let record = f.calls.iter().find(|c| c.is("work_counters::record_spawn"));
        let build = f
            .calls
            .iter()
            .find(|c| !c.method && c.is("process::Command::new"));
        match (record, build) {
            (Some(r), Some(b)) if r.stmt <= b.stmt => {}
            _ => problems.push(format!(
                "{} must count the spawn (`work_counters::record_spawn`) and then build the \
                 `std::process::Command` it returns",
                f.display()
            )),
        }
    }
    for (i, f) in p.funs.iter().enumerate() {
        if wrapper.contains(&i) {
            continue;
        }
        for c in &f.calls {
            if !c.method && c.is("Command::new") {
                problems.push(format!(
                    "{} builds a `Command` directly (`{}`): `spawn::command` is the only \
                     constructor, so every spawn is counted",
                    f.display(),
                    c.written
                ));
            }
        }
        for r in &f.refs {
            if resolve::path_ends_with(r, "Command::new") {
                problems.push(format!("{} names `Command::new` as a value", f.display()));
            }
        }
    }
    verdict(
        "every subprocess is built by `spawn::command`, which counts it: the spawn counter is \
         structural, not a habit (.oh/guardrails/every-spawn-is-counted.md)",
        problems,
    )
}
