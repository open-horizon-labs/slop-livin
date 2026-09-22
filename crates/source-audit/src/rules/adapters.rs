//! The agent-adapter guardrails (`GUARDRAILS_SPEC.md` sections 13 and 14).
//!
//! **Scope, derived.** Adapters are every module under `agents/` and
//! `build_adapters/` except the shared model (`mod.rs`), the static
//! registration (`registry.rs`) and the tool catalog (`matrix.rs`)
//! (`Program::adapter_files`). The shared family and bounded-read
//! mechanics -- `vscode_family.rs`, `pi_family.rs`, `bounded_io.rs` --
//! are adapters: they are six tools' identification code, and re-review
//! 3 put two of its mutations there because they used to be exempt.
//!
//! **Behaviour, derived.** A rule asks whether an adapter function is in
//! a derived set (reads unbounded, traverses, mutates, emits, reads the
//! environment), which follows every callee in the program. What stops
//! the closure is exactly two things:
//!
//! * the **bounded primitives**, each named by resolved path with the cap
//!   constant that earns its exemption (a primitive that stops naming or
//!   stopping on its cap loses the exemption);
//! * the **declared-project handoff**: a function returning
//!   `ProjectLinkState` resolves a path an adapter read *out of a header*
//!   -- the user's project, not the tool's home -- through the walker's
//!   own `git`/`ecosystem` identity code, which the report-path
//!   guardrails govern. A handoff function in the adapter tree may still
//!   not use the capability itself.

use super::{load, verdict};
use crate::program::{self, DeclKind, Program, TypeKind, contains_token};
use std::collections::HashSet;
use std::path::Path;

/// `(resolved path, cap it must name and stop on)`.
const BOUNDED_READERS: &[(&str, &str)] = &[("agents::bounded_io::read_header", "MAX_HEADER_BYTES")];
const BOUNDED_LISTERS: &[(&str, &str)] = &[
    ("locations::shallow_list", "SHALLOW_LIST_CAP"),
    (
        "folded_measurement::folded_bytes_bounded_stamped",
        "max_entries",
    ),
];

/// The type an adapter's declared-project resolution returns.
const PROJECT_LINK_TYPE: &str = "ProjectLinkState";

fn bounded(p: &Program, table: &[(&str, &str)], problems: &mut Vec<String>) -> HashSet<usize> {
    let reader = table.iter().any(|(a, _)| a.contains("read_header"));
    super::bounded_primitives(p, table, reader, problems)
}

/// Functions that hand a declared project path to the walker's identity
/// code.
fn handoff(p: &Program) -> HashSet<usize> {
    p.funs
        .iter()
        .enumerate()
        .filter(|(_, f)| contains_token(&f.ret, PROJECT_LINK_TYPE))
        .map(|(i, _)| i)
        .collect()
}

/// Every adapter function (and adapter-tree handoff) that is in `set`,
/// with the call chain that puts it there.
fn members(
    p: &Program,
    set: &HashSet<usize>,
    direct: impl Fn(usize) -> bool,
    what: &str,
) -> Vec<String> {
    let mut out = Vec::new();
    for i in p.adapter_funs() {
        if set.contains(&i) {
            out.push(format!(
                "{} {what} ({})",
                p.funs[i].display(),
                p.chain(i, set, &direct)
            ));
        }
    }
    out
}

/// Handoff functions written in the agent tree must not use the
/// capability themselves: the exemption is for what lies beyond them.
fn handoffs_direct(
    p: &Program,
    stop: &HashSet<usize>,
    pred: impl Fn(&program::Fun, &program::PCall) -> bool,
    what: &str,
) -> Vec<String> {
    stop.iter()
        .filter(|i| p.funs[**i].rel.starts_with("crates/core/src/agents/"))
        .filter(|i| p.funs[**i].calls.iter().any(|c| pred(&p.funs[**i], c)))
        .map(|i| format!("{} {what} directly", p.funs[*i].display()))
        .collect()
}

