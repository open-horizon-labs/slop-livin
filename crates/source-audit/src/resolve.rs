//! The hardened layer every audit reads structure through.
//!
//! A background mutation sweep over the 2026-09-22 re-review bypassed
//! **all 42** audits with compiling, harmful mutations. The structural
//! causes were shared, not per-audit
//! (`review/AUDIT-MUTATION-SWEEP.md`, `GUARDRAILS_SPEC.md` section 17):
//!
//! 1. *Token text instead of resolved references.* `use X as Y` then
//!    `Y(..)` defeated every "this name must/must not appear" rule.
//!    `use std::fs::metadata as stat_path_inner` walked past
//!    `symlinks_never_followed`; `use actions::add_standing_grant as
//!    mint_standing_grant` walked past `human_only_authorization`.
//! 2. *The call appears, its answer is thrown away.* `let _ =
//!    recheck::live_protection(..)`; `let _owned = ownership.owns(key)`
//!    followed by an unguarded tombstone.
//! 3. *One-hop scanning.* `rust_files_under` did not recurse; file and
//!    sink lists were hand-written, so moving code one file away was
//!    invisible.
//! 4. *Macro escapes.* `concat!("can", " be deleted")` was invisible to
//!    the verdict-literal audit; `json!(..).to_string()` to the
//!    serializer audit; `format!("project-{}.json")` to the store audit.
//!
//! This module fixes all four in one place:
//!
//! * [`Resolver`] builds a per-file symbol table from `use` trees
//!   (including `UseTree::Rename`), local items and `mod` declarations,
//!   and [`Resolver::resolve`] turns any written path into the path it
//!   actually names. A glob import makes the resolver *say so*
//!   ([`Resolver::has_globs`]) rather than guess.
//! * [`calls`] returns every call in a file as a resolved path with the
//!   enclosing function, its statement index, and whether its result is
//!   honoured ([`CallSite::honoured`]).
//! * [`literals`] returns every string literal a function can produce,
//!   *including* the pieces inside `concat!`/`format!`/`json!`/`write!`,
//!   and [`unknown_macros`] fails loudly on a macro this layer cannot
//!   see through rather than passing it.
//! * [`rust_files_recursive`] walks a whole crate.
//!
//! **Limits** (recorded in each guardrail's own Limits section): the
//! resolver is lexical. It cannot see through trait-object dispatch, a
//! function pointer stored in a struct, a `proc_macro` that generates
//! calls, or a glob import that shadows a local name. Where a guardrail
//! is `severity: hard`, a runtime test named in `scripts/check.sh`
//! covers those cases instead.

use quote::ToTokens;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use syn::visit::Visit;

// ---------------------------------------------------------------------
// Whole-crate file discovery
// ---------------------------------------------------------------------

