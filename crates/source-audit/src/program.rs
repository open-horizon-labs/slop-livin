//! The whole-crate program model every audit rule is written against.
//!
//! `resolve.rs` closed the *reference* half of the 2026-09-22 sweep: a
//! `use X as Y` no longer hides a call. The 2026-09-22 re-review 3 sweep
//! then bypassed 43 of 45 audits anyway, and the causes were not about
//! references at all (`review/REVIEW-STACK-3.md` section 1):
//!
//! 1. **Hand-written file lists** (13 slips). An audit read two files; the
//!    mutation went in a third.
//! 2. **Hand-written name lists** (10 slips). `OpenOptions::truncate`
//!    destroys what `fs::write` destroys; `panic!` puts the same bytes on
//!    stderr as `eprintln!`; `std::io::read_to_string` reads what
//!    `fs::read_to_string` reads. Every list was missing a member with
//!    identical effect.
//! 3. **A name checked, a behaviour not** (8 slips). `canonicalize` into
//!    `let _`; a `next_delta_path` computed and dropped.
//! 4. **Only the named function read** (6 slips). A helper one call away
//!    was unaudited.
//! 5. **Syntax the layer could not see** (4 slips). `vec![probe_path(..)]`
//!    hid a blocking sink; an item-level `const` sat inside no function.
//!
//! This module answers all five in one place, so a rule can be written as
//! a question about the program rather than as a list of spellings:
//!
//! * [`Program::load`] parses **every** file under `crates/{core,cli,tui}/src`
//!   recursively. No rule names a file to scan; a rule names a *property*
//!   and asks which functions have it.
//! * [`Fun::calls`] includes calls written inside macro arguments
//!   (`vec![..]`, `matches!(..)`, `assert!`, `format!`, `panic!`,
//!   `write!`), extracted by walking the macro's token stream. A sink
//!   inside a macro is a sink.
//! * [`Program::items`] carries item-level `const`/`static`/`type` and
//!   struct fields, so a coupling or a verdict written outside any
//!   function body is still visible.
//! * [`Program::resolve_global`] follows `pub use .. as ..` re-exports,
//!   including those inside an inline `mod`, which is the second hop the
//!   per-file resolver deliberately does not take.
//! * [`Program::closure_of`] takes a predicate over calls and returns the
//!   set of function names that reach it **transitively**, so
//!   "destructive", "traverses", "reads unbounded" and "emits" are
//!   derived sets rather than lists. A method call whose receiver type is
//!   unknown is treated as *possibly matching* every function of that
//!   name: the model errs towards stricter.
//!
//! **Limits.** The model is lexical and name-keyed: two functions with the
//! same name in different files are one node in the call graph. That
//! over-approximates (an audit becomes stricter, never laxer). It cannot
//! see through trait-object dispatch, a function pointer in a struct, or
//! a `proc_macro` that generates calls; `resolve::unknown_macros` reports
//! macros whose contents this layer cannot read, and rules over a
//! sensitive region fail on them rather than passing.

use quote::ToTokens;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use syn::visit::Visit;

use crate::ast::{self, CachedAst};
use crate::resolve::{self, Honoured};

// ---------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------

/// One call, resolved, with everything a behavioural rule needs to ask
/// whether the answer mattered and what it was about.
#[derive(Debug, Clone)]
pub struct PCall {
    /// Resolved path (`std::fs::rename`), or the bare method name.
    pub path: String,
    /// Exactly as written, for the message.
    pub written: String,
    pub method: bool,
    pub args: Vec<String>,
    pub receiver: String,
    pub honoured: Honoured,
    pub stmt: usize,
    pub conditions: Vec<String>,
    /// The call was written inside a macro's arguments, where
    /// `syn::visit` does not go.
    pub via_macro: bool,
}

impl PCall {
    pub fn is(&self, suffix: &str) -> bool {
        resolve::path_ends_with(&self.path, suffix)
    }
    /// The callee's own name, whatever module it was reached through.
    pub fn callee(&self) -> &str {
        self.path.rsplit("::").next().unwrap_or(&self.path)
    }
    pub fn discarded(&self) -> bool {
        self.honoured == Honoured::Discarded
    }
}

/// One macro invocation inside a function, with its raw tokens: rules
/// that must reason about a `matches!` pattern or a `vec![..]` element
/// read these.
#[derive(Debug, Clone)]
pub struct MacroUse {
    pub name: String,
    pub tokens: String,
    pub literals: Vec<String>,
}