pub fn agent_adapters_read_bounded_headers_only(root: &Path) -> Result<(), String> {
    let p = load(root);
    let mut problems = Vec::new();
    let mut stop = bounded(&p, BOUNDED_READERS, &mut problems);
    let h = handoff(&p);
    problems.extend(handoffs_direct(
        &p,
        &h,
        |_, c| program::unbounded_read(c),
        "reads a whole file",
    ));
    stop.extend(h);
    let set = p.unbounded_reads(&stop);
    problems.extend(members(
        &p,
        &set,
        |g| p.funs[g].calls.iter().any(program::unbounded_read),
        "reads a whole file",
    ));
    // The cap itself: at most 64 KiB.
    match p.const_value("MAX_HEADER_BYTES").map(eval_product) {
        Some(Some(n)) if n <= 64 * 1024 => {}
        Some(Some(n)) => problems.push(format!(
            "MAX_HEADER_BYTES is {n}, above the 64 KiB contract"
        )),
        _ => problems.push("MAX_HEADER_BYTES is not a constant this audit can evaluate".into()),
    }
    verdict(
        "an adapter's only content access is `bounded_io::read_header(path, max_bytes)`, capped; \
         nothing it reaches reads a whole file (privacy is a hard contract)",
        problems,
    )
}

/// `64 * 1024`, `65_536`: a product of integer literals.
fn eval_product(v: &str) -> Option<u64> {
    v.split('*')
        .map(|t| t.trim().replace('_', "").parse::<u64>().ok())
        .try_fold(1u64, |acc, x| x.map(|x| acc.saturating_mul(x)))
}

pub fn agent_adapters_do_not_traverse(root: &Path) -> Result<(), String> {
    let p = load(root);
    let mut problems = Vec::new();
    let mut stop = bounded(&p, BOUNDED_LISTERS, &mut problems);
    let h = handoff(&p);
    problems.extend(handoffs_direct(
        &p,
        &h,
        |_, c| program::traversal_call(c),
        "enumerates a directory",
    ));
    stop.extend(h);
    let set = p.traversal(&stop);
    problems.extend(members(
        &p,
        &set,
        |g| p.funs[g].calls.iter().any(program::traversal_call),
        "enumerates a directory",
    ));
    verdict(
        "directory structure reaches an adapter through folded walk rows, the capped \
         `locations::shallow_list` or the bounded fold; nothing an adapter reaches traverses \
         on its own",
        problems,
    )
}

pub fn agent_adapters_are_inspection_only(root: &Path) -> Result<(), String> {
    let p = load(root);
    let set = p.mutating();
    let problems = members(
        &p,
        &set,
        |g| {
            p.funs[g].calls.iter().any(|c| {
                program::destructive_call(&p.funs[g], c)
                    || c.is("fs::create_dir")
                    || c.is("fs::create_dir_all")
            })
        },
        "writes, deletes, moves or spawns",
    );
    verdict(
        "identification never acts: nothing an adapter reaches writes, truncates, deletes, moves \
         or runs a process",
        problems,
    )
}

pub fn agent_adapters_do_not_emit_content(root: &Path) -> Result<(), String> {
    let p = load(root);
    let set = p.emitters();
    let problems = members(
        &p,
        &set,
        |g| {
            p.funs[g].calls.iter().any(program::emitter_call)
                || p.funs[g].macros.iter().any(program::macro_emits)
        },
        "puts bytes on a terminal or a log",
    );
    verdict(
        "adapters return data and the shared layer renders it through the redaction-aware path; \
         nothing an adapter reaches prints, logs, or panics with a formatted message",
        problems,
    )
}

