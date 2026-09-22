//! The event-bus guardrails: consumers are independent, registered once,
//! statically, and the report path runs only through the bus.
//!
//! **Scope, derived.** A *consumer* is any type that implements
//! `bus::Consumer`, wherever it is written -- `consumers/mod.rs` included,
//! which the old file filter skipped. A *consumer module* is the file
//! that implements one. The *bus* is the type whose methods take a
//! `Box<dyn Consumer>`; its *registration* methods are those methods,
//! and its *static registrar* is its associated constructor that calls
//! them. The *report path* is every module that calls the bus's
//! `run_report`; its *stages* are the functions the consumers call.

use super::{load, verdict};
use crate::program::{Fun, Program, contains_token};
use std::collections::HashSet;
use std::path::Path;

struct Bus {
    handlers: HashSet<String>,
    sink_names: HashSet<String>,
    registrar_names: HashSet<String>,
    consumers: Vec<(String, String)>,
    consumer_files: HashSet<String>,
    bus_types: HashSet<String>,
    register: HashSet<usize>,
    registrars: HashSet<usize>,
}

fn bus(p: &Program) -> Result<Bus, String> {
    let consumers: Vec<(String, String)> = p
        .implementors("bus::Consumer")
        .iter()
        .map(|i| (i.self_ty.clone(), i.rel.clone()))
        .collect();
    if consumers.is_empty() {
        return Err("nothing implements `bus::Consumer`: the pipeline is not on the bus".into());
    }
    let consumer_files: HashSet<String> =
        p.family(&consumers.iter().map(|(_, r)| r.clone()).collect());
    let register: HashSet<usize> = p
        .funs
        .iter()
        .enumerate()
        // The registration sink: a method that takes a consumer and
        // stores it in the bus itself. A method that merely forwards one
        // (`fn register_late(&mut self, c) { self.register(c) }`) is a
        // caller of the sink, not the sink.
        .filter(|(_, f)| {
            f.self_ty.is_some()
                && f.params
                    .iter()
                    .any(|(_, t)| t.contains("dyn") && contains_token(t, "Consumer"))
                && f.calls.iter().any(|c| {
                    c.method
                        && ["push", "insert", "push_back", "extend"].contains(&c.path.as_str())
                        && c.receiver.replace(' ', "").starts_with("self.")
                })
        })
        .map(|(i, _)| i)
        .collect();
    let bus_types: HashSet<String> = register
        .iter()
        .filter_map(|i| p.funs[*i].self_ty.clone())
        .collect();
    if register.is_empty() {
        return Err("no bus type takes a `Box<dyn Consumer>`: nothing registers consumers".into());
    }
    // The static registrar: an associated function of the bus type with
    // no receiver, returning the bus, that registers.
    let registrars: HashSet<usize> = p
        .funs
        .iter()
        .enumerate()
        .filter(|(i, f)| {
            f.self_ty.as_ref().is_some_and(|t| bus_types.contains(t))
                && !f.params.iter().any(|(n, _)| n == "self")
                && (f.ret.trim() == "Self" || f.self_ty.as_ref().is_some_and(|t| f.ret.trim() == t))
                && p.callees(*i).iter().any(|g| register.contains(g))
        })
        .map(|(i, _)| i)
        .collect();
    // The event handlers: the methods the `Consumer` trait declares that
    // take an event.
    let handlers: HashSet<String> = p
        .types
        .iter()
        .filter(|t| t.name == "Consumer" && t.kind == crate::program::TypeKind::Trait)
        .flat_map(|t| t.variants.iter().cloned())
        .filter(|m| m.starts_with("on_"))
        .collect();
    let sink_names: HashSet<String> = register.iter().map(|i| p.funs[*i].name.clone()).collect();
    let registrar_names: HashSet<String> =
        registrars.iter().map(|i| p.funs[*i].name.clone()).collect();
    Ok(Bus {
        handlers,
        sink_names,
        registrar_names,
        consumers,
        consumer_files,
        bus_types,
        register,
        registrars,
    })
}