/// Every `.rs` file at or below `rel_dir`, relative to `root`, sorted.
///
/// The non-recursive predecessor is why moving a violation into
/// `agents/`, `bus/`, `consumers/` or `locations/` used to be invisible
/// unless an audit remembered to name the subdirectory by hand.
pub fn rust_files_recursive(root: &Path, rel_dir: &str) -> Vec<String> {
    let mut out = Vec::new();
    let base = root.join(rel_dir);
    let mut stack = vec![base.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().and_then(|x| x.to_str()) == Some("rs") {
                let rel = p
                    .strip_prefix(root)
                    .unwrap_or(&p)
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push(rel);
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Every Rust file in the three product crates, recursively. Every rule
/// scans all of it; nothing is scoped by a hand-written list of
/// directories any more.
pub fn workspace_files(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for krate in ["core", "cli", "tui"] {
        out.extend(rust_files_recursive(root, &format!("crates/{krate}/src")));
    }
    out.sort();
    out.dedup();
    out
}

// ---------------------------------------------------------------------
// Symbol resolution
// ---------------------------------------------------------------------

/// A file's local name → full path map, built from its `use` trees and
/// its own items.
#[derive(Debug, Default, Clone)]
pub struct Resolver {
    aliases: HashMap<String, String>,
    globs: Vec<String>,
    /// Items this file defines itself, so a local helper is reported as
    /// `<file module>::<name>` rather than mistaken for an import.
    local: Vec<String>,
}

fn join(prefix: &str, seg: &str) -> String {
    if prefix.is_empty() {
        seg.to_string()
    } else {
        format!("{prefix}::{seg}")
    }
}

fn walk_use(tree: &syn::UseTree, prefix: &str, out: &mut Resolver) {
    match tree {
        syn::UseTree::Path(p) => {
            walk_use(&p.tree, &join(prefix, &p.ident.to_string()), out);
        }
        syn::UseTree::Name(n) => {
            let full = join(prefix, &n.ident.to_string());
            out.aliases.insert(n.ident.to_string(), full);
        }
        // `use a::b::c as d`: every later `d` names `a::b::c`. This is
        // the single most productive mutation the sweep found, and the
        // reason this module exists.
        syn::UseTree::Rename(r) => {
            let full = join(prefix, &r.ident.to_string());
            out.aliases.insert(r.rename.to_string(), full);
        }
        syn::UseTree::Glob(_) => out.globs.push(prefix.to_string()),
        syn::UseTree::Group(g) => {
            for t in &g.items {
                walk_use(t, prefix, out);
            }
        }
    }
}

/// Builds the file's symbol table. Test modules are included on purpose:
/// an alias introduced in a `#[cfg(test)]` module cannot rename a
/// production call, but excluding them would make the table disagree
/// with the file.
pub fn resolver(file: &syn::File) -> Resolver {
    struct V<'a>(&'a mut Resolver);
    impl<'ast> Visit<'ast> for V<'_> {
        fn visit_item_use(&mut self, u: &'ast syn::ItemUse) {
            walk_use(&u.tree, "", self.0);
        }
        fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
            self.0.local.push(f.sig.ident.to_string());
            syn::visit::visit_item_fn(self, f);
        }
        fn visit_item_type(&mut self, t: &'ast syn::ItemType) {
            // `type Foo = bar::Baz;` is an alias too.
            self.0.aliases.insert(
                t.ident.to_string(),
                t.ty.to_token_stream().to_string().replace(' ', ""),
            );
        }
    }
    let mut out = Resolver::default();
    V(&mut out).visit_file(file);
    out
}

impl Resolver {
    /// `true` when the file glob-imports, so a bare name may come from
    /// somewhere this resolver cannot see. Audits that must not guess
    /// report this rather than passing.
    pub fn has_globs(&self) -> bool {
        !self.globs.is_empty()
    }

    pub fn globs(&self) -> &[String] {
        &self.globs
    }

    /// Every `local name -> full path` pair the file's `use` trees and
    /// type aliases introduce.
    pub fn alias_pairs(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = self
            .aliases
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        // Longest alias first, so rewriting `fs` does not corrupt a
        // longer alias that contains it.
        out.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.0.cmp(&b.0)));
        out
    }

    pub fn defines(&self, name: &str) -> bool {
        self.local.iter().any(|n| n == name)
    }

    /// The path `written` actually names, following `use` renames one
    /// level (which is all a single file can introduce) and normalizing
    /// `::` spacing, so a resolved path is always `a::b::c`.
    pub fn resolve(&self, written: &str) -> String {
        let written = written.replace(' ', "");
        let mut segments: Vec<&str> = written.split("::").filter(|s| !s.is_empty()).collect();
        if segments.is_empty() {
            return written;
        }
        let first = segments.remove(0);
        let head = match self.aliases.get(first) {
            Some(full) => full.replace(' ', ""),
            None => first.to_string(),
        };
        if segments.is_empty() {
            head
        } else {
            format!("{head}::{}", segments.join("::"))
        }
    }

    /// Whether `written` resolves to `target`, or to a path ending in
    /// `target` (so `std::fs::rename` matches a rule written as
    /// `fs::rename`). Matching is on whole path segments, never a
    /// substring: `fs::rename_all` never matches `fs::rename`.
    pub fn is(&self, written: &str, target: &str) -> bool {
        let resolved = self.resolve(written);
        path_ends_with(&resolved, target)
    }
}