/// One function or impl method, outside test modules.
#[derive(Debug, Clone)]
pub struct Fun {
    pub rel: String,
    pub name: String,
    /// The `impl` type this method belongs to, when it is one.
    pub self_ty: Option<String>,
    pub sig: String,
    /// Body token text with every path resolved through the file's `use`
    /// table (see `ast::functions`).
    pub body: String,
    pub stmts: Vec<String>,
    pub is_pub: bool,
    pub calls: Vec<PCall>,
    pub macros: Vec<MacroUse>,
    pub literals: Vec<String>,
}

impl Fun {
    pub fn key(&self) -> String {
        format!("{}::{}", self.rel, self.name)
    }
    /// Every parameter's token text.
    pub fn params(&self) -> Vec<String> {
        let Some(open) = self.sig.find('(') else {
            return Vec::new();
        };
        let Some(close) = self.sig.rfind(')') else {
            return Vec::new();
        };
        if close <= open {
            return Vec::new();
        }
        split_top_level(&self.sig[open + 1..close])
    }
    pub fn return_type(&self) -> String {
        self.sig
            .rsplit_once("->")
            .map(|(_, t)| t.trim().to_string())
            .unwrap_or_default()
    }
}

/// What kind of item-level declaration this is. Every function-body rule
/// in the old layer was blind to all of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclKind {
    Const,
    Static,
    TypeAlias,
    Field,
}

#[derive(Debug, Clone)]
pub struct Decl {
    pub rel: String,
    pub kind: DeclKind,
    /// The struct a field belongs to; empty otherwise.
    pub owner: String,
    pub name: String,
    /// Declared type, resolved through the file's aliases.
    pub ty: String,
    /// Initializer token text, resolved. Empty for a field.
    pub value: String,
    pub is_pub: bool,
    pub attrs: String,
}

/// One `pub use a::b::c as d`, wherever it was written (including inside
/// an inline `mod`). The sweep's `mod shim { pub use crate::walk::X as
/// gather; }` is exactly this, and it is the hop a per-file resolver does
/// not take.
#[derive(Debug, Clone)]
pub struct ReExport {
    pub rel: String,
    /// The module path it was written in (`""` at file level).
    pub module: String,
    pub alias: String,
    pub target: String,
}

pub struct Program {
    pub root: PathBuf,
    pub files: Vec<ParsedFile>,
    pub funs: Vec<Fun>,
    pub items: Vec<Decl>,
    pub reexports: Vec<ReExport>,
    by_name: HashMap<String, Vec<usize>>,
    consts: HashMap<String, String>,
}

pub struct ParsedFile {
    pub rel: String,
    pub text: String,
    pub ast: std::rc::Rc<CachedAst>,
}

// ---------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------

impl Program {
    /// Every Rust file under `crates/{core,cli,tui}/src`, recursively,
    /// parsed once. Nothing is scoped by a hand-written list of files or
    /// directories; a rule asks which functions have a property.
    pub fn load(root: &Path) -> Program {
        let mut p = Program {
            root: root.to_path_buf(),
            files: Vec::new(),
            funs: Vec::new(),
            items: Vec::new(),
            reexports: Vec::new(),
            by_name: HashMap::new(),
            consts: HashMap::new(),
        };
        for rel in resolve::workspace_files(root) {
            let Some(parsed) = resolve::maybe(root, &rel) else {
                continue;
            };
            // Memoised on the file's exact contents, like every other
            // derivation: an audit run reads the whole workspace and
            // there are forty-five of them.
            let rel_for_facts = rel.clone();
            let facts: std::rc::Rc<FileFacts> = ast::memoised("program_file", &parsed.ast, || {
                std::rc::Rc::new(file_facts(&rel_for_facts, &parsed.ast))
            });
            p.funs.extend(facts.funs.iter().cloned());
            p.items.extend(facts.items.iter().cloned());
            p.reexports.extend(facts.reexports.iter().cloned());
            p.files.push(ParsedFile {
                rel,
                text: parsed.text,
                ast: parsed.ast,
            });
        }
        for (i, f) in p.funs.iter().enumerate() {
            p.by_name.entry(f.name.clone()).or_default().push(i);
        }
        for d in &p.items {
            if matches!(d.kind, DeclKind::Const | DeclKind::Static) {
                p.consts.insert(d.name.clone(), d.value.clone());
            }
        }
        p
    }
}

/// One file's contribution to the model, memoised on its contents.
struct FileFacts {
    funs: Vec<Fun>,
    items: Vec<Decl>,
    reexports: Vec<ReExport>,
}

