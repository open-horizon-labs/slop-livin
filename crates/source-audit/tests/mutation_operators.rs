//! Mutation **operators**, not hand fixtures (CHUNK_R7 Part 2).
//!
//! The corpus (`mutation_corpus.rs`) proves an audit rejects the
//! fixtures someone wrote for it, which is exactly what re-review 3
//! falsified: 43 of 45 audits accepted the first new mutation a reviewer
//! wrote. So this file does not write mutations. It takes every *seed*
//! the corpus already has -- the rejected fixtures, including all 45
//! re-review 3 sweep mutations, and the accepted (legitimate) shapes --
//! and derives variants mechanically, with operators that change how a
//! harmful thing is *written* without changing what it *does*:
//!
//! | operator | what it does to a rejected seed |
//! |---|---|
//! | `alias` | a called path imported under another name (`use a::b::f as g`) |
//! | `pub_use` | the same call routed through a local re-export module |
//! | `helper` | each function's body moved into a same-file helper it calls |
//! | `child_module` | the whole mutation moved one file down, into a child module |
//! | `macro_wrap` | every discarded call statement written inside `vec![..]` |
//! | `via_constant` | every string literal hoisted into a `const` |
//! | `exempt_helper` | the harmful statements added to the bounded primitive the rule exempts |
//!
//! and to accepted (legitimate) seeds:
//!
//! | `discard` | the first honoured call (`?`, `match`) turned into `let _ = ..` -- must now be rejected |
//! | `alias`, `pub_use`, `helper`, `via_constant` | the same spelling changes -- must still be accepted |
//!
//! Every variant must be **rejected** by the seed's audit. A variant the
//! operator cannot build (no call to alias, no literal to hoist) is not
//! generated, and the counts per operator are printed so a narrowing is
//! visible.

use quote::{ToTokens, quote};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use swamp_source_audit::audits::{AUDITS, Audit, TEST_ONLY_RULES};
use syn::visit_mut::VisitMut;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn rule(name: &str) -> Option<Audit> {
    AUDITS
        .iter()
        .chain(TEST_ONLY_RULES.iter())
        .find(|(n, _)| *n == name)
        .map(|(_, a)| *a)
}

const COPIED: &[&str] = &["crates", ".oh", "docs", "scripts", "Cargo.toml"];

fn copy_tree(from: &Path, to: &Path) {
    if from.is_file() {
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        std::fs::copy(from, to).unwrap();
        return;
    }
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let name = e.file_name();
        if name == "target" || name == ".git" {
            continue;
        }
        copy_tree(&e.path(), &to.join(&name));
    }
}

#[derive(Clone)]
struct Seed {
    audit: String,
    name: String,
    expect: String,
    /// `(target, mode, body)`.
    files: Vec<(String, String, String)>,
}

/// The corpus fixture format (see `mutation_corpus.rs`), read the same
/// way.
fn parse_seed(audit: &str, path: &Path) -> Seed {
    let text = std::fs::read_to_string(path).unwrap();
    let mut header: BTreeMap<String, String> = BTreeMap::new();
    let mut files: Vec<(String, String, String)> = Vec::new();
    let mut body = String::new();
    let mut in_header = true;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("//! ")
            && let Some((k, v)) = rest.split_once(':')
        {
            let (k, v) = (k.trim(), v.trim());
            if in_header && ["target", "mode", "expect", "why"].contains(&k) {
                header.insert(k.to_string(), v.to_string());
                continue;
            }
            if k == "file" {
                if files.is_empty() {
                    files.push((
                        header.get("target").cloned().unwrap_or_default(),
                        header.get("mode").cloned().unwrap_or_else(|| "append".into()),
                        std::mem::take(&mut body),
                    ));
                } else {
                    files.last_mut().unwrap().2 = std::mem::take(&mut body);
                }
                files.push((v.to_string(), "append".into(), String::new()));
                in_header = false;
                continue;
            }
            if k == "mode" && !in_header && body.is_empty() {
                files.last_mut().unwrap().1 = v.to_string();
                continue;
            }
        }
        in_header = false;
        body.push_str(line);
        body.push('\n');
    }
    if files.is_empty() {
        files.push((
            header.get("target").cloned().unwrap_or_default(),
            header.get("mode").cloned().unwrap_or_else(|| "append".into()),
            body,
        ));
    } else {
        files.last_mut().unwrap().2 = body;
    }
    Seed {
        audit: audit.to_string(),
        name: path.file_name().unwrap().to_string_lossy().into_owned(),
        expect: header.get("expect").cloned().unwrap_or_else(|| "reject".into()),
        files,
    }
}