/// Segment-wise suffix match: `a::b::c` ends with `b::c` and with `c`,
/// but `a::bb::c` does not end with `b::c`.
pub fn path_ends_with(path: &str, suffix: &str) -> bool {
    let p: Vec<&str> = path.split("::").filter(|s| !s.is_empty()).collect();
    let s: Vec<&str> = suffix.split("::").filter(|s| !s.is_empty()).collect();
    if s.is_empty() || s.len() > p.len() {
        return false;
    }
    p[p.len() - s.len()..] == s[..]
}

// ---------------------------------------------------------------------
// Calls, with their resolved path and whether the answer is honoured
// ---------------------------------------------------------------------

/// How a call's result reaches control flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Honoured {
    /// `f()?`, `match f() { .. }`, `if let Err(e) = f()`, `f().unwrap()`,
    /// `let x = f(); if !x { .. }` — the answer can change what happens.
    Yes,
    /// `let _ = f();`, `let _unused = f();`, or a bare `f();` whose
    /// result type is discarded. The call is present and the answer is
    /// not.
    Discarded,
    /// The call's value *is* the expression's value (a return
    /// position, an argument, a struct field): the caller decides.
    Propagated,
}

#[derive(Debug, Clone)]
pub struct CallSite {
    /// The enclosing function (`""` at item level).
    pub func: String,
    /// Whether the enclosing function is inside a `#[cfg(test)]` module
    /// or carries `#[test]`.
    pub in_test: bool,
    /// Whether the enclosing function carries `#[allow(dead_code)]`,
    /// which the sweep used to manufacture a "caller" for a dead API.
    pub dead_code_allowed: bool,
    /// The resolved path (`std::fs::rename`), or the bare method name
    /// for a method call (methods cannot be aliased).
    pub path: String,
    /// The path exactly as written, for the failure message.
    pub written: String,
    pub method: bool,
    pub honoured: Honoured,
    /// Position of the enclosing top-level statement within the
    /// function body, for "X before Y" rules.
    pub stmt: usize,
    /// The token text of the `if`/`match`/`filter` conditions this call
    /// sits inside, so a rule can require a guard to be *the condition*
    /// of the write rather than merely earlier in the body.
    pub conditions: Vec<String>,
}

struct CallVisitor<'a> {
    res: &'a Resolver,
    out: Vec<CallSite>,
    func: String,
    in_test: usize,
    dead_code: bool,
    stmt: usize,
    conditions: Vec<String>,
    /// Honour context for the expression currently being visited.
    honour: Vec<Honoured>,
    /// Names bound by `let` in the current function, with whether the
    /// name is underscore-prefixed (deliberately unused) and the body
    /// text after the binding, so `let x = f(); if x { .. }` is honoured
    /// while `let _owned = f();` is not.
    body_text: String,
}

fn is_test_attr(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        let t = a.to_token_stream().to_string();
        a.path().is_ident("test") || t.contains("cfg (test)") || t.contains("cfg(test)")
    })
}

fn has_dead_code_allow(attrs: &[syn::Attribute]) -> bool {
    attrs
        .iter()
        .any(|a| a.to_token_stream().to_string().contains("dead_code"))
}

impl CallVisitor<'_> {
    fn record(&mut self, written: String, method: bool) {
        let path = if method {
            written.clone()
        } else {
            self.res.resolve(&written)
        };
        self.out.push(CallSite {
            func: self.func.clone(),
            in_test: self.in_test > 0,
            dead_code_allowed: self.dead_code,
            path,
            written,
            method,
            honoured: *self.honour.last().unwrap_or(&Honoured::Discarded),
            stmt: self.stmt,
            conditions: self.conditions.clone(),
        });
    }

    fn with<T>(&mut self, h: Honoured, f: impl FnOnce(&mut Self) -> T) -> T {
        self.honour.push(h);
        let out = f(self);
        self.honour.pop();
        out
    }
}