fn file_facts(rel: &str, ast: &CachedAst) -> FileFacts {
    {
        // Functions, with resolved bodies (`ast::functions` memoises).
        let resolved = ast::functions(ast);
        let mut calls_by_fn: HashMap<String, Vec<PCall>> = HashMap::new();
        for c in resolve::production_calls(ast) {
            calls_by_fn.entry(c.func.clone()).or_default().push(PCall {
                path: c.path,
                written: c.written,
                method: c.method,
                args: c.args,
                receiver: c.receiver,
                honoured: c.honoured,
                stmt: c.stmt,
                conditions: c.conditions,
                via_macro: false,
            });
        }
        // Calls the visitor cannot reach: macro arguments. `syn::visit`
        // stops at a `syn::Macro`, so `vec![probe_path(p)]` was an empty
        // node in the call graph -- re-review 3's
        // `tui_actions_off_event_thread` mutation.
        let res = resolve::resolver(ast);
        let mut macros_by_fn: HashMap<String, Vec<MacroUse>> = HashMap::new();
        for m in resolve::macro_sites(ast) {
            if m.in_test {
                continue;
            }
            for (written, method, args, receiver) in calls_in_tokens(
                &syn::parse_str::<proc_macro2::TokenStream>(&m.tokens).unwrap_or_default(),
            ) {
                let path = if method {
                    written.clone()
                } else {
                    res.resolve(&written)
                };
                calls_by_fn.entry(m.func.clone()).or_default().push(PCall {
                    path,
                    written,
                    method,
                    args,
                    receiver,
                    // An argument is a propagated value: the macro
                    // decides what to do with it, so the call is not
                    // "discarded" in the sense the honour rules mean.
                    honoured: Honoured::Propagated,
                    stmt: 0,
                    conditions: Vec::new(),
                    via_macro: true,
                });
            }
            macros_by_fn
                .entry(m.func.clone())
                .or_default()
                .push(MacroUse {
                    name: m.name,
                    tokens: m.tokens,
                    literals: m.literals,
                });
        }
        let pubs = pub_fn_names(ast);
        let impl_owners = impl_method_owners(ast);
        let mut funs = Vec::new();
        for f in resolved {
            let calls = calls_by_fn.get(&f.name).cloned().unwrap_or_default();
            let macros = macros_by_fn.get(&f.name).cloned().unwrap_or_default();
            let literals = macros.iter().flat_map(|m| m.literals.clone()).collect();
            funs.push(Fun {
                rel: rel.to_string(),
                is_pub: pubs.contains(&f.name),
                self_ty: impl_owners.get(&f.name).cloned(),
                name: f.name,
                sig: f.sig,
                body: f.body,
                stmts: f.stmts,
                calls,
                macros,
                literals,
            });
        }
        FileFacts {
            funs,
            items: declarations(rel, ast, &res),
            reexports: reexports(rel, ast),
        }
    }
}