pub fn agent_adapters_are_environment_free(root: &Path) -> Result<(), String> {
    let p = load(root);
    let set = p.env_readers();
    let mut problems = members(
        &p,
        &set,
        |g| p.funs[g].calls.iter().any(program::env_call),
        "reads the environment",
    );
    let hardcoded = |l: &str| l == "HOME" || l.starts_with("/Users/") || l.starts_with("/home/");
    for i in p.adapter_funs() {
        let f = &p.funs[i];
        if let Some(l) = f.literals.iter().find(|l| hardcoded(l)) {
            problems.push(format!("{} hardcodes the home path {l:?}", f.display()));
        }
    }
    for d in p.adapter_items() {
        if let Some(l) = d.literals.iter().find(|l| hardcoded(l)) {
            problems.push(format!(
                "{}::{} hardcodes the home path {l:?}",
                d.rel, d.name
            ));
        }
    }
    verdict(
        "the home path arrives from the detector through the registry context, so a fixture can \
         always inject one; no adapter reads the environment",
        problems,
    )
}

pub fn agent_adapters_do_not_reach_detectors(root: &Path) -> Result<(), String> {
    let p = load(root);
    let mut problems = Vec::new();
    // Shared vocabulary: the data types declared in `locations` itself,
    // and the bounded lister. Everything else under `locations` --
    // a detector module, a detector id, the `Detector` trait, a
    // resolution function -- is detector identity.
    // Vocabulary is what the shared agent model itself carries: a type
    // declared in `locations` that a field of an `agents/mod.rs` type
    // names. `Environment`, `Registry` and `Detector` are how homes are
    // resolved; no unit carries them.
    let model_fields: Vec<&str> = p
        .items
        .iter()
        .filter(|d| d.kind == DeclKind::Field && d.rel == "crates/core/src/agents/mod.rs")
        .map(|d| d.ty.as_str())
        .collect();
    let vocabulary: HashSet<String> = p
        .types
        .iter()
        .filter(|t| t.krate == "swamp_core" && t.module == "locations" && t.kind != TypeKind::Trait)
        .filter(|t| model_fields.iter().any(|ty| contains_token(ty, &t.name)))
        .map(|t| t.name.clone())
        .chain(
            BOUNDED_LISTERS
                .iter()
                .filter(|(a, _)| a.starts_with("locations::"))
                .map(|(a, _)| a.rsplit("::").next().unwrap_or(a).to_string()),
        )
        .collect();
    let offending = |text: &str| -> Option<String> {
        super::named_paths(text).into_iter().find(|path| {
            let segs: Vec<&str> = path.split("::").collect();
            let Some(at) = segs.iter().position(|s| *s == "locations") else {
                return false;
            };
            // `swamp_core::locations::X` or `crate::locations::X`.
            let prefix_ok = at == 0 || matches!(segs[at - 1], "crate" | "swamp_core" | "super");
            let next = segs.get(at + 1).copied().unwrap_or("");
            prefix_ok && !next.is_empty() && !vocabulary.contains(next)
        })
    };
    for i in p.adapter_funs() {
        let f = &p.funs[i];
        if let Some(path) = offending(&format!("{} {}", f.sig, f.body)) {
            problems.push(format!("{} names `{path}`", f.display()));
        }
    }
    // Item level: a const, a type alias or a struct field sits inside no
    // function, and re-review 3 put a detector id there.
    for d in p.adapter_items() {
        if let Some(path) = offending(&format!("{} {}", d.ty, d.value)) {
            problems.push(format!(
                "{}::{} names `{path}` at item level",
                d.rel, d.name
            ));
        }
    }
    verdict(
        "detector ids and home resolution live in `locations/`; an adapter knows only the home it \
         is handed and the shared vocabulary types",
        problems,
    )
}

/// Every adapter proves the same five things about itself.
pub const REQUIRED_ADAPTER_TESTS: &[&str] = &[
    "unknown_format_is_explicit_not_empty",
    "canary_content_never_appears_in_output",
    "identification_reads_no_more_than_header_cap",
    "protected_categories_default_protected",
    "project_link_is_declared_or_unresolved_never_basename_guess",
];