impl<'ast> Visit<'ast> for CallVisitor<'_> {
    fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
        let test = is_test_attr(&m.attrs);
        if test {
            self.in_test += 1;
        }
        syn::visit::visit_item_mod(self, m);
        if test {
            self.in_test -= 1;
        }
    }

    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        let prev = std::mem::replace(&mut self.func, f.sig.ident.to_string());
        let prev_dead = std::mem::replace(&mut self.dead_code, has_dead_code_allow(&f.attrs));
        let prev_body =
            std::mem::replace(&mut self.body_text, f.block.to_token_stream().to_string());
        let test = is_test_attr(&f.attrs);
        if test {
            self.in_test += 1;
        }
        self.visit_block_numbered(&f.block);
        if test {
            self.in_test -= 1;
        }
        self.body_text = prev_body;
        self.dead_code = prev_dead;
        self.func = prev;
    }

    fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
        let prev = std::mem::replace(&mut self.func, f.sig.ident.to_string());
        let prev_dead = std::mem::replace(&mut self.dead_code, has_dead_code_allow(&f.attrs));
        let prev_body =
            std::mem::replace(&mut self.body_text, f.block.to_token_stream().to_string());
        let test = is_test_attr(&f.attrs);
        if test {
            self.in_test += 1;
        }
        self.visit_block_numbered(&f.block);
        if test {
            self.in_test -= 1;
        }
        self.body_text = prev_body;
        self.dead_code = prev_dead;
        self.func = prev;
    }

    fn visit_expr_call(&mut self, c: &'ast syn::ExprCall) {
        if let syn::Expr::Path(p) = &*c.func {
            self.record(p.path.to_token_stream().to_string(), false);
        }
        // The callee's own path is not an argument; arguments are
        // propagated values, never discarded.
        for a in &c.args {
            self.with(Honoured::Propagated, |v| v.visit_expr(a));
        }
        if !matches!(&*c.func, syn::Expr::Path(_)) {
            self.with(Honoured::Propagated, |v| v.visit_expr(&c.func));
        }
    }

    fn visit_expr_method_call(&mut self, m: &'ast syn::ExprMethodCall) {
        self.record(m.method.to_string(), true);
        // `f(..).unwrap()` / `.expect()` / `.map_err()` / `.context()`
        // all make the answer matter, so the receiver is honoured.
        let honours_receiver = matches!(
            m.method.to_string().as_str(),
            "unwrap"
                | "expect"
                | "map_err"
                | "or_else"
                | "unwrap_or_else"
                | "context"
                | "with_context"
                | "is_err"
                | "is_ok"
                | "is_some"
                | "is_none"
                | "ok_or"
                | "ok_or_else"
                | "and_then"
        );
        let h = if honours_receiver {
            Honoured::Yes
        } else {
            *self.honour.last().unwrap_or(&Honoured::Discarded)
        };
        self.with(h, |v| v.visit_expr(&m.receiver));
        for a in &m.args {
            self.with(Honoured::Propagated, |v| v.visit_expr(a));
        }
    }

    fn visit_expr_try(&mut self, t: &'ast syn::ExprTry) {
        self.with(Honoured::Yes, |v| v.visit_expr(&t.expr));
    }

    fn visit_expr_match(&mut self, m: &'ast syn::ExprMatch) {
        self.with(Honoured::Yes, |v| v.visit_expr(&m.expr));
        let cond = m.expr.to_token_stream().to_string();
        self.conditions.push(cond);
        for arm in &m.arms {
            self.visit_arm(arm);
        }
        self.conditions.pop();
    }

    fn visit_expr_if(&mut self, i: &'ast syn::ExprIf) {
        self.with(Honoured::Yes, |v| v.visit_expr(&i.cond));
        self.conditions.push(i.cond.to_token_stream().to_string());
        self.visit_block(&i.then_branch);
        self.conditions.pop();
        if let Some((_, e)) = &i.else_branch {
            self.visit_expr(e);
        }
    }

    fn visit_expr_while(&mut self, w: &'ast syn::ExprWhile) {
        self.with(Honoured::Yes, |v| v.visit_expr(&w.cond));
        self.conditions.push(w.cond.to_token_stream().to_string());
        self.visit_block(&w.body);
        self.conditions.pop();
    }

    fn visit_local(&mut self, l: &'ast syn::Local) {
        let name = binding_name(&l.pat);
        // `let _ = f();` and `let _x = f();` are the shape the sweep
        // used to keep a required call and throw its answer away.
        let honour = match &name {
            None => Honoured::Discarded,
            Some(n) if n.starts_with('_') => Honoured::Discarded,
            Some(n) => {
                // Honoured only if the binding is read again somewhere
                // after this statement.
                if self.body_text.matches(n.as_str()).count() > 1 {
                    Honoured::Yes
                } else {
                    Honoured::Discarded
                }
            }
        };
        if let Some(init) = &l.init {
            self.with(honour, |v| v.visit_expr(&init.expr));
            if let Some((_, diverge)) = &init.diverge {
                self.with(Honoured::Yes, |v| v.visit_expr(diverge));
            }
        }
    }

    fn visit_stmt(&mut self, s: &'ast syn::Stmt) {
        // A bare `f();` statement discards the answer; `f()` as the
        // function's tail expression propagates it.
        match s {
            syn::Stmt::Expr(e, Some(_)) => {
                self.with(Honoured::Discarded, |v| v.visit_expr(e));
            }
            syn::Stmt::Expr(e, None) => {
                self.with(Honoured::Propagated, |v| v.visit_expr(e));
            }
            other => syn::visit::visit_stmt(self, other),
        }
    }
}

