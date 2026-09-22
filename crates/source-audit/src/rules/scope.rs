//! Scope guardrails: discovery consumes the authorized scope, explicit
//! scope means explicit, one observation owns discovery, detector ids
//! stay in the detector registry, and a TUI refresh keeps its scope.

use super::{anchors, load, verdict};
use crate::program::{Program, TypeKind, contains_token};
use crate::resolve::Honoured;
use std::collections::HashSet;
use std::path::Path;

const DISCOVERY: &[&str] = &["external::discover_and_measure", "agents::discover_and_measure"];

/// The raw detector-output types: what a discovery pass must not read.
const RAW_DETECTOR_OUTPUT: &[&str] = &["DetectorSummary", "ProposedLocation", "LocationStatus"];

/// Modules that interpret raw detector output into scope: the modules
/// that define those types.
fn interpreters(p: &Program) -> HashSet<String> {
    p.types
        .iter()
        .filter(|t| RAW_DETECTOR_OUTPUT.contains(&t.name.as_str()))
        .map(|t| t.rel.clone())
        // The detectors themselves produce the raw output.
        .chain(p.impls.iter().filter(|i| i.implements("Detector")).map(|i| i.rel.clone()))
        .collect()
}

pub fn discovery_consumes_effective_scope(root: &Path) -> Result<(), String> {
    let p = load(root);
    let mut problems = Vec::new();
    let entries = anchors(&p, DISCOVERY, &mut problems);
    let interp = interpreters(&p);
    for t in RAW_DETECTOR_OUTPUT {
        if !p.types.iter().any(|d| d.name == *t) {
            problems.push(format!("the raw detector-output type `{t}` is gone"));
        }
    }
    // Fields that hold raw detector output, derived from their types.
    let raw_fields: HashSet<String> = p
        .types
        .iter()
        .filter(|t| t.kind == TypeKind::Struct)
        .flat_map(|t| t.fields.iter())
        // A field holding detector output wholesale -- a collection of
        // summaries or proposed locations. (A single status field inside a
        // proposed location is only reachable through one of these.)
        .filter(|(_, ty, _, _)| RAW_DETECTOR_OUTPUT[..2].iter().any(|r| contains_token(ty, r)) && ty.contains("Vec"))
        .map(|(n, _, _, _)| n.clone())
        .collect();
    let variants: Vec<String> = p
        .types
        .iter()
        .filter(|t| t.name == "LocationStatus")
        .flat_map(|t| t.variants.iter().map(|v| format!("LocationStatus :: {v}")))
        .collect();
    // The discovery region: every module a discovery pass reaches, less
    // the interpreters themselves.
    let region = p.reachable_exact(&entries.iter().copied().collect::<Vec<_>>(), &HashSet::new());
    let region_files: HashSet<String> = region
        .iter()
        .map(|i| p.funs[*i].rel.clone())
        .chain(entries.iter().map(|i| p.funs[*i].rel.clone()))
        .filter(|r| !interp.contains(r))
        .collect();
    for f in p.funs.iter() {
        if !region_files.contains(&f.rel) {
            continue;
        }
        if let Some(field) = f.fields.iter().find(|x| raw_fields.contains(*x)) {
            problems.push(format!(
                "{} reads `.{field}`, raw detector output: discovery consumes \
                 `EffectiveScope::authorized_roots()`, whatever the binding is called",
                f.display()
            ));
        }
        if let Some(v) = variants.iter().find(|v| f.body.contains(v.as_str())) {
            problems.push(format!("{} matches on `{v}`, a raw detector status", f.display()));
        }
    }
    let seam: HashSet<usize> = p
        .defs("EffectiveScope::authorized_roots")
        .into_iter()
        .chain(p.defs("EffectiveScope::authorized_detector_paths_in_explicit_roots"))
        .collect();
    if seam.is_empty() {
        problems.push("`EffectiveScope::authorized_roots()` is not defined".into());
    }
    for e in &entries {
        if p.reachable(&[*e], &HashSet::new()).is_disjoint(&seam) {
            problems.push(format!("{} never reaches `EffectiveScope::authorized_roots()`", p.funs[*e].display()));
        }
    }
    verdict("discovery consumes the authorized scope, never raw detector candidates", problems)
}

