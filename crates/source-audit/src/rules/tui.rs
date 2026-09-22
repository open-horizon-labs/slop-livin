//! `tui_actions_off_event_thread`: nothing reachable from a key handler
//! or the render loop blocks.
//!
//! **Roots, by convention.** The event thread's entry points are the TUI
//! functions named for the event loop, key handling and drawing (plus
//! the required ones, which must exist). A new `handle_key_*` is a root
//! the moment it exists.
//!
//! **Blocking, derived.** A TUI function blocks when it (outside a
//! closure handed to `thread::spawn`) builds a subprocess, sleeps, waits
//! on a channel or a child, or joins a worker -- or calls into core
//! code that is in the closure of those plus directory traversal. The
//! old hand-written sink list (`report_scope`, `execute_plan`,
//! `propose`, ...) is gone: those are blocking because of what they
//! reach, and a new core entry point that reaches the same thing is
//! blocking the day it lands. Calls inside macro arguments are calls
//! (`vec![probe_path(p)]` was an `lsof` per keystroke the old graph
//! never saw).

use super::{load, verdict};
use crate::program::{PCall, spawn_call, traversal_call};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;

const REQUIRED_ROOTS: &[&str] = &["event_loop", "handle_terminal_key", "handle_key", "handle_key_mod", "draw"];
const ROOT_PATTERNS: &[&str] = &["handle_key", "handle_terminal", "on_key", "event_loop", "draw"];

/// Blocking on the calling thread, in the standard library's terms.
fn blocks(c: &PCall) -> bool {
    if c.in_spawn {
        return false;
    }
    if c.method {
        if ["recv", "recv_timeout", "wait", "wait_with_output"].contains(&c.path.as_str()) {
            return true;
        }
        let r = crate::resolve::root_ident(&c.receiver);
        return c.path == "join" && (r.contains("handle") || r.contains("worker"));
    }
    spawn_call(c) || c.is("spawn::command") || c.is("thread::sleep")
}

pub fn tui_actions_off_event_thread(root: &Path) -> Result<(), String> {
    let p = load(root);
    let mut problems = Vec::new();
    // Core code that blocks: subprocesses, sleeps, waits, and traversal,
    // transitively.
    // The one-level, capped `locations::shallow_list` is the bounded
    // listing the guardrails sanction everywhere; it is not a scan.
    let bounded = super::bounded_primitives(&p, &[("locations::shallow_list", "SHALLOW_LIST_CAP")], false, &mut problems);
    let core_blocking = p.closure("tui_blocking", &bounded, |_, c| {
        (spawn_call(c) || c.is("thread::sleep") || traversal_call(c) || (c.method && ["recv", "recv_timeout", "wait_with_output"].contains(&c.path.as_str())))
            && !c.in_spawn
    });
    let tui: Vec<usize> = p.funs.iter().enumerate().filter(|(_, f)| f.krate == "swamp_tui").map(|(i, _)| i).collect();
    for r in REQUIRED_ROOTS {
        if !tui.iter().any(|i| p.funs[*i].name == *r) {
            problems.push(format!("missing UI root `{r}`; update the audit when entry points change"));
        }
    }
    let roots: Vec<usize> = tui
        .iter()
        .copied()
        .filter(|i| ROOT_PATTERNS.iter().any(|pat| p.funs[*i].name.contains(pat)))
        .collect();
    let mut prev: HashMap<usize, usize> = HashMap::new();
    let mut seen: HashSet<usize> = roots.iter().copied().collect();
    let mut queue: VecDeque<usize> = roots.iter().copied().collect();
    let chain = |prev: &HashMap<usize, usize>, mut at: usize| {
        let mut names = vec![p.funs[at].name.clone()];
        while let Some(b) = prev.get(&at) {
            names.push(p.funs[*b].name.clone());
            at = *b;
        }
        names.reverse();
        names.join(" -> ")
    };
    while let Some(i) = queue.pop_front() {
        let f = &p.funs[i];
        for (ci, c) in f.calls.iter().enumerate() {
            if c.in_spawn {
                continue;
            }
            if blocks(c) {
                problems.push(format!(
                    "blocking action on the UI path: {} -> `{}`; dispatch it on a worker thread \
                     and report progress over a channel",
                    chain(&prev, i),
                    c.written
                ));
                continue;
            }
            let t = p.target(i, ci);
            for g in &t.local {
                let gf = &p.funs[*g];
                if gf.krate == "swamp_tui" {
                    if seen.insert(*g) {
                        prev.insert(*g, i);
                        queue.push_back(*g);
                    }
                } else if !t.possible && core_blocking.contains(g) {
                    problems.push(format!(
                        "blocking action on the UI path: {} -> `{}` ({})",
                        chain(&prev, i),
                        gf.path(),
                        p.chain(*g, &core_blocking, |h| p.funs[h].calls.iter().any(|c| spawn_call(c) || c.is("thread::sleep") || traversal_call(c)))
                    ));
                }
            }
        }
        // A function named as a value on the UI path may be called there.
        for g in p.ref_targets(i) {
            if p.funs[*g].krate == "swamp_tui" && seen.insert(*g) {
                prev.insert(*g, i);
                queue.push_back(*g);
            } else if p.funs[*g].krate != "swamp_tui" && core_blocking.contains(g) {
                problems.push(format!("blocking action on the UI path: {} names `{}`", chain(&prev, i), p.funs[*g].path()));
            }
        }
    }
    verdict("no blocking scan, review or cleanup on the TUI event or render path", problems)
}