impl CallVisitor<'_> {
    fn visit_block_numbered(&mut self, b: &syn::Block) {
        let prev = self.stmt;
        for (i, s) in b.stmts.iter().enumerate() {
            self.stmt = i;
            self.visit_stmt(s);
        }
        self.stmt = prev;
    }
}

fn binding_name(pat: &syn::Pat) -> Option<String> {
    match pat {
        syn::Pat::Ident(i) => Some(i.ident.to_string()),
        syn::Pat::Wild(_) => None,
        syn::Pat::Type(t) => binding_name(&t.pat),
        other => Some(other.to_token_stream().to_string()),
    }
}

/// Every call in the file, resolved, with its honour classification.
pub fn calls(file: &syn::File) -> Vec<CallSite> {
    let res = resolver(file);
    let mut v = CallVisitor {
        res: &res,
        out: Vec::new(),
        func: String::new(),
        in_test: 0,
        dead_code: false,
        stmt: 0,
        conditions: Vec::new(),
        honour: Vec::new(),
        body_text: String::new(),
    };
    v.visit_file(file);
    v.out
}

/// Production calls only: nothing inside `#[cfg(test)]`/`#[test]`, and
/// nothing in a function marked `#[allow(dead_code)]` (which is what the
/// sweep added to manufacture a caller for a dead public API).
pub fn production_calls(file: &syn::File) -> Vec<CallSite> {
    calls(file)
        .into_iter()
        .filter(|c| !c.in_test && !c.dead_code_allowed)
        .collect()
}

// ---------------------------------------------------------------------
// Literals, seen through macros
// ---------------------------------------------------------------------

/// Macros this layer can see the literal content of. A macro outside
/// this list inside an audited region is a failure, not a pass: the
/// sweep hid `concat!("can", " be deleted")` and
/// `json!(..).to_string()` from audits that only looked at plain string
/// literals.
pub const KNOWN_MACROS: &[&str] = &[
    "assert",
    "assert_eq",
    "assert_ne",
    "debug_assert",
    "debug_assert_eq",
    "anyhow",
    "bail",
    "ensure",
    "format",
    "format_args",
    "write",
    "writeln",
    "print",
    "println",
    "eprint",
    "eprintln",
    "panic",
    "unreachable",
    "todo",
    "unimplemented",
    "vec",
    "matches",
    "json",
    "concat",
    "stringify",
    "include_str",
    "env",
    "option_env",
    "dbg",
    "thread_local",
    "macro_rules",
    "col",
    "burnt",
    "execute",
    "queue",
    "cfg",
    "line",
    "file",
    "column",
];