/// Every local definition a function reaches by call or value.
fn reached(p: &Program, i: usize) -> Vec<usize> {
    p.callees(i).to_vec()
}

fn names_type(f: &Fun, ty: &str) -> bool {
    contains_token(&f.body, ty) || contains_token(&f.sig, ty)
}

pub fn no_consumer_knows_other_consumers(root: &Path) -> Result<(), String> {
    let p = load(root);
    let b = bus(&p)?;
    let mut problems = Vec::new();
    for (i, f) in p.funs.iter().enumerate() {
        if !b.consumer_files.contains(&f.rel) {
            continue;
        }
        for t in &b.bus_types {
            if names_type(f, t) {
                problems.push(format!("{} names the bus type `{t}`", f.display()));
            }
        }
        for g in reached(&p, i) {
            let other = &p.funs[g].rel;
            if other != &f.rel && b.consumer_files.contains(other) {
                problems.push(format!(
                    "{} reaches `{}` in another consumer's module {other}; the bus is the only \
                     coupling",
                    f.display(),
                    p.funs[g].name
                ));
            }
            if b.registrars.contains(&g) || b.register.contains(&g) {
                problems.push(format!("{} registers consumers itself", f.display()));
            }
        }
        for (c, rel) in &b.consumers {
            if rel != &f.rel && names_type(f, c) {
                problems.push(format!("{} names consumer `{c}` from {rel}", f.display()));
            }
        }
        // Registration knowledge by name: a consumer module calling
        // anything spelled like the bus's registrar or its sink.
        for c in &f.calls {
            if b.registrar_names.contains(c.callee())
                || (c.method && b.sink_names.contains(c.callee()))
            {
                problems.push(format!(
                    "{} calls `{}`, which is registration knowledge a consumer must not have",
                    f.display(),
                    c.written
                ));
            }
        }
    }
    // Item level: a const or a field in a consumer module naming another.
    for d in &p.items {
        if !b.consumer_files.contains(&d.rel) {
            continue;
        }
        for (c, rel) in &b.consumers {
            if rel != &d.rel && (contains_token(&d.ty, c) || contains_token(&d.value, c)) {
                problems.push(format!(
                    "{}::{} names consumer `{c}` from {rel}",
                    d.rel, d.name
                ));
            }
        }
    }
    verdict(
        "no consumer knows another consumer or the bus; the bus is the only coupling",
        problems,
    )
}

pub fn static_registration_only(root: &Path) -> Result<(), String> {
    let p = load(root);
    let b = bus(&p)?;
    let mut problems = Vec::new();
    if b.registrars.is_empty() {
        problems.push(
            "the bus has no static registrar (an associated constructor that registers)".into(),
        );
    }
    for (i, f) in p.funs.iter().enumerate() {
        if b.registrars.contains(&i) || b.register.contains(&i) {
            continue;
        }
        for (ci, c) in f.calls.iter().enumerate() {
            let t = p.target(i, ci);
            // Only an exactly resolved call: `registry.register(..)` on
            // the agent registry is not the bus.
            if !t.possible && t.local.iter().any(|g| b.register.contains(g)) {
                problems.push(format!(
                    "{} registers a consumer (`{}`) outside the bus's static registrar: the \
                     registered set is decided before the first event, not while events flow",
                    f.display(),
                    c.written
                ));
            }
        }
    }
    // An event handler (a method named as the `Consumer` trait's
    // handlers are) never registers, however the bus it holds is typed.
    for f in p.funs.iter() {
        if !b.handlers.contains(&f.name) {
            continue;
        }
        for c in &f.calls {
            if b.registrar_names.contains(c.callee())
                || (c.method && b.sink_names.contains(c.callee()))
            {
                problems.push(format!(
                    "{} registers at event time (`{}`): the registered set is decided before the \
                     first event",
                    f.display(),
                    c.written
                ));
            }
        }
    }
    verdict(
        "consumers are registered statically, once, before any event",
        problems,
    )
}

