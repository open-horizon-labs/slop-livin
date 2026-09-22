//! Evidence guardrails: facts, not verdicts, in every string that can
//! reach a person or an agent; every unknown carries its reason; nothing
//! promised is left undelivered; no public evidence API is dead.

use super::{empty_text, load, verdict};
use crate::program::{Program, TypeKind, contains_token};
use std::collections::HashSet;
use std::path::Path;

// ---------------------------------------------------------------------
// agent_interface_facts_not_verdicts
// ---------------------------------------------------------------------

/// The verdict vocabulary the tool never asserts about a path.
pub const VERDICTS: &[&str] = &[
    "safe to delete",
    "safe to remove",
    "can be deleted",
    "should delete",
    "stale",
    "unused",
];

/// Phrases that *deny* a verdict. Whole negating phrases, not a
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

fn verdict_in(lit: &str) -> Option<&'static str> {
    let mut l = lit.to_ascii_lowercase();
    for n in NEGATED_VERDICTS {
        l = l.replace(n, "");
    }
    VERDICTS.iter().find(|v| l.contains(*v)).copied()
}

pub fn agent_interface_facts_not_verdicts(root: &Path) -> Result<(), String> {
    let p = load(root);
    let mut problems = Vec::new();
    // Every literal that can flow into a rendered or serialized string:
    // every production literal in the three crates, and every constant
    // any production function names (followed through constants), so a
    // verdict defined as a `pub const` anywhere and pushed by a renderer
    // is the renderer's literal.
    for f in p.funs.iter() {
        for lit in &f.literals {
            if let Some(v) = verdict_in(lit) {
                problems.push(format!("{} emits {lit:?}, which carries the verdict {v:?}", f.display()));
            }
        }
        for d in p.consts_reached(f) {
            for lit in &d.literals {
                if let Some(v) = verdict_in(lit) {
                    problems.push(format!(
                        "{} names `{}` ({}), whose value {lit:?} carries the verdict {v:?}",
                        f.display(),
                        d.name,
                        d.rel
                    ));
                }
            }
        }
        // A macro this layer cannot see through, in a function that
        // produces strings, is not a pass.
        if !f.unknown_macros.is_empty() && (f.krate != "swamp_core" || f.module == "render" || f.module == "agent_json") {
            problems.push(format!(
                "{} emits through `{}!`, which the resolver cannot see through",
                f.display(),
                f.unknown_macros.join("!, ")
            ));
        }
    }
    verdict("facts, not verdicts: no user- or agent-facing string asserts a verdict", problems)
}

// ---------------------------------------------------------------------
// activity_and_consumer_evidence_have_limits
// ---------------------------------------------------------------------

pub fn activity_and_consumer_evidence_have_limits(root: &Path) -> Result<(), String> {
    let p = load(root);
    let mut problems = Vec::new();
    let status_module: HashSet<String> = p.types.iter().filter(|t| t.name == "FactStatus").map(|t| t.rel.clone()).collect();
    if status_module.is_empty() {
        problems.push("`FactStatus` is not defined".into());
    }
    // 1. A status variant is built only by the evidence module's own
    //    constructors, whose signatures make the reason mandatory.
    for f in p.funs.iter() {
        if status_module.contains(&f.rel) {
            continue;
        }
        for l in &f.struct_lits {
            if l.path.contains("FactStatus") {
                problems.push(format!(
                    "{} builds `{}` as a struct literal; construct it through the `Evidence` \
                     constructors, whose signature makes the reason mandatory",
                    f.display(),
                    l.path
                ));
            }
        }
    }
    // 2. The reasoned constructors: `Evidence` methods with a `reason`
    //    parameter. The argument in that position is never empty -- as a
    //    literal, a constant holding "", or an empty constructor.
    let ctors: Vec<(String, usize)> = p
        .funs
        .iter()
        .filter(|f| f.self_ty.as_deref() == Some("Evidence"))
        .filter_map(|f| {
            f.params
                .iter()
                .filter(|(n, _)| n != "self")
                .position(|(n, _)| n.trim() == "reason")
                .map(|pos| (f.name.clone(), pos))
        })
        .collect();
    if ctors.is_empty() {
        problems.push("no `Evidence` constructor takes a `reason`".into());
    }
    for f in p.funs.iter() {
        for c in &f.calls {
            if c.method {
                continue;
            }
            let Some((name, pos)) = ctors.iter().find(|(n, _)| c.is(&format!("Evidence::{n}"))) else {
                continue;
            };
            let arg = c.args.get(*pos).cloned().unwrap_or_default();
            if arg.trim().is_empty() || empty_text(&p, &arg) {
                problems.push(format!(
                    "{} passes an empty reason to `Evidence::{name}` (`{}`): \"not observed\" has \
                     to say why",
                    f.display(),
                    arg.trim()
                ));
            }
        }
    }
    // 3. Every activity fact names its source, or delegates to a builder
    //    that does.
    let returns_evidence = |f: &crate::program::Fun| contains_token(&f.ret, "Evidence");
    let builders: HashSet<usize> = p.funs.iter().enumerate().filter(|(_, f)| f.module == "activity" && returns_evidence(f)).map(|(i, _)| i).collect();
    for i in &builders {
        let f = &p.funs[*i];
        let names_source = f.body.contains("EvidenceSource ::");
        let delegates = p.callees(*i).iter().any(|g| returns_evidence(&p.funs[*g]) && p.funs[*g].body.contains("EvidenceSource ::"));
        if !names_source && !delegates {
            problems.push(format!("{} returns evidence without naming an `EvidenceSource`", f.display()));
        }
    }
    // 4. The renderer prints the reason with the status.
    let render = p.defs("render::render_evidence_lines");
    if render.is_empty() || !render.iter().any(|i| contains_token(&p.funs[*i].body, "reason")) {
        problems.push("render::render_evidence_lines does not print the reason: an unknown without its reason reads as \"nothing there\"".into());
    }
    verdict("every unknown carries its reason and every fact its source", problems)
}