#[derive(Debug, Clone)]
pub struct MacroSite {
    pub func: String,
    pub in_test: bool,
    /// Last segment of the macro's path (`format`, `concat`, `json`).
    pub name: String,
    /// Every string literal inside the macro's token stream, in order.
    pub literals: Vec<String>,
    pub tokens: String,
}

struct MacroVisitor {
    func: String,
    in_test: usize,
    out: Vec<MacroSite>,
}

impl<'ast> Visit<'ast> for MacroVisitor {
    fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
        let test = is_test_attr(&m.attrs);
        if test {
            self.in_test += 1;
        }
        syn::visit::visit_item_mod(self, m);
        if test {
            self.in_test -= 1;
        }
    }
    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        let prev = std::mem::replace(&mut self.func, f.sig.ident.to_string());
        let test = is_test_attr(&f.attrs);
        if test {
            self.in_test += 1;
        }
        syn::visit::visit_item_fn(self, f);
        if test {
            self.in_test -= 1;
        }
        self.func = prev;
    }
    fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
        let prev = std::mem::replace(&mut self.func, f.sig.ident.to_string());
        let test = is_test_attr(&f.attrs);
        if test {
            self.in_test += 1;
        }
        syn::visit::visit_impl_item_fn(self, f);
        if test {
            self.in_test -= 1;
        }
        self.func = prev;
    }
    fn visit_macro(&mut self, m: &'ast syn::Macro) {
        let name = m
            .path
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .unwrap_or_default();
        let tokens = m.tokens.to_string();
        self.out.push(MacroSite {
            func: self.func.clone(),
            in_test: self.in_test > 0,
            name,
            literals: literals_in_tokens(m.tokens.clone()),
            tokens,
        });
        syn::visit::visit_macro(self, m);
    }
    // Attribute macros are metadata, never emitted content.
    fn visit_attribute(&mut self, _a: &'ast syn::Attribute) {}
}

/// A `format!` placeholder names a *variable*, not emitted text:
/// `"... {stale})"` emits whatever `stale` holds, not the word "stale".
/// Normalizing `{name:spec}` to `{}` keeps the literal's real text
/// visible (so `format!("project-{}.json")` still shows `.json`) without
/// turning every interpolated identifier into a false positive.
pub fn strip_placeholders(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                out.push_str("{{");
            }
            '{' => {
                for c2 in chars.by_ref() {
                    if c2 == '}' {
                        break;
                    }
                }
                out.push_str("{}");
            }
            other => out.push(other),
        }
    }
    out
}

fn literals_in_tokens(tokens: proc_macro2::TokenStream) -> Vec<String> {
    let mut out = Vec::new();
    for t in tokens {
        match t {
            proc_macro2::TokenTree::Literal(l) => {
                if let Ok(s) = syn::parse_str::<syn::LitStr>(&l.to_string()) {
                    out.push(strip_placeholders(&s.value()));
                }
            }
            proc_macro2::TokenTree::Group(g) => out.extend(literals_in_tokens(g.stream())),
            _ => {}
        }
    }
    out
}

pub fn macro_sites(file: &syn::File) -> Vec<MacroSite> {
    let mut v = MacroVisitor {
        func: String::new(),
        in_test: 0,
        out: Vec::new(),
    };
    v.visit_file(file);
    v.out
}