/// `#[test]` functions in `file`'s `#[cfg(test)]` modules that are not
/// `#[ignore]`d and assert something (an `assert*!`, a `panic!`, an
/// `unwrap_err`, or a call into a shared `contract` helper). Parsed, not
/// grepped: a name in a doc comment, a string or a comment is not a test.
pub fn running_tests(file: &syn::File) -> Vec<String> {
    use quote::ToTokens;
    use syn::visit::Visit;
    struct V {
        in_test_mod: usize,
        out: Vec<String>,
    }
    impl<'ast> Visit<'ast> for V {
        fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
            let t = m.attrs.iter().any(|a| {
                a.to_token_stream()
                    .to_string()
                    .replace(' ', "")
                    .contains("cfg(test)")
            });
            if t {
                self.in_test_mod += 1;
            }
            syn::visit::visit_item_mod(self, m);
            if t {
                self.in_test_mod -= 1;
            }
        }
        fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
            let is_test = f.attrs.iter().any(|a| a.path().is_ident("test"));
            let ignored = f.attrs.iter().any(|a| a.path().is_ident("ignore"));
            let body = f.block.to_token_stream().to_string();
            let asserts = [
                "assert !",
                "assert_eq !",
                "assert_ne !",
                "panic !",
                "unwrap_err",
                "contract ::",
                "expect_err",
            ]
            .iter()
            .any(|a| body.contains(a));
            if self.in_test_mod > 0 && is_test && !ignored && asserts {
                self.out.push(f.sig.ident.to_string());
            }
            syn::visit::visit_item_fn(self, f);
        }
    }
    let mut v = V {
        in_test_mod: 0,
        out: Vec::new(),
    };
    v.visit_file(file);
    v.out
}

pub fn agent_adapter_test_contract(root: &Path) -> Result<(), String> {
    let p = load(root);
    let mut problems = Vec::new();
    for rel in p.tool_modules() {
        let Some(file) = p.file(&rel) else { continue };
        let have = running_tests(&file.ast);
        let absent: Vec<&str> = REQUIRED_ADAPTER_TESTS
            .iter()
            .copied()
            .filter(|t| !have.iter().any(|h| h == t))
            .collect();
        if !absent.is_empty() {
            problems.push(format!("{rel}: missing {}", absent.join(", ")));
        }
    }
    verdict(
        "every module that declares a tool proves the same five things about itself, as \
         running, asserting #[test] functions",
        problems,
    )
}