pub fn explicit_only_scope_when_defaults_false(root: &Path) -> Result<(), String> {
    let p = load(root);
    let mut problems = Vec::new();
    // The config type: the struct with the allow-list field.
    let config: Vec<String> = p
        .types
        .iter()
        .filter(|t| t.fields.iter().any(|(n, _, _, _)| n == "enabled_detectors") && t.name.ends_with("Config"))
        .map(|t| t.name.clone())
        .collect();
    if config.is_empty() {
        problems.push("no config type has an `enabled_detectors` allow-list field".into());
    }
    let predicate = anchors(&p, &["scope::detectors_permitted"], &mut problems);
    for i in &predicate {
        let f = &p.funs[*i];
        if !(f.fields.iter().any(|x| x == "defaults") && f.fields.iter().any(|x| x == "enabled_detectors")) {
            problems.push(format!("{} does not read both `defaults` and `enabled_detectors`", f.display()));
        }
    }
    // Detector inference: calling a `Detector` implementation.
    let detector_methods: HashSet<usize> = p
        .funs
        .iter()
        .enumerate()
        .filter(|(_, f)| f.trait_.as_deref() == Some("Detector"))
        .map(|(i, _)| i)
        .collect();
    let infers = p.upward(&detector_methods, &predicate);
    let walks = p.traversal(&HashSet::new());
    // Every function that takes the config and produces roots by
    // inference or by listing the filesystem asks the predicate first.
    for (i, f) in p.funs.iter().enumerate() {
        let takes_config = f.params.iter().any(|(_, t)| config.iter().any(|c| contains_token(t, c)));
        if !takes_config || f.ret.is_empty() || predicate.contains(&i) || detector_methods.contains(&i) {
            continue;
        }
        let produces = (infers.contains(&i) && !detector_methods.contains(&i)) || walks.contains(&i);
        let guarded = !p.reachable(&[i], &HashSet::new()).is_disjoint(&predicate);
        if produces && !guarded {
            problems.push(format!(
                "{} resolves scope from the config (by detector inference or by listing) without \
                 asking `detectors_permitted(config)`: defaults=false means explicit only",
                f.display()
            ));
        }
    }
    // The semantics are pinned by running tests.
    let scope_file = p.file("crates/core/src/scope.rs").map(|f| super::adapters::running_tests(&f.ast)).unwrap_or_default();
    if !scope_file.iter().any(|t| t == "defaults_false_without_includes_or_enabled_detectors_is_empty") {
        problems.push("scope.rs lacks the running test `defaults_false_without_includes_or_enabled_detectors_is_empty`".into());
    }
    if !super::meta::integration_test_exists(root, "crates/core/tests/reviewer_counterexamples.rs", "defaults_false_must_mean_explicit_only") {
        problems.push("crates/core/tests/reviewer_counterexamples.rs lacks the running test `defaults_false_must_mean_explicit_only`".into());
    }
    verdict("defaults=false with no includes or enabled detectors means an explicit-only scope", problems)
}

pub fn discovery_owned_by_report_pipeline(root: &Path) -> Result<(), String> {
    let p = load(root);
    let mut problems = Vec::new();
    let entries = anchors(&p, DISCOVERY, &mut problems);
    // Every production caller of either pass.
    let mut owners: Vec<usize> = entries.iter().flat_map(|e| p.callers(*e)).filter(|c| !entries.contains(c)).collect();
    owners.sort();
    owners.dedup();
    match owners.as_slice() {
        [] => problems.push("nothing runs discovery: the one observation that owns it has to be somewhere".into()),
        [one] => {
            let reaches: HashSet<usize> = p.callees(*one).iter().copied().collect();
            for e in &entries {
                if !reaches.contains(e) {
                    problems.push(format!(
                        "{} does not run `{}`: one observation owns *both* passes, or their \
                         ownership windows can disagree again",
                        p.funs[*one].display(),
                        p.funs[*e].path()
                    ));
                }
            }
        }
        many => {
            let names: Vec<String> = many.iter().map(|i| p.funs[*i].display()).collect();
            problems.push(format!(
                "discovery runs from {} places ({}): a second pass over the shared history table \
                 is how ordering started mattering; one observation owns both",
                many.len(),
                names.join(", ")
            ));
        }
    }
    verdict("one observation owns discovery", problems)
}