pub fn extractors_are_pluggable(root: &Path) -> Result<(), String> {
    let p = load(root);
    let b = bus(&p)?;
    let mut problems = Vec::new();
    // Every consumer is constructed for registration exactly once in the
    // production program, and that construction is in the registrar.
    for (c, rel) in &b.consumers {
        let mut sites: Vec<String> = Vec::new();
        let mut in_registrar = 0usize;
        for (i, f) in p.funs.iter().enumerate() {
            let n = boxed(&f.body, c);
            if n > 0 {
                sites.push(format!("{} x{n}", f.display()));
                if b.registrars.contains(&i) {
                    in_registrar += n;
                }
            }
        }
        let total: usize = p.funs.iter().map(|f| boxed(&f.body, c)).sum();
        if total != 1 || in_registrar != 1 {
            problems.push(format!(
                "consumer `{c}` ({rel}) is boxed for registration {total} time(s), {in_registrar} \
                 in the static registrar ({}): exactly once, there, or every event it handles is \
                 handled a different number of times",
                sites.join(", ")
            ));
        }
    }
    // Nothing outside the consumers and the registrar reaches into a
    // consumer module: a new source must not need walker changes.
    let registrar_files: HashSet<String> = b
        .registrars
        .iter()
        .map(|i| p.funs[*i].rel.clone())
        .collect();
    for (i, f) in p.funs.iter().enumerate() {
        // The bus dispatches to consumers by trait object; that is what
        // it is for.
        if b.consumer_files.contains(&f.rel) || registrar_files.contains(&f.rel) {
            continue;
        }
        let exact: Vec<usize> = (0..f.calls.len())
            .filter(|ci| !p.target(i, *ci).possible)
            .flat_map(|ci| p.target(i, ci).local.clone())
            .chain(p.ref_targets(i).iter().copied())
            .collect();
        let consumer_mods: Vec<String> = b
            .consumer_files
            .iter()
            .map(|r| Program::file_module(r))
            .collect();
        for (ci, c) in f.calls.iter().enumerate() {
            let abs = &p.target(i, ci).abs;
            if let Some(m) = consumer_mods
                .iter()
                .find(|m| abs.starts_with(&format!("{m}::")))
            {
                problems.push(format!(
                    "{} calls `{}` ({abs}) in consumer module {m}",
                    f.display(),
                    c.written
                ));
            }
        }
        for path in super::named_paths(&f.body) {
            let (abs, _) = crate::program::absolute(&path, &f.krate, &f.module, None);
            if let Some(m) = consumer_mods
                .iter()
                .find(|m| abs.starts_with(&format!("{m}::")))
            {
                problems.push(format!(
                    "{} names `{path}` in consumer module {m}: stages never depend on consumers",
                    f.display()
                ));
            }
        }
        for g in exact {
            if b.consumer_files.contains(&p.funs[g].rel) && !b.consumer_files.contains(&f.rel) {
                problems.push(format!(
                    "{} reaches into consumer module {}: stages never depend on consumers",
                    f.display(),
                    p.funs[g].rel
                ));
            }
        }
    }
    verdict(
        "every consumer is registered exactly once and stages never reach consumers",
        problems,
    )
}

/// How many times `body` constructs `Box::new(<..>::ty ..)`.
fn boxed(body: &str, ty: &str) -> usize {
    let mut n = 0;
    let mut rest = body;
    while let Some(at) = rest.find("Box :: new (") {
        let after = &rest[at + "Box :: new (".len()..];
        let path: String = after
            .trim_start()
            .split(|c: char| !(c.is_alphanumeric() || c == '_' || c == ':' || c == ' '))
            .next()
            .unwrap_or("")
            .replace(' ', "");
        // `Box::new(X)`, `Box::new(X::default())`, `Box::new(path::X)`.
        if path.split("::").any(|s| s == ty) {
            n += 1;
        }
        rest = after;
    }
    n
}