pub fn agent_adapters_are_pluggable(root: &Path) -> Result<(), String> {
    let p = load(root);
    let mut problems = Vec::new();
    let tools = p.tool_modules();
    // A tool module's children are that tool's code.
    let tool_family = |rel: &str| -> Option<String> {
        tools
            .iter()
            .find(|t| {
                *t == rel || {
                    let m = Program::file_module(t);
                    Program::file_module(rel).starts_with(&format!("{m}::"))
                }
            })
            .cloned()
    };
    if tools.is_empty() {
        problems.push("no module under agents/ declares a tool".into());
    }
    let tool_module: Vec<(String, String)> = tools
        .iter()
        .map(|rel| (rel.clone(), Program::file_module(rel)))
        .collect();
    let module_of = |abs: &str| -> Option<&String> {
        tool_module
            .iter()
            .find(|(_, m)| abs == m || abs.starts_with(&format!("{m}::")))
            .map(|(rel, _)| rel)
    };
    // (1) No tool module reaches into another: by resolved call, by value
    // reference, or by a path in its body, signature or items.
    for (i, f) in p.funs.iter().enumerate() {
        let Some(me) = tool_family(&f.rel) else {
            continue;
        };
        let mut reached: Vec<String> = Vec::new();
        for (ci, _) in f.calls.iter().enumerate() {
            let t = p.target(i, ci);
            for g in &t.local {
                reached.push(p.funs[*g].rel.clone());
            }
            if let Some(rel) = module_of(&t.abs) {
                reached.push(rel.clone());
            }
        }
        for g in p.ref_targets(i) {
            reached.push(p.funs[*g].rel.clone());
        }
        for path in super::named_paths(&format!("{} {}", f.sig, f.body)) {
            let (abs, _) = program::absolute(&path, &f.krate, &f.module, None);
            if let Some(rel) = module_of(&abs) {
                reached.push(rel.clone());
            }
        }
        if let Some(other) = reached
            .iter()
            .filter_map(|r| tool_family(r))
            .find(|t| *t != me)
        {
            problems.push(format!(
                "{} reaches into adapter {other}: one tool's format change must never change \
                 another tool's identification",
                f.display()
            ));
        }
    }
    // A re-export written in a tool module is a reach too.
    for r in &p.reexports {
        if let Some(me) = tool_family(&r.rel)
            && let Some(other) = module_of(&r.target).and_then(|rel| tool_family(rel))
            && other != me
        {
            problems.push(format!(
                "{} re-exports `{}` from adapter {other}",
                r.rel, r.target
            ));
        }
    }
    for d in &p.items {
        if !tools.contains(&d.rel) {
            continue;
        }
        for path in super::named_paths(&format!("{} {}", d.ty, d.value)) {
            let (abs, _) = program::absolute(&path, &d.krate, &d.module, None);
            if let Some(rel) = module_of(&abs)
                && *rel != d.rel
            {
                problems.push(format!(
                    "{}::{} names adapter {rel} at item level",
                    d.rel, d.name
                ));
            }
        }
    }
    // (2) No central dispatch on tool ids anywhere in the workspace. The
    // ids are derived: every tool module's `*_TOOL_ID` constant, by name
    // and by the value it holds.
    let ids: Vec<(String, String)> = p
        .items
        .iter()
        .filter(|d| {
            tools.contains(&d.rel) && d.kind == DeclKind::Const && d.name.ends_with("_TOOL_ID")
        })
        .map(|d| {
            (
                d.name.clone(),
                d.literals.first().cloned().unwrap_or_default(),
            )
        })
        .collect();
    let catalog =
        |rel: &str| rel.ends_with("agents/registry.rs") || rel.ends_with("agents/matrix.rs");
    for f in p.funs.iter() {
        if catalog(&f.rel) {
            continue;
        }
        let named: HashSet<&str> = ids
            .iter()
            .filter(|(n, v)| {
                contains_token(&f.body, n)
                    || (!v.is_empty()
                        && f.arms
                            .iter()
                            .any(|a| a.pattern.contains(&format!("\"{v}\""))))
            })
            .map(|(n, _)| n.as_str())
            .collect();
        // A `match` arm on a tool id outside the tool's own module is a
        // dispatch table, however many arms it has.
        let own = |n: &str| p.items.iter().any(|d| d.name == n && d.rel == f.rel);
        if let Some((n, _)) = ids.iter().find(|(n, v)| {
            !own(n)
                && f.arms.iter().any(|a| {
                    contains_token(&a.pattern, n)
                        || (!v.is_empty() && a.pattern.contains(&format!("\"{v}\"")))
                })
        }) {
            problems.push(format!(
                "{} matches on the tool id `{n}`: adapter dispatch goes through `agents::Registry`",
                f.display()
            ));
        }
        if named.len() >= 2 {
            let mut named: Vec<&str> = named.into_iter().collect();
            named.sort();
            problems.push(format!(
                "{} dispatches on tool ids {named:?}: adapter dispatch goes through \
                 `agents::Registry`, never a central tool-id table",
                f.display()
            ));
        }
    }
    // (3) The registry registers each tool module exactly once.
    let registry_files = p.family(
        &p.files
            .iter()
            .filter(|f| f.rel.ends_with("agents/registry.rs"))
            .map(|f| f.rel.clone())
            .collect(),
    );
    let registry: Vec<&program::Fun> = p
        .funs
        .iter()
        .filter(|f| registry_files.contains(&f.rel))
        .map(|f| &**f)
        .collect();
    if registry.is_empty() {
        problems.push("agents/registry.rs defines no function: section 13 requires a static `Registry::with_builtins()`".into());
    }
    for rel in &tools {
        let name = Program::module_name(rel);
        let needle = format!("{name} :: Adapter");
        let count: usize = registry
            .iter()
            .map(|f| {
                let mut n = 0;
                let mut start = 0;
                while let Some(at) = f.body[start..].find(&needle) {
                    let abs = start + at;
                    let before = f.body[..abs].chars().next_back();
                    if before.is_none_or(|c| !(c.is_alphanumeric() || c == '_')) {
                        n += 1;
                    }
                    start = abs + needle.len();
                }
                n
            })
            .sum();
        if count != 1 {
            problems.push(format!(
                "agents/registry.rs registers `{name}` {count} times; exactly once"
            ));
        }
    }
    // (4) The catalog exposes ids to compare against.
    let matrix_ids = p
        .funs
        .iter()
        .filter(|f| f.rel.ends_with("agents/matrix.rs"))
        .flat_map(|f| f.literals.iter())
        .chain(
            p.items
                .iter()
                .filter(|d| d.rel.ends_with("agents/matrix.rs"))
                .flat_map(|d| d.literals.iter()),
        )
        .any(|s| s.chars().all(|c| c.is_ascii_lowercase() || c == '-') && s.contains('-'));
    if !matrix_ids.then_some(()).is_some() {
        problems.push("agents/matrix.rs exposes no tool ids to compare with the registry".into());
    }
    verdict(
        "adapters are independent and statically registered: none reaches another, nothing \
         dispatches on their ids, and the registry names each exactly once",
        problems,
    )
}