/// Splits a parameter/argument list on top-level commas (ignoring those
/// inside `<>`, `()`, `[]`).
fn split_top_level(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    for c in s.chars() {
        match c {
            '<' | '(' | '[' | '{' => {
                depth += 1;
                cur.push(c);
            }
            '>' | ')' | ']' | '}' => {
                depth -= 1;
                cur.push(c);
            }
            ',' if depth == 0 => {
                out.push(cur.trim().to_string());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

/// Every call written inside a token stream, found by walking tokens
/// rather than by parsing: a macro's arguments are frequently not a valid
/// expression (`matches!(x, Pat::Y(_))`), so parsing them would drop the
/// whole site.
///
/// Returns `(written path, is_method, args, receiver)`.
fn calls_in_tokens(ts: &proc_macro2::TokenStream) -> Vec<(String, bool, Vec<String>, String)> {
    let mut out = Vec::new();
    let toks: Vec<proc_macro2::TokenTree> = ts.clone().into_iter().collect();
    let mut i = 0usize;
    while i < toks.len() {
        if let proc_macro2::TokenTree::Group(g) = &toks[i] {
            // Arguments of a call: the path immediately before it.
            if g.delimiter() == proc_macro2::Delimiter::Parenthesis {
                let (path, method, receiver) = path_before(&toks[..i]);
                if !path.is_empty() {
                    let args = split_top_level(&g.stream().to_string());
                    out.push((path, method, args, receiver));
                }
            }
            out.extend(calls_in_tokens(&g.stream()));
        }
        i += 1;
    }
    out
}

/// Reads backwards from a parenthesised group for the path that is being
/// called: `a :: b :: c` (a free call) or `. c` (a method call).
fn path_before(prefix: &[proc_macro2::TokenTree]) -> (String, bool, String) {
    let mut segs: Vec<String> = Vec::new();
    let mut i = prefix.len();
    // Trailing ident.
    let Some(proc_macro2::TokenTree::Ident(last)) = prefix.last() else {
        return (String::new(), false, String::new());
    };
    segs.push(last.to_string());
    i -= 1;
    // `::` separated segments before it.
    loop {
        if i >= 2
            && matches!(&prefix[i - 1], proc_macro2::TokenTree::Punct(p) if p.as_char() == ':')
            && matches!(&prefix[i - 2], proc_macro2::TokenTree::Punct(p) if p.as_char() == ':')
        {
            if i >= 3
                && let proc_macro2::TokenTree::Ident(id) = &prefix[i - 3]
            {
                segs.push(id.to_string());
                i -= 3;
                continue;
            }
            // `::path` at the crate root.
            break;
        }
        break;
    }
    // A single ident preceded by `.` is a method call on whatever came
    // before it.
    if segs.len() == 1
        && i >= 1
        && matches!(&prefix[i - 1], proc_macro2::TokenTree::Punct(p) if p.as_char() == '.')
    {
        let receiver = prefix[..i - 1]
            .iter()
            .map(|t| t.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        return (segs[0].clone(), true, receiver);
    }
    segs.reverse();
    (segs.join("::"), false, String::new())
}

fn pub_fn_names(file: &syn::File) -> HashSet<String> {
    fn is_pub(v: &syn::Visibility) -> bool {
        matches!(v, syn::Visibility::Public(_))
    }
    struct V {
        out: HashSet<String>,
    }
    impl<'ast> Visit<'ast> for V {
        fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
            if is_pub(&f.vis) {
                self.out.insert(f.sig.ident.to_string());
            }
            syn::visit::visit_item_fn(self, f);
        }
        fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
            if is_pub(&f.vis) {
                self.out.insert(f.sig.ident.to_string());
            }
            syn::visit::visit_impl_item_fn(self, f);
        }
    }
    let mut v = V {
        out: HashSet::new(),
    };
    v.visit_file(file);
    v.out
}

/// `method name -> the type it is an inherent/trait method of`. Lets a
/// rule resolve a method call by receiver type when that type is local.
fn impl_method_owners(file: &syn::File) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for item in &file.items {
        let syn::Item::Impl(i) = item else { continue };
        let syn::Type::Path(tp) = &*i.self_ty else {
            continue;
        };
        let Some(owner) = tp.path.segments.last().map(|s| s.ident.to_string()) else {
            continue;
        };
        for it in &i.items {
            if let syn::ImplItem::Fn(f) = it {
                out.insert(f.sig.ident.to_string(), owner.clone());
            }
        }
    }
    out
}

/// Item-level `const`/`static`/`type` and every struct field, with the
/// initializer's token text resolved. An audit that only reads function
/// bodies cannot see any of these, which is how re-review 3 put a
/// detector id and a verdict string where nothing looked.
fn declarations(rel: &str, file: &syn::File, res: &resolve::Resolver) -> Vec<Decl> {
    fn is_pub(v: &syn::Visibility) -> bool {
        matches!(v, syn::Visibility::Public(_))
    }
    struct V<'a> {
        rel: &'a str,
        res: &'a resolve::Resolver,
        in_test: usize,
        out: Vec<Decl>,
    }
    impl<'ast> Visit<'ast> for V<'_> {
        fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
            let test = m.attrs.iter().any(|a| {
                a.path().is_ident("cfg") && a.to_token_stream().to_string().contains("test")
            });
            if test {
                self.in_test += 1;
            }
            syn::visit::visit_item_mod(self, m);
            if test {
                self.in_test -= 1;
            }
        }
        fn visit_item_const(&mut self, c: &'ast syn::ItemConst) {
            if self.in_test == 0 {
                self.out.push(Decl {
                    rel: self.rel.to_string(),
                    kind: DeclKind::Const,
                    owner: String::new(),
                    name: c.ident.to_string(),
                    ty: self.res.resolve(&c.ty.to_token_stream().to_string()),
                    value: c.expr.to_token_stream().to_string(),
                    is_pub: is_pub(&c.vis),
                    attrs: attrs_text(&c.attrs),
                });
            }
        }
        fn visit_item_static(&mut self, s: &'ast syn::ItemStatic) {
            if self.in_test == 0 {
                self.out.push(Decl {
                    rel: self.rel.to_string(),
                    kind: DeclKind::Static,
                    owner: String::new(),
                    name: s.ident.to_string(),
                    ty: self.res.resolve(&s.ty.to_token_stream().to_string()),
                    value: s.expr.to_token_stream().to_string(),
                    is_pub: is_pub(&s.vis),
                    attrs: attrs_text(&s.attrs),
                });
            }
        }
        fn visit_item_type(&mut self, t: &'ast syn::ItemType) {
            if self.in_test == 0 {
                self.out.push(Decl {
                    rel: self.rel.to_string(),
                    kind: DeclKind::TypeAlias,
                    owner: String::new(),
                    name: t.ident.to_string(),
                    ty: t.ty.to_token_stream().to_string(),
                    value: t.ty.to_token_stream().to_string(),
                    is_pub: is_pub(&t.vis),
                    attrs: attrs_text(&t.attrs),
                });
            }
        }
        fn visit_item_struct(&mut self, s: &'ast syn::ItemStruct) {
            if self.in_test > 0 {
                return;
            }
            for f in s.fields.iter() {
                let Some(ident) = &f.ident else { continue };
                let written = f.ty.to_token_stream().to_string();
                self.out.push(Decl {
                    rel: self.rel.to_string(),
                    kind: DeclKind::Field,
                    owner: s.ident.to_string(),
                    name: ident.to_string(),
                    ty: format!("{written} {}", self.res.resolve(&written)),
                    value: String::new(),
                    is_pub: is_pub(&f.vis),
                    attrs: attrs_text(&f.attrs),
                });
            }
        }
    }
    let mut v = V {
        rel,
        res,
        in_test: 0,
        out: Vec::new(),
    };
    v.visit_file(file);
    v.out
}

