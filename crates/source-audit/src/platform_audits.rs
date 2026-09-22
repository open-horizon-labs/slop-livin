//! Platform-capability audits (#79), written to the model re-review 3
//! demands (`review/REVIEW-STACK-3.md` §1): derived entity sets over the
//! whole-workspace resolved call graph, behaviour rather than names, and
//! transitive reachability rather than one function's own body.
//!
//! The invariant, in one sentence: **a capability this build does not
//! have is refused, and the refusal happens before anything is
//! written.** Everything below is an attempt to state that so it cannot
//! be walked around.
//!
//! The five structural causes re-review 3 found, and what each gets:
//!
//! 1. *Hand-written file lists.* No rule names a file. The workspace
//!    (`resolve::workspace_files`: all of `crates/{core,cli,tui}/src`) is
//!    parsed once into a model of definitions, and call edges resolve to
//!    *definitions* (file + name) rather than to names, so a same-named
//!    function elsewhere neither lends its guard nor hides the one under
//!    audit. The platform module is found by module path, not by a
//!    literal file path.
//! 2. *Hand-written name lists.* The two lists that remain are
//!    **inverted**, so an omission fails closed rather than open. A
//!    `std::fs::` call is a mutation unless it is one of the handful of
//!    reads; a `platform` function is a capability query if its return
//!    type is a capability enum, whatever it is called.
//! 3. *A name checked, a behaviour not.* Every capability query anywhere
//!    in the workspace must be `Honoured`; `Discarded` fails. Asking is
//!    not refusing.
//! 4. *Only the named function is read.* The guard rule walks the
//!    transitive callee set, so moving the writes into a helper does not
//!    move them out of the rule. The entry points are derived from the
//!    CLI's own `match` on its subcommand enum, so renaming `install` or
//!    adding a second entry point is picked up.
//! 5. *Syntax the layer cannot see.* Bodies are read through the
//!    resolver, which expands the known-arg macros and resolves aliases
//!    before any rule reads a token.
//!
//! What it still cannot see -- trait-object dispatch, function
//! pointers, and a writing helper that some *other* subcommand also
//! calls (the subtraction that keeps shared code out of the rule) -- is
//! in the guardrail's Limits section, with the runtime tests that cover
//! those cases named there. Target gating of `Os::current` and the
//! `CAPABILITIES` table are not text checks here: CI's
//! `scripts/platform-isolation.sh` proves the gating on the built binary's
//! symbols, and `platform_matrix_matches_docs` proves the table.

use crate::ast;
use crate::resolve::{self, CallSite, Honoured};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// The capability enums. A `platform` function returning one of these is
/// a capability query, whatever it is named -- the derivation that
/// replaces a list of function names.
const CAPABILITY_TYPES: &[&str] = &["Scheduling", "ContinuitySource", "Support", "Capability"];

/// `std::fs` functions that only read. **Everything else under
/// `std::fs::` is a mutation**, so a function nobody thought of fails
/// closed instead of silently joining the allowed set. That inversion is
/// the answer to re-review 3's cause 2: `OpenOptions::truncate` destroys
/// what `fs::write` destroys, and the list did not have it.
const FS_READS: &[&str] = &[
    "read",
    "read_to_string",
    "read_dir",
    "read_link",
    "metadata",
    "symlink_metadata",
    "canonicalize",
    "exists",
    "try_exists",
];

/// Non-`std::fs` ways to put state on disk or start something, checked
/// by resolved suffix. Short on purpose: it stands beside the inverted
/// `std::fs` rule rather than carrying the weight alone.
const OTHER_MUTATORS: &[&str] = &[
    "File::create",
    "File::create_new",
    "File::options",
    "OpenOptions::new",
    "std::process::Command::new",
];

pub fn platform_capabilities_gate_their_backends(root: &Path) -> Result<(), String> {
    let ws = Workspace::load(root);
    let queries = capability_queries(&ws)?;
    every_capability_answer_is_honoured(&ws, &queries)?;
    state_installing_paths_are_guarded_first(&ws, &queries)?;
    the_platform_refusal_is_decided_once(&ws)?;
    Ok(())
}