fn seeds() -> Vec<Seed> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/mutations");
    let mut out = Vec::new();
    for a in std::fs::read_dir(&dir).unwrap().flatten() {
        if !a.path().is_dir() {
            continue;
        }
        let audit = a.file_name().to_string_lossy().into_owned();
        let mut fs: Vec<PathBuf> = std::fs::read_dir(a.path())
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("rs"))
            .collect();
        fs.sort();
        for f in fs {
            out.push(parse_seed(&audit, &f));
        }
    }
    out.sort_by(|a, b| (&a.audit, &a.name).cmp(&(&b.audit, &b.name)));
    out
}

/// The product crates' dependency names: a path starting with one is
/// absolute from any module.
fn dependency_crates() -> Vec<String> {
    let mut out = Vec::new();
    for k in ["core", "cli", "tui"] {
        let text = std::fs::read_to_string(repo_root().join(format!("crates/{k}/Cargo.toml"))).unwrap_or_default();
        let mut on = false;
        for line in text.lines() {
            let t = line.trim();
            if t.starts_with('[') {
                on = t == "[dependencies]";
                continue;
            }
            if on && let Some((name, _)) = t.split_once(['=', '.']) {
                out.push(name.trim().replace('-', "_"));
            }
        }
    }
    out
}

// ---------------------------------------------------------------------
// Operators
// ---------------------------------------------------------------------

/// A called path worth renaming: at least two segments, a function (its
/// last segment is lower case), not a variant or constructor.
fn renameable(path: &syn::Path) -> bool {
    let segs: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
    segs.len() >= 2
        && segs.last().is_some_and(|l| l.chars().next().is_some_and(|c| c.is_ascii_lowercase()))
        && !matches!(segs[0].as_str(), "Self" | "Some" | "Ok" | "Err" | "Box" | "Vec" | "String")
        // `use Type::f` is not Rust: an associated function cannot be
        // imported, so only module paths are renamed.
        && !segs[segs.len() - 2].chars().next().is_some_and(|c| c.is_ascii_uppercase())
        && path.segments.iter().all(|s| s.arguments.is_none())
}

/// The first renameable call path in a function body.
fn first_call(block: &syn::Block) -> Option<syn::Path> {
    struct F(Option<syn::Path>);
    impl syn::visit::Visit<'_> for F {
        fn visit_expr_call(&mut self, c: &syn::ExprCall) {
            if self.0.is_none()
                && let syn::Expr::Path(p) = &*c.func
                && renameable(&p.path)
            {
                self.0 = Some(p.path.clone());
            }
            syn::visit::visit_expr_call(self, c);
        }
    }
    let mut f = F(None);
    syn::visit::visit_block(&mut f, block);
    f.0
}

/// Replaces every call to `from` in a body with a call to `to`.
struct Retarget {
    from: String,
    to: syn::Path,
}
impl VisitMut for Retarget {
    fn visit_expr_call_mut(&mut self, c: &mut syn::ExprCall) {
        if let syn::Expr::Path(p) = &mut *c.func
            && p.path.to_token_stream().to_string() == self.from
        {
            p.path = self.to.clone();
        }
        syn::visit_mut::visit_expr_call_mut(self, c);
    }
}