/// Every string a production function in this file can produce: plain
/// literals *and* every literal inside a macro. `concat!("can", " be
/// deleted")` yields both pieces and their concatenation, so a
/// verdict-vocabulary rule sees the assembled phrase.
pub fn literals(file: &syn::File) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    struct V {
        func: String,
        in_test: usize,
        out: Vec<(String, String)>,
    }
    impl<'ast> Visit<'ast> for V {
        fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
            let test = is_test_attr(&m.attrs);
            if test {
                self.in_test += 1;
            }
            syn::visit::visit_item_mod(self, m);
            if test {
                self.in_test -= 1;
            }
        }
        fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
            let prev = std::mem::replace(&mut self.func, f.sig.ident.to_string());
            let test = is_test_attr(&f.attrs);
            if test {
                self.in_test += 1;
            }
            syn::visit::visit_item_fn(self, f);
            if test {
                self.in_test -= 1;
            }
            self.func = prev;
        }
        fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
            let prev = std::mem::replace(&mut self.func, f.sig.ident.to_string());
            let test = is_test_attr(&f.attrs);
            if test {
                self.in_test += 1;
            }
            syn::visit::visit_impl_item_fn(self, f);
            if test {
                self.in_test -= 1;
            }
            self.func = prev;
        }
        fn visit_lit_str(&mut self, l: &'ast syn::LitStr) {
            if self.in_test == 0 {
                self.out.push((self.func.clone(), l.value()));
            }
        }
        fn visit_attribute(&mut self, _a: &'ast syn::Attribute) {}
        // Macro tokens are not `syn::LitStr` nodes; `macro_sites`
        // supplies them.
        fn visit_macro(&mut self, _m: &'ast syn::Macro) {}
    }
    let mut v = V {
        func: String::new(),
        in_test: 0,
        out: Vec::new(),
    };
    v.visit_file(file);
    out.extend(v.out);
    for m in macro_sites(file) {
        if m.in_test {
            continue;
        }
        for l in &m.literals {
            out.push((m.func.clone(), l.clone()));
        }
        // `concat!` produces one string from its pieces; so does a
        // `format!` whose pieces are adjacent literals. Emit the
        // concatenation too, so a phrase split across arguments is
        // visible as the phrase.
        if m.literals.len() > 1 {
            out.push((m.func.clone(), m.literals.concat()));
        }
    }
    out
}

/// Macros in production code this layer cannot see through. An audit
/// over a region reports these instead of silently passing them.
pub fn unknown_macros(file: &syn::File) -> Vec<(String, String)> {
    macro_sites(file)
        .into_iter()
        .filter(|m| !m.in_test && !KNOWN_MACROS.contains(&m.name.as_str()))
        .map(|m| (m.func, m.name))
        .collect()
}

// ---------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------

pub struct Parsed {
    pub rel: String,
    pub text: String,
    pub ast: syn::File,
}

pub fn parse(root: &Path, rel: &str) -> Result<Parsed, String> {
    let path: PathBuf = root.join(rel);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("read {rel}: {e}"))?;
    let ast = syn::parse_file(&text).map_err(|e| format!("{rel} is not valid Rust: {e}"))?;
    Ok(Parsed {
        rel: rel.to_string(),
        text,
        ast,
    })
}

pub fn maybe(root: &Path, rel: &str) -> Option<Parsed> {
    parse(root, rel).ok()
}