// ---------------------------------------------------------------------
// The program model: every definition, parsed once, with call edges
// resolved to definitions rather than to names
// ---------------------------------------------------------------------

/// One function definition: the file it lives in and its name.
///
/// Keyed by *definition*, not by name. The first version of this audit
/// keyed its call graph by last path segment, reasoning that collapsing
/// two same-named functions over-approximates and so fails closed. It
/// does the opposite wherever the graph is *subtracted*:
/// `work_counters::install` is called by every other subcommand, so the
/// name `install` landed in the "shared" set and `schedule::install` --
/// the one function this rule exists to read -- was never walked. Four
/// of the corpus's installers passed that way.
pub(crate) type Node = (String, String);

pub(crate) struct FileModel {
    pub(crate) rel: String,
    /// The module path inside the crate (`["platform"]` for
    /// `platform/mod.rs`, `[]` for `lib.rs`/`main.rs`).
    pub(crate) mods: Vec<String>,
    pub(crate) funcs: Vec<ast::Func>,
    pub(crate) arms: Vec<resolve::MatchArm>,
    /// Every call, test code included (the dispatch derivation filters).
    pub(crate) all_calls: Vec<CallSite>,
}

/// Every file under `crates/{core,cli,tui}/src`, parsed and resolved
/// **once**. The three rules read the same derived sets; building them
/// per rule parsed the workspace five times.
pub(crate) struct Workspace {
    pub(crate) files: Vec<FileModel>,
    /// Production calls only, tagged with the calling file.
    pub(crate) calls: Vec<(String, CallSite)>,
    /// Function name -> every definition with that name.
    by_name: BTreeMap<String, Vec<Node>>,
    /// File -> (crate directory, module path).
    modules: BTreeMap<String, (String, Vec<String>)>,
}

fn module_of(rel: &str) -> (String, Vec<String>) {
    let mut parts = rel.split('/');
    let krate = parts.nth(1).unwrap_or_default().to_string();
    let rest: Vec<&str> = parts.skip(1).collect();
    let mut mods: Vec<String> = rest
        .iter()
        .map(|s| s.trim_end_matches(".rs").to_string())
        .collect();
    if mods
        .last()
        .is_some_and(|m| ["mod", "lib", "main"].contains(&m.as_str()))
    {
        mods.pop();
    }
    (krate, mods)
}

/// The crate directory an external crate name refers to, for the three
/// product crates only; anything else (`std`, `anyhow`, ...) is not a
/// definition this model has.
fn crate_dir(head: &str) -> Option<&'static str> {
    match head {
        "swamp_core" => Some("core"),
        "swamp_tui" => Some("tui"),
        "swamp" => Some("cli"),
        _ => None,
    }
}

impl Workspace {
    pub(crate) fn load(root: &Path) -> Self {
        let mut files = Vec::new();
        let mut calls = Vec::new();
        let mut by_name: BTreeMap<String, Vec<Node>> = BTreeMap::new();
        let mut modules = BTreeMap::new();
        for rel in resolve::workspace_files(root) {
            let Some(parsed) = resolve::maybe(root, &rel) else {
                continue;
            };
            let all_calls = resolve::calls(&parsed.ast);
            for c in &all_calls {
                if !c.in_test && !c.dead_code_allowed {
                    calls.push((rel.clone(), c.clone()));
                }
            }
            let funcs = ast::functions(&parsed.ast);
            for f in &funcs {
                let defs = by_name.entry(f.name.clone()).or_default();
                let node = (rel.clone(), f.name.clone());
                if !defs.contains(&node) {
                    defs.push(node);
                }
            }
            let (krate, mods) = module_of(&rel);
            modules.insert(rel.clone(), (krate, mods.clone()));
            files.push(FileModel {
                arms: resolve::match_arms(&parsed.ast),
                funcs,
                all_calls,
                mods,
                rel,
            });
        }
        Workspace {
            files,
            calls,
            by_name,
            modules,
        }
    }