// ---------------------------------------------------------------------
// computed_but_not_delivered
// ---------------------------------------------------------------------

/// Initializers that mean "nothing was computed here".
fn empty_init(e: &str) -> bool {
    let e = e.replace(' ', "");
    matches!(
        e.as_str(),
        "Vec::new()" | "vec![]" | "Default::default()" | "Vec::default()" | "None" | "String::new()"
            | "BTreeMap::new()" | "HashMap::new()" | "BTreeSet::new()" | "HashSet::new()"
    )
}

pub fn computed_but_not_delivered(root: &Path) -> Result<(), String> {
    let p = load(root);
    let mut problems = Vec::new();
    // A delivered surface: every public struct the core crate serializes.
    let surfaces: Vec<&crate::program::TypeDecl> = p
        .types
        .iter()
        .filter(|t| t.krate == "swamp_core" && t.kind == TypeKind::Struct && t.is_pub && t.derives("Serialize"))
        .collect();
    for t in surfaces {
        for (field, _ty, is_pub, attrs) in &t.fields {
            if !is_pub {
                continue;
            }
            let mut inits: Vec<String> = Vec::new();
            for f in p.funs.iter() {
                for l in &f.struct_lits {
                    if crate::resolve::path_ends_with(&l.path, &t.name) || (l.path == "Self" && f.self_ty.as_deref() == Some(t.name.as_str())) {
                        for (n, init) in &l.fields {
                            if n == field {
                                inits.push(init.clone());
                            }
                        }
                    }
                }
            }
            let mutated = p.funs.iter().any(|f| {
                f.assigns.iter().any(|a| a.lhs.replace(' ', "").ends_with(&format!(".{field}")))
                    || f.calls.iter().any(|c| {
                        c.method
                            && ["push", "extend", "insert", "append", "push_str", "get_or_insert_with", "entry", "retain", "sort", "iter_mut"].contains(&c.path.as_str())
                            && c.receiver.replace(' ', "").ends_with(&format!(".{field}"))
                    })
            });
            if !inits.is_empty() && inits.iter().all(|i| empty_init(i)) && !mutated {
                problems.push(format!(
                    "{}::{}.{field} is written only as an empty default ({} site(s)): compute it \
                     or remove it together with the claim that it is delivered",
                    t.rel,
                    t.name,
                    inits.len()
                ));
                continue;
            }
            // A field serde hides when empty has to be read by something.
            if attrs.contains("skip_serializing_if") && !p.funs.iter().any(|f| f.fields.iter().any(|x| x == field)) {
                problems.push(format!(
                    "{}::{}.{field} is populated but nothing reads it: a field serde hides and no \
                     renderer names is not delivered",
                    t.rel, t.name
                ));
            }
        }
    }
    verdict("what is computed for a delivered surface is delivered", problems)
}

// ---------------------------------------------------------------------
// no_dead_public_evidence_api
// ---------------------------------------------------------------------

pub fn no_dead_public_evidence_api(root: &Path) -> Result<(), String> {
    let p = load(root);
    let mut problems = Vec::new();
    // The evidence API: modules whose public functions produce or take the
    // evidence vocabulary (types declared beside `Evidence`) or the
    // occupancy tri-state.
    let vocab: HashSet<String> = p
        .types
        .iter()
        .filter(|t| p.types.iter().any(|e| e.name == "Evidence" && e.rel == t.rel))
        .map(|t| t.name.clone())
        .chain(["OccupancyState".to_string()])
        .collect();
    let speaks = |f: &crate::program::Fun| vocab.iter().any(|v| contains_token(&f.ret, v) || f.params.iter().any(|(_, t)| contains_token(t, v)));
    let modules: HashSet<String> = p
        .funs
        .iter()
        .filter(|f| f.krate == "swamp_core" && f.is_pub && speaks(f))
        .map(|f| f.rel.clone())
        .collect();
    // Entries: every binary's `main`, every trait method (dispatched by
    // the bus, serde or the runtime), and the TUI's public run functions.
    let entries: Vec<usize> = p
        .funs
        .iter()
        .enumerate()
        .filter(|(_, f)| (f.name == "main" && f.krate == "swamp") || f.trait_.is_some() || (f.krate == "swamp_tui" && f.is_pub && f.name.starts_with("run")))
        .map(|(i, _)| i)
        .collect();
    let live = p.reachable(&entries, &HashSet::new());
    for (i, f) in p.funs.iter().enumerate() {
        if !modules.contains(&f.rel) || !f.is_pub || f.trait_.is_some() || live.contains(&i) {
            continue;
        }
        problems.push(format!(
            "{} is public evidence API that nothing reachable from a binary calls (a caller that \
             is itself dead does not count)",
            f.display()
        ));
    }
    // Public constants and statics are read, not called.
    for d in &p.items {
        if !modules.contains(&d.rel) || !d.is_pub || !matches!(d.kind, crate::program::DeclKind::Const | crate::program::DeclKind::Static) {
            continue;
        }
        let read = live.iter().any(|i| contains_token(&p.funs[*i].body, &d.name))
            || p.items.iter().any(|o| o.name != d.name && contains_token(&o.value, &d.name));
        if !read {
            problems.push(format!("{}::{} (pub const/static) has no live reader", d.rel, d.name));
        }
    }
    verdict(
        "public evidence API is wired into the live pipeline, or deleted with the claims that it \
         is delivered",
        problems,
    )
}