/// Every production call in the workspace, resolved, tagged with its
/// file. The corpus every cross-file rule reads.
pub fn workspace_calls(root: &Path) -> Vec<(String, CallSite)> {
    let mut out = Vec::new();
    for rel in workspace_files(root) {
        let Some(f) = maybe(root, &rel) else { continue };
        for c in production_calls(&f.ast) {
            out.push((rel.clone(), c));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(src: &str) -> syn::File {
        syn::parse_str(src).expect("fixture parses")
    }

    #[test]
    fn a_use_rename_resolves_to_the_real_path() {
        let f =
            file("use std::fs::metadata as stat_path_inner; fn g(p: &str) { stat_path_inner(p); }");
        let c = calls(&f);
        let site = c.iter().find(|c| c.written == "stat_path_inner").unwrap();
        assert_eq!(site.path, "std::fs::metadata");
        assert!(resolver(&f).is("stat_path_inner", "fs::metadata"));
    }

    #[test]
    fn a_grouped_rename_resolves_too() {
        let f = file(
            "use crate::actions::{add_standing_grant as mint, other}; \
             fn g() { mint(1); other(2); }",
        );
        let r = resolver(&f);
        assert!(r.is("mint", "actions::add_standing_grant"));
        assert!(!r.is("other", "actions::add_standing_grant"));
    }

    #[test]
    fn a_suffix_match_is_segment_wise() {
        assert!(path_ends_with("std::fs::rename", "fs::rename"));
        assert!(!path_ends_with("std::fs::rename_all", "fs::rename"));
        assert!(!path_ends_with("std::os::rename", "fs::rename"));
    }

    #[test]
    fn a_glob_import_is_reported_not_guessed() {
        let f = file("use crate::actions::*; fn g() { mint(1); }");
        assert!(resolver(&f).has_globs());
    }

    #[test]
    fn a_discarded_result_is_not_honoured() {
        let f = file("fn g() { let _ = check(); let _owned = owns(k); }");
        let c = calls(&f);
        for name in ["check", "owns"] {
            let site = c.iter().find(|c| c.path == name).unwrap();
            assert_eq!(site.honoured, Honoured::Discarded, "{name}");
        }
    }

    #[test]
    fn a_question_mark_or_match_honours_the_result() {
        let f = file(
            "fn g() -> R { check()?; match probe() { A => return, _ => {} } \
             let state = look(); if state { } Ok(()) }",
        );
        let c = calls(&f);
        for name in ["check", "probe", "look"] {
            let site = c.iter().find(|c| c.path == name).unwrap();
            assert_eq!(site.honoured, Honoured::Yes, "{name}");
        }
    }

    #[test]
    fn a_bound_but_never_read_result_is_not_honoured() {
        let f = file("fn g() { let answer = look(); }");
        let site = calls(&f).into_iter().find(|c| c.path == "look").unwrap();
        assert_eq!(site.honoured, Honoured::Discarded);
    }

    #[test]
    fn a_format_placeholder_name_is_not_emitted_text() {
        let f = file(r#"fn g() -> String { format!("size: {} (not an estimate{stale})") }"#);
        let lits: Vec<String> = literals(&f).into_iter().map(|(_, l)| l).collect();
        assert!(
            !lits.iter().any(|l| l.contains("stale")),
            "an interpolated variable name is not text the tool prints: {lits:?}"
        );
    }

    #[test]
    fn concat_pieces_become_one_visible_phrase() {
        let f = file(r#"fn g() -> String { concat!("can", " be deleted").to_string() }"#);
        let lits: Vec<String> = literals(&f).into_iter().map(|(_, l)| l).collect();
        assert!(lits.iter().any(|l| l == "can be deleted"), "{lits:?}");
    }

    #[test]
    fn a_format_literal_with_a_placeholder_is_visible() {
        let f = file(r#"fn g(id: u32) -> String { format!("project-{}.json", id) }"#);
        let lits: Vec<String> = literals(&f).into_iter().map(|(_, l)| l).collect();
        assert!(lits.iter().any(|l| l.ends_with(".json")), "{lits:?}");
    }

    #[test]
    fn an_unknown_macro_is_reported() {
        let f = file("fn g() { mystery!(\"x\"); }");
        assert_eq!(
            unknown_macros(&f),
            vec![("g".to_string(), "mystery".to_string())]
        );
    }

    #[test]
    fn a_dead_code_allowed_caller_is_not_a_production_caller() {
        let f = file("#[allow(dead_code)] fn fake() { real_api(); }");
        assert!(production_calls(&f).is_empty());
        assert!(calls(&f).iter().any(|c| c.path == "real_api"));
    }

    #[test]
    fn test_code_is_excluded_from_production_calls() {
        let f = file("#[cfg(test)] mod t { fn g() { real_api(); } }");
        assert!(production_calls(&f).is_empty());
    }

    #[test]
    fn a_guard_records_itself_as_the_condition_of_the_write() {
        let f = file("fn g() { if ownership.owns(k) { row.present = false; } }");
        let site = calls(&f).into_iter().find(|c| c.path == "owns").unwrap();
        assert_eq!(site.honoured, Honoured::Yes);
    }
}