/// `pub struct Adapter;` / `pub enum Adapter` -- the type a tool module
/// registers.
fn declares_adapter_type(file: &syn::File) -> bool {
    file.items.iter().any(|i| match i {
        syn::Item::Struct(s) => s.ident == "Adapter",
        syn::Item::Enum(e) => e.ident == "Adapter",
        _ => false,
    })
}

fn attrs_text(attrs: &[syn::Attribute]) -> String {
    attrs
        .iter()
        .map(|a| a.to_token_stream().to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Every `pub use .. as ..` in the file, including inside inline modules.
fn reexports(rel: &str, file: &syn::File) -> Vec<ReExport> {
    fn walk(tree: &syn::UseTree, prefix: &str, module: &str, rel: &str, out: &mut Vec<ReExport>) {
        match tree {
            syn::UseTree::Path(p) => {
                let next = if prefix.is_empty() {
                    p.ident.to_string()
                } else {
                    format!("{prefix}::{}", p.ident)
                };
                walk(&p.tree, &next, module, rel, out);
            }
            syn::UseTree::Name(n) => out.push(ReExport {
                rel: rel.to_string(),
                module: module.to_string(),
                alias: n.ident.to_string(),
                target: if prefix.is_empty() {
                    n.ident.to_string()
                } else {
                    format!("{prefix}::{}", n.ident)
                },
            }),
            syn::UseTree::Rename(r) => out.push(ReExport {
                rel: rel.to_string(),
                module: module.to_string(),
                alias: r.rename.to_string(),
                target: if prefix.is_empty() {
                    r.ident.to_string()
                } else {
                    format!("{prefix}::{}", r.ident)
                },
            }),
            syn::UseTree::Group(g) => {
                for t in &g.items {
                    walk(t, prefix, module, rel, out);
                }
            }
            syn::UseTree::Glob(_) => {}
        }
    }
    fn items(its: &[syn::Item], module: &str, rel: &str, out: &mut Vec<ReExport>) {
        for it in its {
            match it {
                syn::Item::Use(u) => walk(&u.tree, "", module, rel, out),
                syn::Item::Mod(m) => {
                    if let Some((_, inner)) = &m.content {
                        let next = if module.is_empty() {
                            m.ident.to_string()
                        } else {
                            format!("{module}::{}", m.ident)
                        };
                        items(inner, &next, rel, out);
                    }
                }
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    items(&file.items, "", rel, &mut out);
    out
}

// ---------------------------------------------------------------------
// Queries
// ---------------------------------------------------------------------

impl Program {
    pub fn file(&self, rel: &str) -> Option<&ParsedFile> {
        self.files.iter().find(|f| f.rel == rel)
    }

    pub fn text_of(&self, rel: &str) -> &str {
        self.file(rel).map(|f| f.text.as_str()).unwrap_or("")
    }

    pub fn fns_named<'a>(&'a self, name: &str) -> Vec<&'a Fun> {
        self.by_name
            .get(name)
            .map(|ix| ix.iter().map(|i| &self.funs[*i]).collect())
            .unwrap_or_default()
    }

    pub fn defines(&self, name: &str) -> bool {
        self.by_name.contains_key(name)
    }

    /// A `const`/`static`'s initializer text, by name.
    pub fn const_value(&self, name: &str) -> Option<&str> {
        self.consts.get(name).map(String::as_str)
    }

    /// The string a `const NAME: &str = "..."` holds, unquoted.
    pub fn const_string(&self, name: &str) -> Option<String> {
        let raw = self.const_value(name)?;
        let t = raw.trim();
        if t.starts_with('"') && t.ends_with('"') && t.len() >= 2 {
            Some(t[1..t.len() - 1].to_string())
        } else {
            None
        }
    }

    /// Follows `pub use .. as ..` re-exports, including the second hop
    /// through an inline `mod`, until the path stops changing.
    ///
    /// `mod shim { pub use crate::walk::discover_and_attribute as gather; }`
    /// then `shim::gather(..)` resolves to
    /// `crate::walk::discover_and_attribute`, which is the mutation that
    /// walked past `all_report_paths_through_bus`.
    pub fn resolve_global(&self, path: &str) -> String {
        let mut cur = path.replace(' ', "");
        for _ in 0..4 {
            let mut next = cur.clone();
            for r in &self.reexports {
                let qualified = if r.module.is_empty() {
                    r.alias.clone()
                } else {
                    format!("{}::{}", r.module, r.alias)
                };
                if cur == qualified || cur.ends_with(&format!("::{qualified}")) {
                    next = r.target.clone();
                    break;
                }
            }
            if next == cur {
                break;
            }
            cur = next;
        }
        cur
    }

    /// Whether `call` reaches `target`, after re-export resolution.
    pub fn call_is(&self, c: &PCall, target: &str) -> bool {
        c.is(target) || resolve::path_ends_with(&self.resolve_global(&c.path), target)
    }

    /// Every function name that directly satisfies `pred`, plus every
    /// function that transitively calls one of them.
    ///
    /// Name-keyed on purpose: a method call whose receiver type is not a
    /// local struct is recorded as its bare name, so it *possibly*
    /// matches every function of that name. Over-approximating makes a
    /// rule stricter, and re-review 3's slip class 4 was a rule that
    /// under-approximated by never following the callee at all.
    pub fn closure_of(&self, pred: impl Fn(&Fun, &PCall) -> bool) -> HashSet<String> {
        let mut set: HashSet<String> = HashSet::new();
        for f in &self.funs {
            if f.calls.iter().any(|c| pred(f, c)) {
                set.insert(f.name.clone());
            }
        }
        loop {
            let before = set.len();
            for f in &self.funs {
                if set.contains(&f.name) {
                    continue;
                }
                let reaches = f.calls.iter().any(|c| {
                    let callee = c.callee();
                    set.contains(callee) && self.defines(callee)
                });
                if reaches {
                    set.insert(f.name.clone());
                }
            }
            if set.len() == before {
                break;
            }
        }
        set
    }

    /// Every function name reachable **from** one of `entries`, following
    /// resolved callees. "Dead" means unreachable from here.
    pub fn reachable_from(&self, entries: &[String]) -> HashSet<String> {
        let mut set: HashSet<String> = entries.iter().cloned().collect();
        loop {
            let before = set.len();
            for f in &self.funs {
                if !set.contains(&f.name) {
                    continue;
                }
                for c in &f.calls {
                    let callee = c.callee().to_string();
                    if self.defines(&callee) {
                        set.insert(callee);
                    }
                    // A re-exported path may name the function under
                    // another spelling.
                    let g = self.resolve_global(&c.path);
                    let last = g.rsplit("::").next().unwrap_or(&g).to_string();
                    if self.defines(&last) {
                        set.insert(last);
                    }
                }
            }
            if set.len() == before {
                break;
            }
        }
        set
    }

    // -----------------------------------------------------------------
    // Derived entity sets. None of these is a list of spellings.
    // -----------------------------------------------------------------

    /// Functions that destroy or replace bytes outside swamp's control,
    /// **and every function that transitively reaches one**.
    ///
    /// `OpenOptions` with `write`/`truncate`/`append` destroys what
    /// `fs::write` destroys; `Command` runs whatever it is given. The old
    /// list had `fs::*` and `Command::new` and nothing else, which is how
    /// an adapter got to truncate a tool's session index.
    pub fn destructive(&self) -> HashSet<String> {
        self.closure_of(|f, c| is_destructive_call(f, c))
    }

    /// Functions that enumerate a directory, and their callers.
    pub fn traversal(&self) -> HashSet<String> {
        self.closure_of(|_, c| is_traversal_call(c))
    }

    /// Functions that read a whole file rather than a bounded header.
    pub fn unbounded_reads(&self) -> HashSet<String> {
        self.closure_of(|_, c| is_unbounded_read(c))
    }

    /// Functions that put bytes on a terminal or a log.
    pub fn emitters(&self) -> HashSet<String> {
        self.closure_of(|f, c| is_emitter(f, c))
    }

    /// Every adapter module under `agents/`: the per-tool identification
    /// code, **including the shared family and bounded-read mechanics**.
    ///
    /// `vscode_family.rs`, `pi_family.rs` and `bounded_io.rs` used to be
    /// exempt as "neutral helpers". They are the per-tool mechanics for
    /// Cursor, Windsurf, Continue, Cline, Pi and Oh My Pi and the reader
    /// every adapter's content access goes through, so a violation
    /// written in one of them is a violation in six adapters at once --
    /// which is exactly where re-review 3 put two of its mutations.
    /// `mod.rs` is the shared model, `registry.rs` the static
    /// registration and `matrix.rs` the tool catalog; those three are not
    /// per-tool mechanics.
    pub fn adapter_files(&self) -> Vec<String> {
        self.files
            .iter()
            .map(|f| f.rel.clone())
            .filter(|rel| {
                rel.starts_with("crates/core/src/agents/") && {
                    let name = rel.rsplit('/').next().unwrap_or(rel);
                    !["mod.rs", "registry.rs", "matrix.rs"].contains(&name)
                }
            })
            .collect()
    }

    /// The modules under `agents/` that *declare a tool*: they define a
    /// `*_TOOL_ID` constant or an `Adapter` type. Derived, so a new
    /// adapter is in scope the moment it exists -- re-review 3 added
    /// `sweep_tool.rs` with both and no tests at all.
    ///
    /// This is deliberately narrower than [`Program::adapter_files`]:
    /// the shared family mechanics are held to the *behavioural*
    /// guardrails (they are six adapters' code) but they declare no tool,
    /// so the registration and per-tool test contracts are not about
    /// them.
    pub fn tool_modules(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .adapter_files()
            .into_iter()
            .filter(|rel| {
                self.items.iter().any(|d| {
                    d.rel == *rel
                        && ((matches!(d.kind, DeclKind::Const) && d.name.ends_with("_TOOL_ID"))
                            || (matches!(d.kind, DeclKind::TypeAlias) && d.name == "Adapter"))
                }) || self
                    .file(rel)
                    .map(|f| declares_adapter_type(&f.ast))
                    .unwrap_or(false)
            })
            .collect();
        out.sort();
        out.dedup();
        out
    }

    pub fn module_name(rel: &str) -> String {
        rel.rsplit('/')
            .next()
            .unwrap_or(rel)
            .trim_end_matches(".rs")
            .to_string()
    }

    pub fn funs_in<'a>(&'a self, rel: &str) -> Vec<&'a Fun> {
        self.funs.iter().filter(|f| f.rel == rel).collect()
    }

    /// Every `Fun` in an adapter module.
    pub fn adapter_funs(&self) -> Vec<&Fun> {
        let files = self.adapter_files();
        self.funs
            .iter()
            .filter(|f| files.contains(&f.rel))
            .collect()
    }

    /// Item-level declarations in an adapter module.
    pub fn adapter_items(&self) -> Vec<&Decl> {
        let files = self.adapter_files();
        self.items
            .iter()
            .filter(|d| files.contains(&d.rel))
            .collect()
    }
}

/// `std::fs` primitives that replace or remove bytes, `OpenOptions`
/// opened for writing/truncation/appending, `trash`, and any subprocess.
pub fn is_destructive_call(f: &Fun, c: &PCall) -> bool {
    const PRIMITIVES: &[&str] = &[
        "fs::rename",
        "fs::remove_file",
        "fs::remove_dir",
        "fs::remove_dir_all",
        "fs::write",
        "fs::copy",
        "fs::hard_link",
        "fs::set_permissions",
        "File::create",
        "File::create_new",
    ];
    if !c.method && PRIMITIVES.iter().any(|p| c.is(p)) {
        return true;
    }
    if c.path.starts_with("trash::") || c.is("trash::delete") || c.is("trash::delete_all") {
        return true;
    }
    if !c.method && (c.is("Command::new") || c.is("process::Command::new")) {
        return true;
    }
    // `OpenOptions::new().write(true).truncate(true).open(p)` destroys
    // exactly what `fs::write` destroys and was on no list.
    if c.method
        && c.path == "open"
        && f.body.contains("OpenOptions")
        && ["write", "truncate", "append", "create"]
            .iter()
            .any(|m| f.body.replace(' ', "").contains(&format!(".{m}(true)")))
    {
        return true;
    }
    false
}

/// Enumerating a directory, however it is spelled.
pub fn is_traversal_call(c: &PCall) -> bool {
    (!c.method && (c.is("fs::read_dir") || c.path.ends_with("read_dir")))
        || c.path.contains("walkdir")
        || c.path.contains("jwalk")
        || (c.method && c.path == "read_dir")
}

/// Reading a whole file. The free function, the method and the `std::io`
/// variants are the same read; the old list had two of the three.
pub fn is_unbounded_read(c: &PCall) -> bool {
    const NAMES: &[&str] = &["read_to_string", "read_to_end", "read_to_vec"];
    if c.is("fs::read") && !c.method {
        return true;
    }
    if NAMES.contains(&c.callee()) {
        return true;
    }
    if c.is("serde_json::from_reader") || c.is("BufReader::new") {
        return true;
    }
    false
}

/// Putting bytes where a person or a log can see them.
///
/// `panic!`/`expect`/`unwrap` write to stderr exactly as `eprintln!`
/// does. They count when the message can carry a path or a session
/// field -- a formatted message, or one naming one of those things --
/// which is the content the adapter guardrail exists to keep out of
/// logs.
pub fn is_emitter(f: &Fun, c: &PCall) -> bool {
    const HANDLES: &[&str] = &["stdout", "stderr"];
    if !c.method && (c.path.starts_with("log::") || c.path.starts_with("tracing::")) {
        return true;
    }
    if !c.method && HANDLES.iter().any(|h| c.is(&format!("io::{h}"))) {
        return true;
    }
    if c.method && c.path == "write_all" {
        return true;
    }
    if c.method && ["expect", "unwrap_or_else"].contains(&c.path.as_str()) {
        return c.args.iter().any(|a| carries_content(a));
    }
    let _ = f;
    false
}

/// A macro invocation that emits.
pub fn macro_emits(m: &MacroUse) -> bool {
    const PRINTS: &[&str] = &["println", "print", "eprintln", "eprint", "dbg"];
    const PANICS: &[&str] = &["panic", "unreachable", "todo", "unimplemented", "assert"];
    if PRINTS.contains(&m.name.as_str()) {
        return true;
    }
    if ["write", "writeln"].contains(&m.name.as_str()) {
        return true;
    }
    // A panic message is stderr output. It only carries *content* when it
    // formats something in.
    PANICS.contains(&m.name.as_str())
        && (m.tokens.contains('{') || m.literals.iter().any(|l| l.contains("{}")))
}

/// Whether a message argument can carry a path, a session id or file
/// content into the output.
fn carries_content(arg: &str) -> bool {
    let a = arg.replace(' ', "");
    a.contains("{}")
        || a.contains("display()")
        || a.contains("path")
        || a.contains("session")
        || a.contains("transcript")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(src: &str) -> proc_macro2::TokenStream {
        syn::parse_str(src).unwrap()
    }

    #[test]
    fn a_call_inside_a_macro_is_a_call() {
        let found = calls_in_tokens(&tokens("occupancy :: probe_path (p)"));
        assert!(
            found
                .iter()
                .any(|(p, m, _, _)| p == "occupancy::probe_path" && !m),
            "{found:?}"
        );
    }

    #[test]
    fn a_method_call_inside_a_macro_keeps_its_receiver() {
        let found = calls_in_tokens(&tokens("self . probe (p)"));
        let hit = found
            .iter()
            .find(|(p, _, _, _)| p == "probe")
            .expect("probe");
        assert!(hit.1, "a method");
        assert_eq!(hit.3.replace(' ', ""), "self");
    }

    #[test]
    fn a_matches_pattern_does_not_stop_the_walk() {
        // `matches!(state, Occupied(_))` is not a valid expression, so a
        // parse-based extractor would drop the whole site.
        let found = calls_in_tokens(&tokens(
            "state , crate :: occupancy :: OccupancyState :: Occupied (_)",
        ));
        assert!(!found.is_empty(), "{found:?}");
    }

    #[test]
    fn a_parameter_list_splits_on_top_level_commas_only() {
        assert_eq!(
            split_top_level("a: HashMap<K, V>, b: &[(u8, u8)]"),
            vec!["a: HashMap<K, V>".to_string(), "b: &[(u8, u8)]".to_string()]
        );
    }
}