/// Applies `edit` to every item list (the file and each inline module's
/// contents), so an inserted `use` or helper lands in the same scope as
/// the function it serves.
fn each_scope(items: &mut Vec<syn::Item>, edit: &mut dyn FnMut(&mut Vec<syn::Item>) -> bool) -> bool {
    let mut changed = edit(items);
    for it in items.iter_mut() {
        if let syn::Item::Mod(m) = it
            && let Some((_, inner)) = &mut m.content
        {
            changed |= each_scope(inner, edit);
        }
    }
    changed
}

fn fns_in(items: &mut [syn::Item]) -> Vec<&mut syn::ItemFn> {
    items
        .iter_mut()
        .filter_map(|i| match i {
            syn::Item::Fn(f) => Some(f),
            _ => None,
        })
        .collect()
}

fn op_alias(file: &mut syn::File, via_shim: bool) -> bool {
    let mut done = false;
    each_scope(&mut file.items, &mut |items| {
        if done {
            return false;
        }
        let mut target: Option<syn::Path> = None;
        for f in fns_in(items) {
            if let Some(p) = first_call(&f.block) {
                target = Some(p);
                break;
            }
        }
        let Some(path) = target else { return false };
        let from = path.to_token_stream().to_string();
        let (to, added): (syn::Path, syn::Item) = if via_shim {
            // Inside the shim, a relative path is one level further away.
            let written = from.replace(' ', "");
            let first = written.split("::").next().unwrap_or("").to_string();
            let external = dependency_crates().contains(&first);
            let inner = match first.as_str() {
                _ if external => written.clone(),
                "crate" | "std" | "swamp_core" | "swamp_tui" => written.clone(),
                "super" => format!("super::{written}"),
                "self" => written.replacen("self", "super", 1),
                _ => format!("super::{written}"),
            };
            let inner: syn::Path = syn::parse_str(&inner).unwrap();
            (
                syn::parse_quote!(sweep_op_shim::sweep_op_g),
                syn::parse_quote!(mod sweep_op_shim { pub use #inner as sweep_op_g; }),
            )
        } else {
            (
                syn::parse_quote!(sweep_op_alias),
                syn::parse_quote!(use #path as sweep_op_alias;),
            )
        };
        let mut r = Retarget { from, to };
        for f in fns_in(items) {
            r.visit_block_mut(&mut f.block);
        }
        items.insert(0, added);
        done = true;
        true
    })
}

fn op_helper(file: &mut syn::File) -> bool {
    each_scope(&mut file.items, &mut |items| {
        let mut added: Vec<syn::Item> = Vec::new();
        for f in fns_in(items) {
            let simple = f.sig.inputs.iter().all(|a| matches!(a, syn::FnArg::Typed(t) if matches!(&*t.pat, syn::Pat::Ident(_))));
            if !simple || f.block.stmts.is_empty() || f.sig.asyncness.is_some() {
                continue;
            }
            let helper = syn::Ident::new(&format!("sweep_op_helper_{}", f.sig.ident), f.sig.ident.span());
            let mut sig = f.sig.clone();
            sig.ident = helper.clone();
            let block = f.block.clone();
            added.push(syn::Item::Fn(syn::ItemFn {
                attrs: Vec::new(),
                vis: syn::Visibility::Inherited,
                sig,
                block,
            }));
            let args: Vec<syn::Ident> = f
                .sig
                .inputs
                .iter()
                .filter_map(|a| match a {
                    syn::FnArg::Typed(t) => match &*t.pat {
                        syn::Pat::Ident(i) => Some(i.ident.clone()),
                        _ => None,
                    },
                    _ => None,
                })
                .collect();
            *f.block = syn::parse_quote!({ #helper(#(#args),*) });
        }
        let changed = !added.is_empty();
        items.extend(added);
        changed
    })
}

/// Every discarded call statement (`f(..);`, `let _ = f(..);`) written
/// inside `vec![..]`, still discarded.
fn op_macro_wrap(file: &mut syn::File) -> bool {
    struct W(bool);
    impl VisitMut for W {
        fn visit_block_mut(&mut self, b: &mut syn::Block) {
            for s in b.stmts.iter_mut() {
                let replacement: Option<syn::Stmt> = match s {
                    syn::Stmt::Expr(e @ (syn::Expr::Call(_) | syn::Expr::MethodCall(_) | syn::Expr::Try(_)), Some(_)) => {
                        Some(syn::parse_quote!(let _ = vec![#e];))
                    }
                    syn::Stmt::Local(l)
                        if matches!(l.pat, syn::Pat::Wild(_))
                            && l.init.as_ref().is_some_and(|i| i.diverge.is_none()) =>
                    {
                        let e = &l.init.as_ref().unwrap().expr;
                        Some(syn::parse_quote!(let _ = vec![#e];))
                    }
                    _ => None,
                };
                if let Some(r) = replacement {
                    *s = r;
                    self.0 = true;
                }
            }
            syn::visit_mut::visit_block_mut(self, b);
        }
    }
    let mut w = W(false);
    w.visit_file_mut(file);
    w.0
}

/// Every string literal in a function body hoisted into a `const` in
/// the same scope.
fn op_via_constant(file: &mut syn::File) -> bool {
    struct H {
        n: usize,
        consts: Vec<(syn::Ident, syn::LitStr)>,
    }
    impl VisitMut for H {
        fn visit_expr_mut(&mut self, e: &mut syn::Expr) {
            if let syn::Expr::Lit(l) = e
                && let syn::Lit::Str(s) = &l.lit
            {
                let id = syn::Ident::new(&format!("SWEEP_OP_C{}", self.n), s.span());
                self.n += 1;
                self.consts.push((id.clone(), s.clone()));
                *e = syn::parse_quote!(#id);
                return;
            }
            syn::visit_mut::visit_expr_mut(self, e);
        }
    }
    let mut n = 0usize;
    each_scope(&mut file.items, &mut |items| {
        let mut h = H { n, consts: Vec::new() };
        for f in fns_in(items) {
            h.visit_block_mut(&mut f.block);
        }
        n = h.n;
        let changed = !h.consts.is_empty();
        for (id, lit) in h.consts {
            items.insert(0, syn::parse_quote!(const #id: &str = #lit;));
        }
        changed
    })
}

/// The first honoured call (`x?`, `match x`) of a legitimate shape turned
/// into `let _ = x;`.
fn op_discard(file: &mut syn::File) -> bool {
    struct D(bool);
    impl VisitMut for D {
        fn visit_block_mut(&mut self, b: &mut syn::Block) {
            if self.0 {
                return;
            }
            for s in b.stmts.iter_mut() {
                let inner: Option<syn::Expr> = match s {
                    syn::Stmt::Expr(syn::Expr::Try(t), _) => Some((*t.expr).clone()),
                    syn::Stmt::Expr(syn::Expr::Match(m), _) if matches!(&*m.expr, syn::Expr::Call(_)) => {
                        Some((*m.expr).clone())
                    }
                    syn::Stmt::Local(l) => match l.init.as_ref().map(|i| &*i.expr) {
                        Some(syn::Expr::Try(t)) => Some((*t.expr).clone()),
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(e) = inner {
                    // Keep the binding (later statements still name it) but
                    // throw the answer away.
                    *s = match s {
                        syn::Stmt::Local(l) => {
                            let pat = &l.pat;
                            syn::parse_quote!(let #pat = { let _ = #e; Default::default() };)
                        }
                        _ => syn::parse_quote!(let _ = #e;),
                    };
                    self.0 = true;
                    return;
                }
            }
            syn::visit_mut::visit_block_mut(self, b);
        }
    }
    let mut d = D(false);
    d.visit_file_mut(file);
    d.0
}

/// A variant: the seed with its first file's body replaced (and, for the
/// structural operators, extra files).
fn variant(seed: &Seed, op: &str, root: &Path) -> Option<Seed> {
    let (target, mode, body) = seed.files.first()?.clone();
    if !target.ends_with(".rs") {
        return None;
    }
    let mut out = seed.clone();
    out.name = format!("{} [{op}]", seed.name);
    if op == "child_module" {
        if mode != "append" {
            return None;
        }
        let (dir, stem) = target.rsplit_once('/')?;
        let stem = stem.trim_end_matches(".rs");
        let child = if ["mod", "lib", "main"].contains(&stem) {
            format!("{dir}/sweep_op_moved.rs")
        } else {
            format!("{dir}/{stem}/sweep_op_moved.rs")
        };
        out.files[0] = (target.clone(), "append".into(), "pub mod sweep_op_moved;\n".into());
        out.files.insert(1, (child, "create".into(), format!("#[allow(unused_imports)]\nuse super::*;\n{body}")));
        return Some(out);
    }
    if op == "exempt_helper" {
        let (file, anchor) = match seed.audit.as_str() {
            "agent_adapters_read_bounded_headers_only" => ("crates/core/src/agents/bounded_io.rs", "pub fn read_header("),
            "agent_adapters_do_not_traverse" | "no_second_traversal_on_report_path" => {
                ("crates/core/src/locations/mod.rs", "pub fn shallow_list(")
            }
            _ => return None,
        };
        let mut parsed: syn::File = syn::parse_str(&body).ok()?;
        let stmts = fns_in(&mut parsed.items).into_iter().next()?.block.stmts.clone();
        let text = std::fs::read_to_string(root.join(file)).ok()?;
        let at = text.find(anchor)?;
        let open = at + text[at..].find('{')? + 1;
        let injected: String = stmts.iter().map(|s| format!("\n    {};", s.to_token_stream())).collect::<String>().replace(";;", ";");
        // The seed's own imports come along, or an aliased primitive would
        // arrive under a name the primitive's file never declared.
        let uses: String = parsed
            .items
            .iter()
            .filter(|i| matches!(i, syn::Item::Use(_)))
            .map(|i| format!("{}\n", i.to_token_stream()))
            .collect();
        let new = format!("{uses}{}{injected}{}", &text[..open], &text[open..]);
        out.files = vec![(file.to_string(), "replace".into(), new)];
        return Some(out);
    }
    let mut parsed: syn::File = syn::parse_str(&body).ok()?;
    let changed = match op {
        "alias" => op_alias(&mut parsed, false),
        "pub_use" => op_alias(&mut parsed, true),
        "helper" => op_helper(&mut parsed),
        "macro_wrap" => op_macro_wrap(&mut parsed),
        "via_constant" => op_via_constant(&mut parsed),
        "discard" => op_discard(&mut parsed),
        _ => false,
    };
    if !changed {
        return None;
    }
    // Inner attributes and doc comments survive the round trip.
    out.files[0].2 = quote!(#parsed).to_string();
    Some(out)
}

/// Operators that must not turn a legitimate shape into a rejection.
const ACCEPT_OPERATORS: &[&str] = &["alias", "pub_use", "helper", "via_constant"];

const REJECT_OPERATORS: &[&str] = &[
    "alias",
    "pub_use",
    "helper",
    "child_module",
    "macro_wrap",
    "via_constant",
    "exempt_helper",
];

fn apply(work: &Path, s: &Seed) -> Vec<(PathBuf, Option<String>)> {
    let mut originals = Vec::new();
    for (target, mode, body) in &s.files {
        let path = work.join(target);
        let original = std::fs::read_to_string(&path).ok();
        let mutated = match mode.as_str() {
            "replace" | "create" => body.clone(),
            _ => format!("{}\n{body}", original.clone().unwrap_or_default()),
        };
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &mutated).unwrap();
        originals.push((path, original));
    }
    originals
}

fn restore(originals: Vec<(PathBuf, Option<String>)>) {
    for (path, original) in originals.into_iter().rev() {
        match original {
            Some(t) => std::fs::write(&path, t).unwrap(),
            None => {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}

#[test]
fn every_operator_variant_of_every_seed_is_rejected() {
    let root = repo_root();
    let mut jobs: Vec<(String, Seed)> = Vec::new();
    for seed in seeds() {
        if rule(&seed.audit).is_none() {
            continue;
        }
        if seed.expect == "accept" {
            if let Some(v) = variant(&seed, "discard", &root) {
                jobs.push(("discard".into(), v));
            }
            // Precision: renaming, extracting a helper or naming a
            // literal does not make a legitimate shape illegitimate.
            for op in ACCEPT_OPERATORS {
                if let Some(mut v) = variant(&seed, op, &root) {
                    v.expect = "accept".into();
                    jobs.push((format!("{op} (accept)"), v));
                }
            }
            continue;
        }
        for op in REJECT_OPERATORS {
            if let Some(v) = variant(&seed, op, &root) {
                jobs.push((op.to_string(), v));
            }
        }
    }
    // `OPERATORS_ONLY=<substring>` narrows the run and prints the
    // variants, for debugging one seed.
    if let Ok(only) = std::env::var("OPERATORS_ONLY") {
        jobs.retain(|(op, v)| format!("{}/{} {op}", v.audit, v.name).contains(&only));
        for (_, v) in &jobs {
            for (t, m, b) in &v.files {
                eprintln!("--- {} {t} ({m})\n{b}", v.name);
            }
        }
    }
    let mut per_op: BTreeMap<String, usize> = BTreeMap::new();
    for (op, _) in &jobs {
        *per_op.entry(op.clone()).or_default() += 1;
    }
    assert!(
        jobs.len() > 300 || std::env::var("OPERATORS_ONLY").is_ok(),
        "the operators generated only {} variants",
        jobs.len()
    );
    let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).clamp(1, 12);
    let tmp = tempfile::tempdir().unwrap();
    let problems: Vec<String> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|w| {
                let mine: Vec<&(String, Seed)> = jobs.iter().enumerate().filter(|(i, _)| i % workers == w).map(|(_, j)| j).collect();
                let dir = tmp.path().join(format!("w{w}"));
                let root = root.clone();
                scope.spawn(move || {
                    let work = dir.join("workspace");
                    for e in COPIED {
                        let from = root.join(e);
                        if from.exists() {
                            copy_tree(&from, &work.join(e));
                        }
                    }
                    let mut out = Vec::new();
                    for (op, v) in mine {
                        let audit = rule(&v.audit).unwrap();
                        let originals = apply(&work, v);
                        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| audit(&work)));
                        restore(originals);
                        let want_accept = v.expect == "accept" && op.ends_with("(accept)");
                        match r {
                            Err(_) => out.push(format!("{}/{}: the audit panicked ({op})", v.audit, v.name)),
                            Ok(Ok(())) if !want_accept => out.push(format!("{}/{}: ACCEPTED the `{op}` variant", v.audit, v.name)),
                            Ok(Err(e)) if want_accept => out.push(format!("{}/{}: REJECTED the legitimate `{op}` variant: {e}", v.audit, v.name)),
                            _ => {}
                        }
                    }
                    out
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });
    println!("mutation operators: {} variants {per_op:?}", jobs.len());
    let mut problems = problems;
    problems.sort();
    assert!(
        problems.is_empty(),
        "{} of {} generated variants were not rejected ({per_op:?}):\n{}",
        problems.len(),
        jobs.len(),
        problems.join("\n")
    );
}