    /// The definitions a call in `caller` may reach.
    ///
    /// Exact where the resolved path says which module it means;
    /// every same-named candidate where it cannot (a method call, a glob
    /// import, a type whose impl the path does not locate). Callers pick
    /// which of the two they can afford: a walk that must reach every
    /// write takes all candidates, a set that is *subtracted* takes only
    /// exact ones.
    pub(crate) fn targets(&self, caller: &str, c: &CallSite) -> Vec<Node> {
        let name = last_segment(&c.path);
        let Some(cands) = self.by_name.get(name) else {
            return Vec::new();
        };
        if c.method {
            return cands.clone();
        }
        let Some((ck, cm)) = self.modules.get(caller) else {
            return cands.clone();
        };
        let segs: Vec<&str> = c.path.split("::").filter(|s| !s.is_empty()).collect();
        let same_crate = || -> Vec<Node> {
            cands
                .iter()
                .filter(|(r, _)| self.modules.get(r).is_some_and(|(k, _)| k == ck))
                .cloned()
                .collect()
        };
        if segs.len() <= 1 {
            if cands.iter().any(|(r, _)| r == caller) {
                return vec![(caller.to_string(), name.to_string())];
            }
            // A glob import or a nested module: this file cannot say
            // which one, so every definition in the crate.
            return same_crate();
        }
        let mut qual: Vec<String> = segs[..segs.len() - 1]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (krate, base): (Option<String>, Vec<String>) = match qual[0].as_str() {
            "crate" => {
                qual.remove(0);
                (Some(ck.clone()), Vec::new())
            }
            "self" => {
                qual.remove(0);
                (Some(ck.clone()), cm.clone())
            }
            "super" => {
                let mut b = cm.clone();
                while qual.first().is_some_and(|s| s == "super") {
                    qual.remove(0);
                    b.pop();
                }
                (Some(ck.clone()), b)
            }
            "Self" => {
                return cands.iter().filter(|(r, _)| r == caller).cloned().collect();
            }
            head => match crate_dir(head) {
                Some(k) => {
                    qual.remove(0);
                    (Some(k.to_string()), Vec::new())
                }
                None => (None, Vec::new()),
            },
        };
        // `platform::Scheduling::for_os`: trailing type segments name an
        // impl, not a module.
        while qual
            .last()
            .is_some_and(|s| s.starts_with(char::is_uppercase))
        {
            qual.pop();
        }
        let want: Vec<String> = base.into_iter().chain(qual).collect();
        let found: Vec<Node> = cands
            .iter()
            .filter(|(r, _)| {
                let Some((k, m)) = self.modules.get(r) else {
                    return false;
                };
                krate.as_ref().is_none_or(|kk| k == kk)
                    && m.len() >= want.len()
                    && m[m.len() - want.len()..] == want[..]
            })
            .cloned()
            .collect();
        if !found.is_empty() {
            return found;
        }
        match krate {
            // A product-crate path the module layout does not explain (a
            // re-export): every definition in that crate.
            Some(k) => cands
                .iter()
                .filter(|(r, _)| self.modules.get(r).is_some_and(|(kk, _)| *kk == k))
                .cloned()
                .collect(),
            // `PathBuf::from`, `Type::new`: a type the file imported.
            None if want.is_empty() => same_crate(),
            // `std::fs::write`: not a definition in this workspace.
            None => Vec::new(),
        }
    }

    /// The one definition a call reaches, when it is unambiguous.
    pub(crate) fn exact_target(&self, caller: &str, c: &CallSite) -> Option<Node> {
        let t = self.targets(caller, c);
        (t.len() == 1).then(|| t[0].clone())
    }
}

// ---------------------------------------------------------------------
// Derivation: what counts as asking the platform
// ---------------------------------------------------------------------