pub fn detector_ids_only_in_registry(root: &Path) -> Result<(), String> {
    let p = load(root);
    let mut problems = Vec::new();
    // Detector ids, derived: the constants a `Detector` implementation's
    // `id()` returns, with the values they hold.
    let mut ids: Vec<(String, String)> = Vec::new();
    for f in p.funs.iter().filter(|f| f.trait_.as_deref() == Some("Detector") && f.name == "id") {
        for d in &p.items {
            if d.kind == crate::program::DeclKind::Const && contains_token(&f.body, &d.name) {
                ids.push((d.name.clone(), d.literals.first().cloned().unwrap_or_default()));
            }
        }
    }
    ids.sort();
    ids.dedup();
    if ids.is_empty() {
        problems.push("no `Detector::id()` returns a constant: detector ids are not declared".into());
    }
    // The registry: the detector implementations' own modules and the
    // module that declares the trait.
    let registry: HashSet<String> = p
        .impls
        .iter()
        .filter(|i| i.implements("Detector"))
        .map(|i| i.rel.clone())
        .chain(p.types.iter().filter(|t| t.name == "Detector" && t.kind == TypeKind::Trait).map(|t| t.rel.clone()))
        .collect();
    let interp = interpreters(&p);
    let inside = |rel: &str| registry.contains(rel) || interp.contains(rel) || rel.contains("/locations/");
    for f in p.funs.iter() {
        if inside(&f.rel) {
            continue;
        }
        if let Some((n, _)) = ids.iter().find(|(n, _)| contains_token(&f.body, n) || contains_token(&f.sig, n)) {
            problems.push(format!("{} names the detector id constant `{n}`", f.display()));
        }
        let matched: HashSet<&str> = ids
            .iter()
            .filter(|(_, v)| !v.is_empty() && (f.arms.iter().any(|a| a.pattern.contains(&format!("\"{v}\""))) || f.body.contains(&format!("== \"{v}\""))))
            .map(|(_, v)| v.as_str())
            .collect();
        if matched.len() >= 2 {
            let mut m: Vec<&str> = matched.into_iter().collect();
            m.sort();
            problems.push(format!(
                "{} dispatches on detector id values {m:?}: consumers match on detector-declared \
                 capabilities, never on ids",
                f.display()
            ));
        }
    }
    for d in &p.items {
        if !inside(&d.rel) && let Some((n, _)) = ids.iter().find(|(n, _)| contains_token(&d.value, n)) {
            problems.push(format!("{}::{} names the detector id constant `{n}` at item level", d.rel, d.name));
        }
    }
    if p.named("manager_conventions").is_empty() {
        problems.push("`Detector` has no `manager_conventions()` capability for wiring to match on".into());
    }
    verdict("detector ids live only in the detector registry", problems)
}

pub fn tui_refresh_preserves_scope(root: &Path) -> Result<(), String> {
    let p = load(root);
    let mut problems = Vec::new();
    let run = p.defs("bus::run_report");
    let report_files: HashSet<String> = p
        .funs
        .iter()
        .enumerate()
        .filter(|(i, _)| p.callees(*i).iter().any(|g| run.contains(g)))
        .map(|(_, f)| f.rel.clone())
        .collect();
    if report_files.is_empty() {
        problems.push("nothing runs the bus: there is no report module to audit calls into".into());
    }
    // A scopeless report entry: a public function of the report module
    // that takes a bare path and no scope.
    // A scopeless entry: a public function of the report module that
    // takes a bare root (one path, not a list) and no scope -- unless it
    // only loads a stored report back (reads a file, returns the report),
    // which carries whatever scope computed it.
    let reads = p.unbounded_reads(&HashSet::new());
    let scopeless = |g: usize| {
        let f = &p.funs[g];
        let bare_root = f.params.iter().any(|(_, t)| {
            let t = t.replace(' ', "");
            (t.ends_with("Path") || t.ends_with("PathBuf")) && !t.contains('[') && !t.contains("Vec<")
        });
        let opens = reads.contains(&g) || f.calls.iter().any(|c| c.is("File::open"));
        let cache_load = f.ret.contains("Report") && opens && !p.destructive().contains(&g) && !p.traversal(&HashSet::new()).contains(&g);
        f.is_pub
            && report_files.contains(&f.rel)
            && bare_root
            && !cache_load
            && !f.params.iter().any(|(_, t)| t.contains("Scope"))
    };
    for (i, f) in p.funs.iter().enumerate() {
        if f.krate != "swamp_tui" {
            continue;
        }
        for (ci, c) in f.calls.iter().enumerate() {
            let t = p.target(i, ci);
            if t.possible {
                continue;
            }
            if let Some(g) = t.local.iter().find(|g| scopeless(**g)) {
                problems.push(format!(
                    "{} refreshes through `{}`, which takes a bare root and no EffectiveScope: \
                     excluded subtrees and pruned locations reappear on refresh",
                    f.display(),
                    p.funs[*g].path()
                ));
            }
            let _ = Honoured::Yes;
        }
    }
    verdict("a TUI refresh keeps its effective scope", problems)
}