pub fn agent_units_built_through_builder(root: &Path) -> Result<(), String> {
    let p = load(root);
    let mut problems = Vec::new();
    // The unit types are the ones the builder builds.
    let builder_ok = p.types.iter().any(|t| t.name == "AgentUnitBuilder");
    if !builder_ok {
        problems.push("agents/mod.rs does not define `AgentUnitBuilder`".into());
    }
    let unit_types = ["CandidateAgentUnit", "AgentUnit"];
    // Fields the builder decides from the category: every field of a
    // unit type whose name the builder's constructor sets from its
    // defaults. Derived from the builder's own struct literals.
    let mut decided: HashSet<String> = HashSet::new();
    for f in p
        .funs
        .iter()
        .filter(|f| f.self_ty.as_deref() == Some("AgentUnitBuilder"))
    {
        // What the constructor derives from the category it was given.
        let from_category =
            crate::resolve::derived_from(&f.bindings, &f.name, &["category".to_string()]);
        for l in &f.struct_lits {
            if unit_types
                .iter()
                .any(|u| crate::resolve::path_ends_with(&l.path, u))
            {
                for (field, init) in &l.fields {
                    let root = crate::resolve::root_ident(init);
                    if root != "category" && from_category.contains(&root) {
                        decided.insert(field.clone());
                    }
                }
            }
        }
    }
    for i in p.adapter_funs() {
        let f = &p.funs[i];
        for l in &f.struct_lits {
            if let Some(u) = unit_types
                .iter()
                .find(|u| crate::resolve::path_ends_with(&l.path, u))
            {
                problems.push(format!(
                    "{} builds a `{u} {{ .. }}` literal: units are built with \
                     `AgentUnitBuilder::new(tool, category, path)`, whose constructor applies the \
                     protected-by-default categories a literal can omit",
                    f.display()
                ));
            }
        }
        // A unit built correctly and then unprotected field by field is
        // the same silent unprotect.
        for a in &f.assigns {
            let field = a.lhs.rsplit('.').next().unwrap_or("").trim().to_string();
            if decided.contains(&field) {
                problems.push(format!(
                    "{} writes `{}` after the builder decided it: lifting protection is \
                     `unprotect_with_reason`, stated and reviewed",
                    f.display(),
                    a.lhs.replace(' ', "")
                ));
            }
        }
        for c in &f.calls {
            if c.method
                && c.path == "unprotect_with_reason"
                && c.args.iter().all(|a| super::empty_text(&p, a))
            {
                problems.push(format!(
                    "{} lifts protected-by-default with no stated reason",
                    f.display()
                ));
            }
        }
    }
    if decided.is_empty() && builder_ok {
        problems.push(
            "AgentUnitBuilder no longer decides any protection field from the category".into(),
        );
    }
    verdict(
        "agent units are built through the builder and its protection decision stands",
        problems,
    )
}