/// Every definition whose *return type* is a capability enum.
///
/// A renamed query, a second query, or a query added in a new submodule
/// joins the set without anyone editing this file -- which is the
/// difference between a rule and a snapshot of one afternoon.
fn capability_queries(ws: &Workspace) -> Result<BTreeSet<Node>, String> {
    if !ws
        .files
        .iter()
        .any(|f| f.mods.first().is_some_and(|m| m == "platform"))
    {
        return Err(
            "no platform module under crates/*/src: there is no capability contract for \
             anything to be checked against"
                .into(),
        );
    }
    // Whole-workspace, not just the platform module: a wrapper like
    // `schedule::scheduling()` that returns a capability enum *is* a
    // capability query, and a rule that only looked inside `platform/`
    // would miss every call to it -- which is the shape of re-review
    // 3's cause 1 applied to this contract.
    let mut out = BTreeSet::new();
    for file in &ws.files {
        for f in &file.funcs {
            // `sig` carries the declaration's tokens including the
            // return type, so this reads the signature, not the name.
            let Some((_, ret)) = f.sig.split_once("->") else {
                continue;
            };
            let ret_idents: Vec<&str> = ret
                .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .filter(|t| !t.is_empty())
                .collect();
            if CAPABILITY_TYPES.iter().any(|t| ret_idents.contains(t)) {
                out.insert((file.rel.clone(), f.name.clone()));
            }
        }
    }
    if out.is_empty() {
        return Err(format!(
            "no function anywhere in the workspace returns a capability type ({}): nothing can \
             ask what this build is able to do",
            CAPABILITY_TYPES.join("/")
        ));
    }
    Ok(out)
}

pub(crate) fn last_segment(path: &str) -> &str {
    path.rsplit("::").next().unwrap_or(path)
}

/// A call that puts state on disk, or starts a process. Inverted for
/// `std::fs`: anything under it that is not a known read counts.
fn is_mutator(c: &CallSite) -> bool {
    if let Some(rest) = c.path.strip_prefix("std::fs::") {
        return !FS_READS.contains(&last_segment(rest));
    }
    OTHER_MUTATORS
        .iter()
        .any(|m| resolve::path_ends_with(&c.path, m))
}

// ---------------------------------------------------------------------
// Rule 1: asking is not refusing
// ---------------------------------------------------------------------