pub fn all_report_paths_through_bus(root: &Path) -> Result<(), String> {
    let p = load(root);
    let b = bus(&p)?;
    let mut problems = Vec::new();
    let run = p.defs("bus::run_report");
    if run.is_empty() {
        return Err("the bus has no `run_report`".into());
    }
    let run: HashSet<usize> = run.into_iter().collect();
    // The report path: modules that run the bus.
    let report_files: HashSet<String> = p
        .funs
        .iter()
        .enumerate()
        .filter(|(i, _)| p.callees(*i).iter().any(|g| run.contains(g)))
        .map(|(_, f)| f.rel.clone())
        .collect();
    if report_files.is_empty() {
        problems.push("nothing runs the bus (`bus::run_report`)".into());
    }
    let report_files = p.family(&report_files);
    // Stages: what the consumers call, outside the bus and the consumers,
    // that does observation work -- walks, reads a store, writes history
    // or runs a process. `entities::now()` is a consumer's helper, not a
    // stage.
    let none = HashSet::new();
    let heavy: HashSet<usize> = p
        .traversal(&none)
        .iter()
        .chain(p.destructive().iter())
        .chain(p.unbounded_reads(&none).iter())
        .copied()
        .collect();
    // Everything the consumers' handlers reach that does that work.
    let handlers: Vec<usize> = p
        .funs
        .iter()
        .enumerate()
        .filter(|(_, f)| b.consumer_files.contains(&f.rel) && f.trait_.is_some())
        .map(|(i, _)| i)
        .collect();
    let stages: HashSet<usize> = p
        .reachable_exact(&handlers, &HashSet::new())
        .into_iter()
        .filter(|g| {
            let gf = &p.funs[*g];
            !b.consumer_files.contains(&gf.rel)
                && !gf.module.starts_with("bus")
                && !report_files.contains(&gf.rel)
                && heavy.contains(g)
        })
        .collect();
    // ... and the entry points that wrap them in their own modules
    // (`observe_tracked_with_source` over `stage_tracked_with_source`).
    // A stage *entry* is one a handler calls directly; a wrapper is a
    // function beside it that calls it directly.
    let entries: HashSet<usize> = handlers
        .iter()
        .flat_map(|h| p.callees(*h).to_vec())
        .filter(|g| stages.contains(g))
        .collect();
    let wrappers: Vec<usize> = entries
        .iter()
        .flat_map(|e| p.callers(*e))
        .filter(|g| {
            entries.iter().any(|e| p.funs[*e].rel == p.funs[*g].rel)
                && !report_files.contains(&p.funs[*g].rel)
                && !b.consumer_files.contains(&p.funs[*g].rel)
        })
        .collect();
    let stages: HashSet<usize> = stages.into_iter().chain(wrappers).collect();
    // A stage the report module itself defines (a consumer calls it) may
    // call other stages: it *is* the bus's work.
    let consumer_methods: Vec<usize> = p
        .funs
        .iter()
        .enumerate()
        .filter(|(_, f)| b.consumer_files.contains(&f.rel) && f.trait_.is_some())
        .map(|(i, _)| i)
        .collect();
    let bus_work = p.reachable_exact(&consumer_methods, &HashSet::new());
    for (i, f) in p.funs.iter().enumerate() {
        if !report_files.contains(&f.rel) || bus_work.contains(&i) {
            continue;
        }
        for (ci, c) in f.calls.iter().enumerate() {
            let t = p.target(i, ci);
            if t.possible {
                continue;
            }
            if let Some(g) = t.local.iter().find(|g| stages.contains(g)) {
                problems.push(format!(
                    "{} calls stage `{}` (`{}`) directly instead of through the bus",
                    f.display(),
                    p.funs[*g].path(),
                    c.written
                ));
            }
        }
    }
    verdict("every report path runs through the bus", problems)
}

pub fn event_bus_pluggable_consumers(root: &Path) -> Result<(), String> {
    no_consumer_knows_other_consumers(root)?;
    static_registration_only(root)?;
    extractors_are_pluggable(root)?;
    all_report_paths_through_bus(root)
}