/// Every capability query anywhere in the workspace flows into control
/// flow. `let _ = scheduling();` is re-review 3's cause 3 applied to
/// this contract: the call is there and the answer is not.
///
/// Reads every candidate a call may reach, so an ambiguous call to a
/// query is held to the rule rather than excused by the ambiguity.
fn every_capability_answer_is_honoured(
    ws: &Workspace,
    queries: &BTreeSet<Node>,
) -> Result<(), String> {
    for (rel, c) in &ws.calls {
        if c.honoured != Honoured::Discarded {
            continue;
        }
        if ws.targets(rel, c).iter().any(|t| queries.contains(t)) {
            return Err(format!(
                "{rel}::{}: asks the platform (`{}`) and discards the answer. Asking is not \
                 refusing -- the capability is still used and the user is told nothing.",
                c.func, c.written
            ));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------
// Rule 2: the guard comes first, through helpers too
// ---------------------------------------------------------------------

/// What the scheduling feature owns, derived from the CLI's own
/// dispatch rather than named here.
///
/// Every `match` arm in the workspace whose pattern names a
/// `Schedule`-shaped subcommand contributes the definitions its body
/// calls; every *other* subcommand arm contributes the definitions it
/// calls. The scheduling feature is the first closure minus the second,
/// which is how a genuinely shared helper stays out of a rule about
/// installing a scheduled job -- `resolve_scope` spawns a detector's
/// allow-listed tool query on every subcommand, and is not an installer.
///
/// The subtracted side uses **exact** edges only: an ambiguous call
/// there would widen "shared" and so remove code from the rule. The
/// walked side uses every candidate. Both choices fail closed.
///
/// Renaming `install`, or adding a second entry point beside it, joins
/// the set automatically. Deleting the subcommand makes the rule say so
/// rather than pass vacuously.
struct SchedulingFeature {
    /// Direct callees of the `Schedule` arms: where the walk starts.
    front_doors: BTreeSet<Node>,
    /// Everything reachable from those and from no other subcommand.
    owned: BTreeSet<Node>,
}

pub(crate) type Graph = BTreeMap<Node, BTreeSet<Node>>;

pub(crate) fn closure(seed: &BTreeSet<Node>, graph: &Graph) -> BTreeSet<Node> {
    let mut seen = seed.clone();
    let mut queue: Vec<Node> = seed.iter().cloned().collect();
    while let Some(f) = queue.pop() {
        for callee in graph.get(&f).into_iter().flatten() {
            if seen.insert(callee.clone()) {
                queue.push(callee.clone());
            }
        }
    }
    seen
}

/// `definition -> definitions it may call`, production code only.
/// `exact_only` selects which of the two edge sets (see
/// [`SchedulingFeature`]).
pub(crate) fn call_graph(ws: &Workspace, exact_only: bool) -> Graph {
    let mut g: Graph = BTreeMap::new();
    for (rel, c) in &ws.calls {
        if c.func.is_empty() {
            continue;
        }
        let from = (rel.clone(), c.func.clone());
        let to: Vec<Node> = if exact_only {
            ws.exact_target(rel, c).into_iter().collect()
        } else {
            ws.targets(rel, c)
        };
        g.entry(from).or_default().extend(to);
    }
    g
}

fn scheduling_feature(
    ws: &Workspace,
    walk: &Graph,
    exact: &Graph,
) -> Result<SchedulingFeature, String> {
    let mut schedule_seed = BTreeSet::new();
    let mut other_seed = BTreeSet::new();
    let mut saw_schedule_arm = false;

    for file in &ws.files {
        // The subcommand dispatch only: an arm whose pattern names the
        // CLI's own command enum.
        let dispatch: Vec<&resolve::MatchArm> = file
            .arms
            .iter()
            .filter(|a| a.pattern.contains("Command ::") || a.pattern.contains("Command::"))
            .collect();
        for arm in dispatch {
            let is_schedule = arm.pattern.contains("Schedule");
            saw_schedule_arm |= is_schedule;
            for c in &file.all_calls {
                if c.in_test || c.func != arm.func || !arm.body.contains(&c.written) {
                    continue;
                }
                if is_schedule {
                    schedule_seed.extend(ws.targets(&file.rel, c));
                } else {
                    other_seed.extend(ws.exact_target(&file.rel, c));
                }
            }
        }
    }

    if !saw_schedule_arm {
        return Err(
            "no `Schedule` subcommand arm anywhere in the workspace: either the scheduled \
             observation is gone, or its dispatch moved somewhere this rule cannot derive it \
             from. Both need a human."
                .into(),
        );
    }

    let shared = closure(&other_seed, exact);
    let front_doors: BTreeSet<Node> = schedule_seed.difference(&shared).cloned().collect();
    if front_doors.is_empty() {
        return Err(
            "the `Schedule` arm calls nothing that is not also called by another subcommand: \
             the scheduling feature has no code of its own for this rule to guard"
                .into(),
        );
    }
    let owned: BTreeSet<Node> = closure(&front_doors, walk)
        .difference(&shared)
        .cloned()
        .collect();
    Ok(SchedulingFeature { front_doors, owned })
}

/// Where one definition stands relative to the guard.
enum Guarded {
    /// Asks, honours the answer, and asks before it writes. Calls made
    /// *after* the asking statement are behind the guard; calls made
    /// before it are not, and the walk continues into those.
    Yes { asked_at: usize },
    /// Asks, and writes first. A refusal after a write is not a refusal.
    TooLate {
        asked_at: usize,
        wrote_at: usize,
        what: String,
    },
    /// Does not ask. The walk continues through it.
    No,
}

pub(crate) fn show(n: &Node) -> String {
    format!("{}::{}", n.0, n.1)
}

/// Every path from a scheduling front door to something that writes
/// passes through an honoured capability check first.
///
/// The transitive form re-review 3's cause 4 asks for: the guard may sit
/// in the entry point or in any function between it and the write, and
/// moving the writes into a helper moves them no further out of reach.
/// A guard only counts when the call reaches a query **exactly**: an
/// ambiguous call that might be a query is not evidence of asking.
fn state_installing_paths_are_guarded_first(
    ws: &Workspace,
    queries: &BTreeSet<Node>,
) -> Result<(), String> {
    let walk = call_graph(ws, false);
    let exact = call_graph(ws, true);
    let feature = scheduling_feature(ws, &walk, &exact)?;

    let mut by_node: BTreeMap<Node, Vec<(&str, &CallSite)>> = BTreeMap::new();
    for (rel, c) in &ws.calls {
        if !c.func.is_empty() {
            by_node
                .entry((rel.clone(), c.func.clone()))
                .or_default()
                .push((rel.as_str(), c));
        }
    }

    let guard_state = |node: &Node| -> Guarded {
        let Some(sites) = by_node.get(node) else {
            return Guarded::No;
        };
        let asked_at = sites
            .iter()
            .filter(|(rel, c)| {
                c.honoured != Honoured::Discarded
                    && ws
                        .exact_target(rel, c)
                        .is_some_and(|t| queries.contains(&t))
            })
            .map(|(_, c)| c.stmt)
            .min();
        let Some(asked_at) = asked_at else {
            return Guarded::No;
        };
        match sites
            .iter()
            .filter(|(_, c)| is_mutator(c))
            .map(|(_, c)| (c.stmt, c.written.clone()))
            .min()
        {
            Some((wrote_at, what)) if wrote_at < asked_at => Guarded::TooLate {
                asked_at,
                wrote_at,
                what,
            },
            _ => Guarded::Yes { asked_at },
        }
    };

    let mut reached_a_guard = false;
    let mut seen: BTreeSet<Node> = BTreeSet::new();
    let mut queue: Vec<(Node, Vec<String>)> = feature
        .front_doors
        .iter()
        .map(|f| (f.clone(), vec![show(f)]))
        .collect();

    while let Some((node, path)) = queue.pop() {
        if !seen.insert(node.clone()) {
            continue;
        }
        let before_guard: Option<usize> = match guard_state(&node) {
            Guarded::Yes { asked_at } => {
                reached_a_guard = true;
                Some(asked_at)
            }
            Guarded::TooLate {
                asked_at,
                wrote_at,
                what,
            } => {
                return Err(format!(
                    "{}: statement {wrote_at} calls `{what}`, which writes, before statement \
                     {asked_at} asks whether this platform can schedule at all. A refusal after \
                     a write is not a refusal: it leaves the state behind and reports failure. \
                     Reached from the Schedule subcommand as {}.",
                    show(&node),
                    path.join(" -> ")
                ));
            }
            Guarded::No => None,
        };

        if let Some(asked_at) = before_guard {
            // Behind the guard from here on -- except whatever this
            // function called *before* asking. Moving the writes into a
            // helper that runs first moves them no further out of reach.
            for (rel, c) in by_node.get(&node).into_iter().flatten() {
                if c.stmt >= asked_at {
                    continue;
                }
                for callee in ws.targets(rel, c) {
                    if feature.owned.contains(&callee) && !seen.contains(&callee) {
                        let mut next = path.clone();
                        next.push(show(&callee));
                        queue.push((callee, next));
                    }
                }
            }
            continue;
        }

        if let Some((_, c)) = by_node
            .get(&node)
            .and_then(|sites| sites.iter().find(|(_, c)| is_mutator(c)))
        {
            return Err(format!(
                "{}: writes (`{}`) on a path from the Schedule subcommand that never asks \
                 whether this platform can run a scheduled job. On a platform without the \
                 backend that writes a job which cannot run, and reports success. Path: {}.",
                show(&node),
                c.written,
                path.join(" -> ")
            ));
        }

        for callee in walk.get(&node).into_iter().flatten() {
            if feature.owned.contains(callee) && !seen.contains(callee) {
                let mut next = path.clone();
                next.push(show(callee));
                queue.push((callee.clone(), next));
            }
        }
    }

    if !reached_a_guard {
        return Err(
            "no scheduling path reaches a capability guard: either the installer stopped \
             installing, or the derivation no longer finds the code that does it"
                .into(),
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------
// Rule 3: which refusal a platform gives is decided once
// ---------------------------------------------------------------------

/// The refusals meaning "this platform cannot replay history" are
/// **derived**: whichever `RefreshRefusal` variants the function that
/// reads `ContinuitySource` constructs. Nothing is listed, so adding a
/// third or renaming both keeps the rule true.
///
/// No other non-test function may construct them. An exhaustive match
/// over the enum is exempt *structurally* -- a function that matches on
/// `RefreshRefusal` has to name every variant and decides nothing --
/// rather than by the author's choice of function name.
fn the_platform_refusal_is_decided_once(ws: &Workspace) -> Result<(), String> {
    let mut decider: Option<(String, String)> = None;
    let mut variants: BTreeSet<String> = BTreeSet::new();

    for file in &ws.files {
        let (rel, arms) = (&file.rel, &file.arms);
        for f in &file.funcs {
            if !(f.body.contains("ContinuitySource") || f.body.contains("replays_history")) {
                continue;
            }
            // A function that *matches on* a refusal (to explain it,
            // name it, render it) is reading a decision, not making one.
            // Same structural test the second loop uses, for the same
            // reason: recognised by shape, not by function name.
            if arms
                .iter()
                .any(|a| a.func == f.name && a.pattern.contains("RefreshRefusal"))
            {
                continue;
            }
            let found = refusal_variants(&f.body);
            if found.is_empty() {
                continue;
            }
            if let Some((prev_rel, prev_fn)) = &decider {
                return Err(format!(
                    "{prev_rel}::{prev_fn} and {rel}::{} both decide which refusal a platform \
                     without replay gives. Two decisions can disagree, and then \"no backend \
                     yet\" and \"this kernel keeps no history\" become the same message.",
                    f.name
                ));
            }
            decider = Some((rel.clone(), f.name.clone()));
            variants = found;
        }
    }

    let Some((decider_rel, decider_fn)) = decider else {
        return Err(
            "no function anywhere in the workspace derives a RefreshRefusal from the platform's \
             continuity source: the refusal a platform without replay gives is hardcoded \
             somewhere instead of decided from the contract"
                .into(),
        );
    };

    for file in &ws.files {
        let (rel, arms) = (&file.rel, &file.arms);
        for f in &file.funcs {
            if *rel == decider_rel && f.name == decider_fn {
                continue;
            }
            // Exhaustive matches name every variant because a match
            // must. Recognised by shape, not by name.
            if arms
                .iter()
                .any(|a| a.func == f.name && a.pattern.contains("RefreshRefusal"))
            {
                continue;
            }
            let offending: Vec<String> = refusal_variants(&f.body)
                .into_iter()
                .filter(|v| variants.contains(v))
                .collect();
            if !offending.is_empty() {
                return Err(format!(
                    "{rel}::{}: constructs {} directly. Which refusal a platform without replay \
                     gives is {decider_rel}::{decider_fn}'s decision, derived from the \
                     continuity source; a second copy can disagree with it.",
                    f.name,
                    offending
                        .iter()
                        .map(|v| format!("RefreshRefusal::{v}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
    }
    Ok(())
}

/// The `RefreshRefusal::X` variants a body names. Reads the *resolved*
/// body, so an aliased spelling is still found, and tolerates the token
/// spacing `syn`'s stringification introduces.
fn refusal_variants(body: &str) -> BTreeSet<String> {
    let compact: String = body.chars().filter(|c| !c.is_whitespace()).collect();
    let needle = "RefreshRefusal::";
    let mut out = BTreeSet::new();
    let mut rest = compact.as_str();
    while let Some(at) = rest.find(needle) {
        rest = &rest[at + needle.len()..];
        let end = rest
            .find(|c: char| !(c.is_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        let name = &rest[..end];
        if name.chars().next().is_some_and(char::is_uppercase) {
            out.insert(name.to_string());
        }
    }
    out
}
