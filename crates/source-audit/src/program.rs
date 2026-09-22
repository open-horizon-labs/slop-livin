//! The whole-crate program model every audit rule is written against.
//!
//! `resolve.rs` closed the *reference* half of the 2026-09-22 sweep: a
//! `use X as Y` no longer hides a call. Re-review 3 then bypassed 43 of
//! 45 audits anyway, and the causes were not about references at all
//! (`review/REVIEW-STACK-3.md` section 1):
//!
//! 1. **Hand-written file lists** (13 slips). An audit read two files; the
//!    mutation went in a third.
//! 2. **Hand-written name lists** (10 slips). `OpenOptions::truncate`
//!    destroys what `fs::write` destroys; `panic!` puts the same bytes on
//!    stderr as `eprintln!`; `std::io::read_to_string` reads what
//!    `fs::read_to_string` reads.
//! 3. **A name checked, a behaviour not** (8 slips). `canonicalize` into
//!    `let _`; a `next_delta_path` computed and dropped.
//! 4. **Only the named function read** (6 slips). A helper one call away
//!    was unaudited.
//! 5. **Syntax the layer could not see** (4 slips). `vec![probe_path(..)]`
//!    hid a blocking sink; an item-level `const` sat inside no function.
//!
//! This module answers all five in one place, so a rule is a question
//! about the program rather than a list of spellings:
//!
//! * [`Program::load`] parses **every** file under `crates/{core,cli,tui}/src`
//!   once. No rule names a file to scan; a rule names a *property* and
//!   asks which functions have it.
//! * Every definition is its own [`Fun`] -- a `new` per `impl`, a second
//!   `stage_tracked_with_source` in an inline module -- with its module
//!   path, its `impl` type, its resolved calls (including calls written
//!   inside macro arguments), the paths it references as values, its
//!   assignments, match arms, bindings, struct literals and literals.
//! * [`Program::items`] and [`Program::types`] carry item-level
//!   `const`/`static`/`type`, struct fields and enum variants, so a
//!   coupling or a verdict written outside any function body is visible.
//! * [`Program::targets`] resolves a call to the definitions it can land
//!   on: `crate::`/`super::`/`Self::` normalised, `pub use .. as ..`
//!   re-exports followed (including inside inline modules), `Type::f`
//!   resolved to that type's method, `self.f()` to the enclosing type's.
//!   A method call on any other receiver is **possibly** every method of
//!   that name: the model errs towards stricter.
//! * [`Program::closure`] derives a set -- destructive, traverses, reads
//!   unbounded, emits -- as every function that satisfies a primitive
//!   predicate *or transitively calls one that does*. The primitives are
//!   capabilities of the standard library and the tool's dependencies
//!   (`std::fs::rename`, `OpenOptions` opened for writing, a subprocess),
//!   never a list of this workspace's own function names.
//!
//! **Limits.** Resolution is lexical: trait-object dispatch resolves to
//! every method of that name, a function pointer stored in a struct is
//! followed only where its path is written, and a `proc_macro` that
//! generates calls is invisible. [`Fun::unknown_macros`] names every
//! macro this layer cannot read so a rule over a sensitive region can
//! fail on it rather than pass.

use quote::ToTokens;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use syn::visit::Visit;

use crate::ast::{self, CachedAst};
use crate::resolve::{self, Assignment, Binding, Honoured, MatchArm};

// ---------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------

/// One call, resolved through its file's `use` table, with everything a
/// behavioural rule needs to ask whether the answer mattered.
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
    /// Written inside a closure handed to `thread::spawn`.
    pub in_spawn: bool,
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
    /// Last path segment (`println`, `info`, `matches`).
    pub name: String,
    /// The path as written, resolved (`log::info`).
    pub path: String,
    pub tokens: String,
    pub literals: Vec<String>,
}

/// A struct-literal *expression* (never a pattern).
#[derive(Debug, Clone)]
pub struct StructLit {
    /// The path, resolved through the file's `use` table.
    pub path: String,
    /// `(field, initializer token text)`.
    pub fields: Vec<(String, String)>,
    /// Ends in `..base`.
    pub rest: bool,
}

/// One definition: a free `fn`, an `impl` method or a trait's default
/// method, outside test code.
#[derive(Debug, Clone)]
pub struct Fun {
    pub rel: String,
    /// `swamp_core`, `swamp_tui`, `swamp` (the CLI binary).
    pub krate: String,
    /// Module path inside the crate, inline modules included
    /// (`agents::claude_code`, `growth::sweep_stage`); empty at the root.
    pub module: String,
    pub name: String,
    /// The `impl` (or `trait`) type this is a method of.
    pub self_ty: Option<String>,
    /// The trait an `impl Trait for X` method implements.
    pub trait_: Option<String>,
    /// Declaration token text, resolved.
    pub sig: String,
    /// `(pattern, resolved type)` per parameter; `self` is `("self", T)`.
    pub params: Vec<(String, String)>,
    /// Resolved return type text, empty for `()`.
    pub ret: String,
    /// Body token text with every path resolved through the file's `use`
    /// table, plus every macro literal (see `ast::functions`).
    pub body: String,
    /// Top-level statements, resolved.
    pub stmts: Vec<String>,
    pub is_pub: bool,
    pub attrs: String,
    /// `#[allow(dead_code)]`: the sweep's way of manufacturing a caller.
    pub dead_code_allowed: bool,
    pub calls: Vec<PCall>,
    pub macros: Vec<MacroUse>,
    /// Paths referenced as *values* (`let run = actions::execute_plan;`),
    /// resolved. A function named as a value is possibly called.
    pub refs: Vec<String>,
    /// Every string literal the body contains, macro contents included.
    pub literals: Vec<String>,
    /// Every `.field` access.
    pub fields: Vec<String>,
    pub assigns: Vec<Assignment>,
    pub arms: Vec<MatchArm>,
    pub bindings: Vec<Binding>,
    pub struct_lits: Vec<StructLit>,
    /// Macros this layer cannot see through.
    pub unknown_macros: Vec<String>,
    /// `(local, type)` for every `let x: T = ..`, `let x = T::f(..)` and
    /// `let x = T { .. }`: enough to resolve `x.method()` by receiver
    /// type instead of by name alone.
    pub local_types: Vec<(String, String)>,
    /// The file glob-imports these (absolute) modules.
    pub globs: Vec<String>,
}

impl Fun {
    pub fn key(&self) -> String {
        match &self.self_ty {
            Some(t) => format!("{}::{t}::{}", self.rel, self.name),
            None => format!("{}::{}", self.rel, self.name),
        }
    }
    /// `swamp_core::walk::discover_and_attribute`, or
    /// `swamp_core::bus::EventBus::register` for a method.
    pub fn path(&self) -> String {
        let mut p = self.krate.clone();
        if !self.module.is_empty() {
            p.push_str("::");
            p.push_str(&self.module);
        }
        if let Some(t) = &self.self_ty {
            p.push_str("::");
            p.push_str(t);
        }
        p.push_str("::");
        p.push_str(&self.name);
        p
    }
    /// `module::name`, the spelling a message should use.
    pub fn display(&self) -> String {
        match &self.self_ty {
            Some(t) => format!("{}::{t}::{}", self.rel, self.name),
            None => format!("{}::{}", self.rel, self.name),
        }
    }
    /// Returns `bool` or `Option<..>`: the shape of an answer to a
    /// yes/no question.
    pub fn returns_verdict(&self) -> bool {
        let r = self.ret.replace(' ', "");
        r == "bool" || r.starts_with("Option<")
    }
    /// Whether the resolved body names `needle` as a whole path suffix
    /// token (`recheck :: live_protection`).
    pub fn names(&self, needle: &str) -> bool {
        let spaced = needle.replace("::", " :: ");
        contains_token(&self.body, &spaced) || contains_token(&self.sig, &spaced)
    }
    pub fn in_crate(&self, krate: &str) -> bool {
        self.krate == krate
    }
    /// The module path relative to its crate, as a `::`-separated list.
    pub fn module_segments(&self) -> Vec<&str> {
        self.module.split("::").filter(|s| !s.is_empty()).collect()
    }
}

/// Whether `needle` occurs in `hay` as a whole token sequence (not as
/// part of a longer identifier on either side).
pub fn contains_token(hay: &str, needle: &str) -> bool {
    fn ident(c: char) -> bool {
        c.is_alphanumeric() || c == '_'
    }
    let n = needle.len();
    let mut start = 0;
    while let Some(i) = hay[start..].find(needle) {
        let at = start + i;
        let before = hay[..at].chars().next_back();
        let after = hay[at + n..].chars().next();
        let first = needle.chars().next().unwrap_or(' ');
        let last = needle.chars().next_back().unwrap_or(' ');
        let ok_before = !ident(first) || before.is_none_or(|c| !ident(c));
        let ok_after = !ident(last) || after.is_none_or(|c| !ident(c));
        if ok_before && ok_after {
            return true;
        }
        start = at + needle.len().max(1);
        if start >= hay.len() {
            break;
        }
    }
    false
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
    pub krate: String,
    pub module: String,
    pub kind: DeclKind,
    /// The struct a field belongs to; empty otherwise.
    pub owner: String,
    pub name: String,
    /// Declared type: written spelling, then resolved spelling.
    pub ty: String,
    /// Initializer token text, resolved. Empty for a field.
    pub value: String,
    /// String literals inside the initializer.
    pub literals: Vec<String>,
    pub is_pub: bool,
    pub attrs: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeKind {
    Struct,
    Enum,
    Trait,
}

#[derive(Debug, Clone)]
pub struct TypeDecl {
    pub rel: String,
    pub krate: String,
    pub module: String,
    pub name: String,
    pub kind: TypeKind,
    pub is_pub: bool,
    pub attrs: String,
    pub variants: Vec<String>,
    /// `(name, resolved type, is_pub, attrs)` for a struct's named fields.
    pub fields: Vec<(String, String, bool, String)>,
}

impl TypeDecl {
    pub fn derives(&self, what: &str) -> bool {
        self.attrs.contains("derive") && contains_token(&self.attrs, what)
    }
}

#[derive(Debug, Clone)]
pub struct ImplDecl {
    pub rel: String,
    pub krate: String,
    pub module: String,
    pub self_ty: String,
    /// The trait's resolved path, if this is a trait impl.
    pub trait_: Option<String>,
}

impl ImplDecl {
    pub fn implements(&self, trait_name: &str) -> bool {
        self.trait_
            .as_deref()
            .is_some_and(|t| resolve::path_ends_with(t, trait_name))
    }
}

/// One `pub use a::b::c as d`, wherever it was written (including inside
/// an inline `mod`). The sweep's `mod shim { pub use crate::walk::X as
/// gather; }` is exactly this, and it is the hop a per-file resolver
/// does not take.
#[derive(Debug, Clone)]
pub struct ReExport {
    pub rel: String,
    /// `krate::module` it was written in.
    pub module: String,
    pub alias: String,
    /// Absolute target path.
    pub target: String,
}

pub struct ParsedFile {
    pub rel: String,
    pub text: String,
    pub ast: Rc<CachedAst>,
}

/// Which definitions a call can land on.
#[derive(Debug, Clone, Default)]
pub struct Target {
    /// Local definitions (indices into [`Program::funs`]).
    pub local: Vec<usize>,
    /// The absolute path the call names after normalisation and
    /// re-export resolution (`std::fs::rename`,
    /// `swamp_core::walk::discover_and_attribute`).
    pub abs: String,
    /// A method call whose receiver type is unknown: `local` is every
    /// method of that name.
    pub possible: bool,
}

pub struct Program {
    pub root: PathBuf,
    pub files: Vec<ParsedFile>,
    pub funs: Vec<Rc<Fun>>,
    pub items: Vec<Decl>,
    pub types: Vec<TypeDecl>,
    pub impls: Vec<ImplDecl>,
    pub reexports: Vec<ReExport>,
    free_by_path: HashMap<String, Vec<usize>>,
    methods_by_key: HashMap<String, Vec<usize>>,
    methods_by_name: HashMap<String, Vec<usize>>,
    free_by_name: HashMap<String, Vec<usize>>,
    by_name: HashMap<String, Vec<usize>>,
    reexport_index: HashMap<String, String>,
    consts: HashMap<String, Vec<usize>>,
    /// `targets[f][c]`: where `funs[f].calls[c]` can land.
    targets: Vec<Vec<Target>>,
    /// Local definitions each function references as a value.
    ref_targets: Vec<Vec<usize>>,
    /// Callees (calls and value references), deduplicated.
    edges: Vec<Vec<usize>>,
    reverse: RefCell<Option<Rc<Vec<Vec<usize>>>>>,
    derived: RefCell<HashMap<String, Rc<HashSet<usize>>>>,
}

// ---------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------

thread_local! {
    /// The last program assembled on this thread, keyed on the exact set
    /// of parsed files (their paths and the ids of their contents). The
    /// forty-five audits over one tree share one model; a different tree
    /// (a mutation-corpus fixture) gets its own.
    static LAST: RefCell<Option<(Vec<(String, u64)>, Rc<Program>)>> = const { RefCell::new(None) };
}

impl Program {
    /// Every Rust file under `crates/{core,cli,tui}/src`, recursively,
    /// parsed once. Nothing is scoped by a hand-written list of files or
    /// directories; a rule asks which functions have a property.
    pub fn load(root: &Path) -> Rc<Program> {
        let mut parsed: Vec<ParsedFile> = Vec::new();
        for rel in resolve::workspace_files(root) {
            let Some(p) = resolve::maybe(root, &rel) else {
                continue;
            };
            parsed.push(ParsedFile {
                rel,
                text: p.text,
                ast: p.ast,
            });
        }
        let key: Vec<(String, u64)> = parsed.iter().map(|f| (f.rel.clone(), f.ast.id())).collect();
        if let Some(hit) = LAST.with(|l| {
            l.borrow()
                .as_ref()
                .filter(|(k, _)| *k == key)
                .map(|(_, p)| Rc::clone(p))
        }) {
            return hit;
        }
        let program = Rc::new(Program::assemble(root, parsed));
        LAST.with(|l| *l.borrow_mut() = Some((key, Rc::clone(&program))));
        program
    }

    fn assemble(root: &Path, files: Vec<ParsedFile>) -> Program {
        let mut funs: Vec<Rc<Fun>> = Vec::new();
        let mut items = Vec::new();
        let mut types = Vec::new();
        let mut impls = Vec::new();
        let mut reexports = Vec::new();
        for f in &files {
            let rel = f.rel.clone();
            let facts: Rc<FileFacts> = ast::memoised("program_file_v2", &f.ast, || {
                Rc::new(file_facts(&rel, &f.ast))
            });
            funs.extend(facts.funs.iter().cloned());
            items.extend(facts.items.iter().cloned());
            types.extend(facts.types.iter().cloned());
            impls.extend(facts.impls.iter().cloned());
            reexports.extend(facts.reexports.iter().cloned());
        }
        let mut p = Program {
            root: root.to_path_buf(),
            files,
            funs,
            items,
            types,
            impls,
            reexports,
            free_by_path: HashMap::new(),
            methods_by_key: HashMap::new(),
            methods_by_name: HashMap::new(),
            free_by_name: HashMap::new(),
            by_name: HashMap::new(),
            reexport_index: HashMap::new(),
            consts: HashMap::new(),
            targets: Vec::new(),
            ref_targets: Vec::new(),
            edges: Vec::new(),
            reverse: RefCell::new(None),
            derived: RefCell::new(HashMap::new()),
        };
        for (i, f) in p.funs.iter().enumerate() {
            p.by_name.entry(f.name.clone()).or_default().push(i);
            match &f.self_ty {
                Some(t) => {
                    p.methods_by_key
                        .entry(format!("{t}::{}", f.name))
                        .or_default()
                        .push(i);
                    p.methods_by_name.entry(f.name.clone()).or_default().push(i);
                }
                None => {
                    p.free_by_path.entry(f.path()).or_default().push(i);
                    p.free_by_name.entry(f.name.clone()).or_default().push(i);
                }
            }
        }
        for (i, d) in p.items.iter().enumerate() {
            if matches!(d.kind, DeclKind::Const | DeclKind::Static) {
                p.consts.entry(d.name.clone()).or_default().push(i);
            }
        }
        for r in &p.reexports {
            p.reexport_index
                .insert(format!("{}::{}", r.module, r.alias), r.target.clone());
        }
        let mut targets = Vec::with_capacity(p.funs.len());
        let mut ref_targets = Vec::with_capacity(p.funs.len());
        let mut edges = Vec::with_capacity(p.funs.len());
        for f in &p.funs {
            let ts: Vec<Target> = f.calls.iter().map(|c| p.resolve_call(f, c)).collect();
            let rs: Vec<usize> = f
                .refs
                .iter()
                .flat_map(|r| p.resolve_path(f, r).local)
                .collect();
            let mut e: Vec<usize> = ts.iter().flat_map(|t| t.local.iter().copied()).collect();
            e.extend(rs.iter().copied());
            e.sort_unstable();
            e.dedup();
            targets.push(ts);
            ref_targets.push(rs);
            edges.push(e);
        }
        p.targets = targets;
        p.ref_targets = ref_targets;
        p.edges = edges;
        p
    }
}

/// `crates/core/src/agents/mod.rs` -> (`swamp_core`, `agents`).
pub fn crate_and_module(rel: &str) -> (String, String) {
    let parts: Vec<&str> = rel.split('/').collect();
    let krate = match parts.get(1).copied().unwrap_or("") {
        "core" => "swamp_core".to_string(),
        "tui" => "swamp_tui".to_string(),
        "cli" => "swamp".to_string(),
        other => other.replace('-', "_"),
    };
    let mut segs: Vec<String> = parts
        .iter()
        .skip(3)
        .map(|s| s.trim_end_matches(".rs").to_string())
        .collect();
    if let Some(last) = segs.last()
        && ["mod", "lib", "main"].contains(&last.as_str())
    {
        segs.pop();
    }
    (krate, segs.join("::"))
}

/// One file's contribution to the model, memoised on its contents.
struct FileFacts {
    funs: Vec<Rc<Fun>>,
    items: Vec<Decl>,
    types: Vec<TypeDecl>,
    impls: Vec<ImplDecl>,
    reexports: Vec<ReExport>,
}

fn is_test_attr(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        let t = a.to_token_stream().to_string().replace(' ', "");
        a.path().is_ident("test") || t.contains("cfg(test)")
    })
}

fn attrs_text(attrs: &[syn::Attribute]) -> String {
    attrs
        .iter()
        .map(|a| a.to_token_stream().to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_pub(v: &syn::Visibility) -> bool {
    !matches!(v, syn::Visibility::Inherited)
}

/// `krate::module` joined, with an empty module handled.
fn qualify(krate: &str, module: &str) -> String {
    if module.is_empty() {
        krate.to_string()
    } else {
        format!("{krate}::{module}")
    }
}

/// Normalises a path written in `krate::module` (inside `self_ty`'s
/// `impl`, if any) to an absolute one. Relative paths (`walk::x`,
/// `helper`) come back unchanged with `false`.
pub fn absolute(path: &str, krate: &str, module: &str, self_ty: Option<&str>) -> (String, bool) {
    let path = path.replace(' ', "");
    let mut segs: Vec<&str> = path.split("::").filter(|s| !s.is_empty()).collect();
    if segs.is_empty() {
        return (path, false);
    }
    let mut base: Vec<String> = module
        .split("::")
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    match segs[0] {
        "crate" => {
            segs.remove(0);
            let mut out = vec![krate.to_string()];
            out.extend(segs.iter().map(|s| s.to_string()));
            (out.join("::"), true)
        }
        "self" => {
            segs.remove(0);
            let mut out = vec![krate.to_string()];
            out.extend(base);
            out.extend(segs.iter().map(|s| s.to_string()));
            (out.join("::"), true)
        }
        "super" => {
            while segs.first() == Some(&"super") {
                segs.remove(0);
                base.pop();
            }
            let mut out = vec![krate.to_string()];
            out.extend(base);
            out.extend(segs.iter().map(|s| s.to_string()));
            (out.join("::"), true)
        }
        "Self" => match self_ty {
            Some(t) => {
                segs.remove(0);
                let mut out = vec![t.to_string()];
                out.extend(segs.iter().map(|s| s.to_string()));
                (out.join("::"), false)
            }
            None => (path, false),
        },
        "swamp_core" | "swamp_tui" | "std" | "core" | "alloc" => (path, true),
        _ => (path, false),
    }
}

/// Converts an `impl` method into the free-function shape the
/// per-function extractors take.
fn as_item_fn(
    attrs: &[syn::Attribute],
    vis: &syn::Visibility,
    sig: &syn::Signature,
    block: &syn::Block,
) -> syn::ItemFn {
    syn::ItemFn {
        attrs: attrs.to_vec(),
        vis: vis.clone(),
        sig: sig.clone(),
        block: Box::new(block.clone()),
    }
}

struct Collector<'a> {
    rel: &'a str,
    krate: String,
    modules: Vec<String>,
    impl_ctx: Vec<(Option<String>, Option<String>)>,
    res: &'a resolve::Resolver,
    funs: Vec<Rc<Fun>>,
    items: Vec<Decl>,
    types: Vec<TypeDecl>,
    impls: Vec<ImplDecl>,
    reexports: Vec<ReExport>,
}

impl Collector<'_> {
    fn module(&self) -> String {
        self.modules.join("::")
    }

    fn record_fn(
        &mut self,
        item: &syn::ItemFn,
        self_ty: Option<String>,
        trait_: Option<String>,
        is_pub_fn: bool,
    ) {
        let res = self.res;
        let module = self.module();
        let raw_body = item.block.to_token_stream().to_string();
        let macro_sites = resolve::macro_sites_in_fn(item);
        let mut extra = String::new();
        for m in &macro_sites {
            for l in &m.literals {
                extra.push_str(" \"");
                extra.push_str(l);
                extra.push_str("\" ");
            }
            if m.literals.len() > 1 {
                extra.push_str(" \"");
                extra.push_str(&m.literals.concat());
                extra.push_str("\" ");
            }
        }
        let body = format!("{}{extra}", ast::resolve_body(res, &raw_body));
        let stmts = item
            .block
            .stmts
            .iter()
            .map(|s| ast::resolve_body(res, &s.to_token_stream().to_string()))
            .collect();
        let sig = ast::resolve_body(res, &item.sig.to_token_stream().to_string());
        let params = item
            .sig
            .inputs
            .iter()
            .map(|a| match a {
                syn::FnArg::Receiver(_) => {
                    ("self".to_string(), self_ty.clone().unwrap_or_default())
                }
                syn::FnArg::Typed(t) => (
                    t.pat.to_token_stream().to_string(),
                    ast::resolve_body(res, &t.ty.to_token_stream().to_string()),
                ),
            })
            .collect();
        let ret = match &item.sig.output {
            syn::ReturnType::Default => String::new(),
            syn::ReturnType::Type(_, t) => ast::resolve_body(res, &t.to_token_stream().to_string()),
        };
        let mut calls: Vec<PCall> = resolve::calls_in_fn(item, res)
            .into_iter()
            .map(|c| PCall {
                path: c.path,
                written: c.written,
                method: c.method,
                args: c.args,
                receiver: c.receiver,
                honoured: c.honoured,
                stmt: c.stmt,
                conditions: c.conditions,
                via_macro: false,
                in_spawn: c.in_spawn,
            })
            .collect();
        let mut macros = Vec::new();
        let mut unknown_macros = Vec::new();
        let mut literals = resolve::plain_literals_in_fn(item);
        for m in macro_sites {
            let ts = syn::parse_str::<proc_macro2::TokenStream>(&m.tokens).unwrap_or_default();
            for (written, method, args, receiver) in calls_in_tokens(&ts) {
                let path = if method {
                    written.clone()
                } else {
                    res.resolve(&written)
                };
                calls.push(PCall {
                    path,
                    written,
                    method,
                    args,
                    receiver,
                    // An argument is a propagated value: the macro decides
                    // what to do with it.
                    honoured: Honoured::Propagated,
                    stmt: 0,
                    conditions: Vec::new(),
                    via_macro: true,
                    in_spawn: false,
                });
            }
            if !resolve::KNOWN_MACROS.contains(&m.name.as_str()) {
                unknown_macros.push(m.name.clone());
            }
            literals.extend(m.literals.iter().cloned());
            if m.literals.len() > 1 {
                literals.push(m.literals.concat());
            }
            macros.push(MacroUse {
                path: res.resolve(&m.path),
                name: m.name,
                tokens: m.tokens,
                literals: m.literals,
            });
        }
        let mut rv = RefVisitor {
            res,
            depth: 0,
            refs: Vec::new(),
            fields: Vec::new(),
            lits: Vec::new(),
            locals: Vec::new(),
        };
        rv.visit_item_fn(item);
        let fun = Fun {
            rel: self.rel.to_string(),
            krate: self.krate.clone(),
            module,
            name: item.sig.ident.to_string(),
            self_ty,
            trait_,
            sig,
            params,
            ret,
            body,
            stmts,
            is_pub: is_pub_fn,
            attrs: attrs_text(&item.attrs),
            dead_code_allowed: attrs_text(&item.attrs).contains("dead_code"),
            calls,
            macros,
            refs: rv.refs,
            literals,
            fields: rv.fields,
            assigns: resolve::assignments_in_fn(item),
            arms: resolve::match_arms_in_fn(item),
            bindings: resolve::bindings_in_fn(item),
            struct_lits: rv.lits,
            unknown_macros,
            local_types: rv.locals,
            globs: res
                .globs()
                .iter()
                .map(|g| absolute(g, &self.krate, &self.module(), None).0)
                .collect(),
        };
        self.funs.push(Rc::new(fun));
    }

    fn decl(
        &mut self,
        kind: DeclKind,
        owner: &str,
        name: String,
        ty: &syn::Type,
        value: Option<&syn::Expr>,
        vis: &syn::Visibility,
        attrs: &[syn::Attribute],
    ) {
        let written = ty.to_token_stream().to_string();
        let (value_text, literals) = match value {
            Some(e) => {
                let mut lv = LitVisitor { out: Vec::new() };
                lv.visit_expr(e);
                (
                    ast::resolve_body(self.res, &e.to_token_stream().to_string()),
                    lv.out,
                )
            }
            None => (String::new(), Vec::new()),
        };
        self.items.push(Decl {
            rel: self.rel.to_string(),
            krate: self.krate.clone(),
            module: self.module(),
            kind,
            owner: owner.to_string(),
            name,
            ty: format!("{written} {}", ast::resolve_body(self.res, &written)),
            value: value_text,
            literals,
            is_pub: is_pub(vis),
            attrs: attrs_text(attrs),
        });
    }

    fn use_tree(&mut self, tree: &syn::UseTree, prefix: &str) {
        let module = qualify(&self.krate, &self.module());
        let target = |p: &str, me: &Collector| absolute(p, &me.krate, &me.module(), None).0;
        match tree {
            syn::UseTree::Path(p) => {
                let next = if prefix.is_empty() {
                    p.ident.to_string()
                } else {
                    format!("{prefix}::{}", p.ident)
                };
                self.use_tree(&p.tree, &next);
            }
            syn::UseTree::Name(n) => {
                let (alias, full) = if n.ident == "self" {
                    (
                        prefix.rsplit("::").next().unwrap_or(prefix).to_string(),
                        prefix.to_string(),
                    )
                } else if prefix.is_empty() {
                    (n.ident.to_string(), n.ident.to_string())
                } else {
                    (n.ident.to_string(), format!("{prefix}::{}", n.ident))
                };
                let t = target(&full, self);
                self.reexports.push(ReExport {
                    rel: self.rel.to_string(),
                    module,
                    alias,
                    target: t,
                });
            }
            syn::UseTree::Rename(r) => {
                let full = if prefix.is_empty() {
                    r.ident.to_string()
                } else {
                    format!("{prefix}::{}", r.ident)
                };
                let t = target(&full, self);
                self.reexports.push(ReExport {
                    rel: self.rel.to_string(),
                    module,
                    alias: r.rename.to_string(),
                    target: t,
                });
            }
            syn::UseTree::Group(g) => {
                for t in &g.items {
                    self.use_tree(t, prefix);
                }
            }
            syn::UseTree::Glob(_) => {}
        }
    }
}

impl<'ast> Visit<'ast> for Collector<'_> {
    fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
        if is_test_attr(&m.attrs) {
            return;
        }
        self.modules.push(m.ident.to_string());
        syn::visit::visit_item_mod(self, m);
        self.modules.pop();
    }
    fn visit_item_impl(&mut self, i: &'ast syn::ItemImpl) {
        if is_test_attr(&i.attrs) {
            return;
        }
        let owner = match &*i.self_ty {
            syn::Type::Path(tp) => tp.path.segments.last().map(|s| s.ident.to_string()),
            _ => None,
        };
        let trait_ = i.trait_.as_ref().map(|(_, p, _)| {
            let written = p.to_token_stream().to_string();
            // Generic arguments are not part of the trait's identity.
            let written = written.split('<').next().unwrap_or(&written).to_string();
            let resolved = self.res.resolve(&written);
            absolute(&resolved, &self.krate, &self.module(), None).0
        });
        if let Some(o) = &owner {
            self.impls.push(ImplDecl {
                rel: self.rel.to_string(),
                krate: self.krate.clone(),
                module: self.module(),
                self_ty: o.clone(),
                trait_: trait_.clone(),
            });
        }
        self.impl_ctx.push((
            owner.or(Some(String::new())),
            trait_.map(|t| t.rsplit("::").next().unwrap_or(&t).to_string()),
        ));
        syn::visit::visit_item_impl(self, i);
        self.impl_ctx.pop();
    }
    fn visit_item_trait(&mut self, t: &'ast syn::ItemTrait) {
        self.types.push(TypeDecl {
            rel: self.rel.to_string(),
            krate: self.krate.clone(),
            module: self.module(),
            name: t.ident.to_string(),
            kind: TypeKind::Trait,
            is_pub: is_pub(&t.vis),
            attrs: attrs_text(&t.attrs),
            variants: Vec::new(),
            fields: Vec::new(),
        });
        for it in &t.items {
            if let syn::TraitItem::Fn(f) = it
                && let Some(block) = &f.default
                && !is_test_attr(&f.attrs)
            {
                let item = as_item_fn(&f.attrs, &syn::Visibility::Inherited, &f.sig, block);
                self.record_fn(
                    &item,
                    Some(t.ident.to_string()),
                    Some(t.ident.to_string()),
                    true,
                );
            }
        }
    }
    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        if is_test_attr(&f.attrs) {
            return;
        }
        self.record_fn(f, None, None, is_pub(&f.vis));
        // A free fn nested in a method body is still free.
        let saved = std::mem::take(&mut self.impl_ctx);
        syn::visit::visit_item_fn(self, f);
        self.impl_ctx = saved;
    }
    fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
        if is_test_attr(&f.attrs) {
            return;
        }
        let (owner, trait_) = self.impl_ctx.last().cloned().unwrap_or((None, None));
        let item = as_item_fn(&f.attrs, &f.vis, &f.sig, &f.block);
        // A trait impl's methods are as public as the trait.
        let public = is_pub(&f.vis) || trait_.is_some();
        self.record_fn(&item, owner.filter(|o| !o.is_empty()), trait_, public);
        let saved = std::mem::take(&mut self.impl_ctx);
        syn::visit::visit_impl_item_fn(self, f);
        self.impl_ctx = saved;
    }
    fn visit_item_const(&mut self, c: &'ast syn::ItemConst) {
        self.decl(
            DeclKind::Const,
            "",
            c.ident.to_string(),
            &c.ty,
            Some(&c.expr),
            &c.vis,
            &c.attrs,
        );
    }
    fn visit_item_static(&mut self, s: &'ast syn::ItemStatic) {
        self.decl(
            DeclKind::Static,
            "",
            s.ident.to_string(),
            &s.ty,
            Some(&s.expr),
            &s.vis,
            &s.attrs,
        );
    }
    fn visit_item_type(&mut self, t: &'ast syn::ItemType) {
        self.decl(
            DeclKind::TypeAlias,
            "",
            t.ident.to_string(),
            &t.ty,
            None,
            &t.vis,
            &t.attrs,
        );
    }
    fn visit_item_struct(&mut self, s: &'ast syn::ItemStruct) {
        if is_test_attr(&s.attrs) {
            return;
        }
        let mut fields = Vec::new();
        for f in s.fields.iter() {
            let Some(ident) = &f.ident else { continue };
            self.decl(
                DeclKind::Field,
                &s.ident.to_string(),
                ident.to_string(),
                &f.ty,
                None,
                &f.vis,
                &f.attrs,
            );
            fields.push((
                ident.to_string(),
                ast::resolve_body(self.res, &f.ty.to_token_stream().to_string()),
                is_pub(&f.vis),
                attrs_text(&f.attrs),
            ));
        }
        self.types.push(TypeDecl {
            rel: self.rel.to_string(),
            krate: self.krate.clone(),
            module: self.module(),
            name: s.ident.to_string(),
            kind: TypeKind::Struct,
            is_pub: is_pub(&s.vis),
            attrs: attrs_text(&s.attrs),
            variants: Vec::new(),
            fields,
        });
    }
    fn visit_item_enum(&mut self, e: &'ast syn::ItemEnum) {
        if is_test_attr(&e.attrs) {
            return;
        }
        self.types.push(TypeDecl {
            rel: self.rel.to_string(),
            krate: self.krate.clone(),
            module: self.module(),
            name: e.ident.to_string(),
            kind: TypeKind::Enum,
            is_pub: is_pub(&e.vis),
            attrs: attrs_text(&e.attrs),
            variants: e.variants.iter().map(|v| v.ident.to_string()).collect(),
            fields: Vec::new(),
        });
    }
    fn visit_item_use(&mut self, u: &'ast syn::ItemUse) {
        if is_pub(&u.vis) {
            self.use_tree(&u.tree, "");
        }
    }
}

/// Paths used as values, field accesses and struct literals in one
/// definition (not its nested `fn`s).
struct RefVisitor<'a> {
    res: &'a resolve::Resolver,
    depth: usize,
    refs: Vec<String>,
    fields: Vec<String>,
    lits: Vec<StructLit>,
    locals: Vec<(String, String)>,
}

impl<'ast> Visit<'ast> for RefVisitor<'_> {
    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        self.depth += 1;
        if self.depth == 1 {
            syn::visit::visit_item_fn(self, f);
        }
        self.depth -= 1;
    }
    fn visit_expr_call(&mut self, c: &'ast syn::ExprCall) {
        // The callee path is a call, not a value reference.
        if !matches!(&*c.func, syn::Expr::Path(_)) {
            self.visit_expr(&c.func);
        }
        for a in &c.args {
            self.visit_expr(a);
        }
    }
    fn visit_expr_path(&mut self, p: &'ast syn::ExprPath) {
        self.refs
            .push(self.res.resolve(&p.path.to_token_stream().to_string()));
    }
    fn visit_local(&mut self, l: &'ast syn::Local) {
        let (pat, annotated) = match &l.pat {
            syn::Pat::Type(t) => (&*t.pat, Some(t.ty.to_token_stream().to_string())),
            other => (other, None),
        };
        if let syn::Pat::Ident(id) = pat {
            let ty = annotated.or_else(|| {
                let init = l.init.as_ref()?;
                match &*init.expr {
                    syn::Expr::Call(c) => match &*c.func {
                        syn::Expr::Path(p) if p.path.segments.len() >= 2 => {
                            let segs: Vec<String> = p
                                .path
                                .segments
                                .iter()
                                .map(|s| s.ident.to_string())
                                .collect();
                            Some(segs[segs.len() - 2].clone())
                        }
                        _ => None,
                    },
                    syn::Expr::Struct(st) => st.path.segments.last().map(|s| s.ident.to_string()),
                    _ => None,
                }
            });
            if let Some(t) = ty {
                self.locals
                    .push((id.ident.to_string(), self.res.resolve(&t)));
            }
        }
        syn::visit::visit_local(self, l);
    }
    fn visit_expr_field(&mut self, f: &'ast syn::ExprField) {
        if let syn::Member::Named(n) = &f.member {
            self.fields.push(n.to_string());
        }
        syn::visit::visit_expr_field(self, f);
    }
    fn visit_expr_struct(&mut self, s: &'ast syn::ExprStruct) {
        self.lits.push(StructLit {
            path: self.res.resolve(&s.path.to_token_stream().to_string()),
            fields: s
                .fields
                .iter()
                .filter_map(|fv| match &fv.member {
                    syn::Member::Named(n) => {
                        Some((n.to_string(), fv.expr.to_token_stream().to_string()))
                    }
                    _ => None,
                })
                .collect(),
            rest: s.rest.is_some(),
        });
        syn::visit::visit_expr_struct(self, s);
    }
}

struct LitVisitor {
    out: Vec<String>,
}

impl<'ast> Visit<'ast> for LitVisitor {
    fn visit_lit_str(&mut self, l: &'ast syn::LitStr) {
        self.out.push(l.value());
    }
    fn visit_macro(&mut self, m: &'ast syn::Macro) {
        self.out.extend(resolve::literals_in(m.tokens.clone()));
    }
}

fn file_facts(rel: &str, file: &CachedAst) -> FileFacts {
    let res = resolve::resolver(file);
    let (krate, module) = crate_and_module(rel);
    let mut c = Collector {
        rel,
        krate,
        modules: module
            .split("::")
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
        impl_ctx: Vec::new(),
        res: &res,
        funs: Vec::new(),
        items: Vec::new(),
        types: Vec::new(),
        impls: Vec::new(),
        reexports: Vec::new(),
    };
    c.visit_file(file);
    FileFacts {
        funs: c.funs,
        items: c.items,
        types: c.types,
        impls: c.impls,
        reexports: c.reexports,
    }
}

/// Splits a parameter/argument list on top-level commas (ignoring those
/// inside `<>`, `()`, `[]`, `{}`).
pub fn split_top_level(s: &str) -> Vec<String> {
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
pub fn calls_in_tokens(ts: &proc_macro2::TokenStream) -> Vec<(String, bool, Vec<String>, String)> {
    let mut out = Vec::new();
    let toks: Vec<proc_macro2::TokenTree> = ts.clone().into_iter().collect();
    for (i, t) in toks.iter().enumerate() {
        if let proc_macro2::TokenTree::Group(g) = t {
            if g.delimiter() == proc_macro2::Delimiter::Parenthesis {
                let (path, method, receiver) = path_before(&toks[..i]);
                if !path.is_empty() {
                    let args = split_top_level(&g.stream().to_string());
                    out.push((path, method, args, receiver));
                }
            }
            out.extend(calls_in_tokens(&g.stream()));
        }
    }
    out
}

/// Reads backwards from a parenthesised group for the path that is being
/// called: `a :: b :: c` (a free call) or `. c` (a method call).
fn path_before(prefix: &[proc_macro2::TokenTree]) -> (String, bool, String) {
    let mut segs: Vec<String> = Vec::new();
    let mut i = prefix.len();
    let Some(proc_macro2::TokenTree::Ident(last)) = prefix.last() else {
        return (String::new(), false, String::new());
    };
    segs.push(last.to_string());
    i -= 1;
    while i >= 3
        && matches!(&prefix[i - 1], proc_macro2::TokenTree::Punct(p) if p.as_char() == ':')
        && matches!(&prefix[i - 2], proc_macro2::TokenTree::Punct(p) if p.as_char() == ':')
    {
        let proc_macro2::TokenTree::Ident(id) = &prefix[i - 3] else {
            break;
        };
        segs.push(id.to_string());
        i -= 3;
    }
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
    // Keywords are not callees (`if (..)`, `match (..)`).
    if segs.len() == 1
        && [
            "if", "match", "while", "return", "in", "for", "let", "move", "as", "mut", "ref",
        ]
        .contains(&segs[0].as_str())
    {
        return (String::new(), false, String::new());
    }
    segs.reverse();
    (segs.join("::"), false, String::new())
}

// ---------------------------------------------------------------------
// Resolution
// ---------------------------------------------------------------------

impl Program {
    /// Follows `pub use` re-exports on the longest matching prefix until
    /// the path stops changing.
    ///
    /// `mod shim { pub use crate::walk::discover_and_attribute as gather; }`
    /// then `shim::gather(..)` resolves to
    /// `swamp_core::walk::discover_and_attribute`, which is the mutation
    /// that walked past `all_report_paths_through_bus`.
    pub fn follow(&self, abs: &str) -> String {
        let mut cur = abs.to_string();
        for _ in 0..8 {
            let segs: Vec<&str> = cur.split("::").collect();
            let mut changed = false;
            for n in (1..=segs.len()).rev() {
                let prefix = segs[..n].join("::");
                if let Some(t) = self.reexport_index.get(&prefix) {
                    if *t == prefix {
                        break;
                    }
                    let rest = &segs[n..];
                    cur = if rest.is_empty() {
                        t.clone()
                    } else {
                        format!("{t}::{}", rest.join("::"))
                    };
                    changed = true;
                    break;
                }
            }
            if !changed {
                break;
            }
        }
        cur
    }

    /// The absolute spellings `path`, written in `f`, can name.
    fn candidates(&self, f: &Fun, path: &str) -> Vec<String> {
        let (abs, known) = absolute(path, &f.krate, &f.module, f.self_ty.as_deref());
        if known {
            return vec![abs];
        }
        let mut out = Vec::new();
        let segs = f.module_segments();
        for n in (0..=segs.len()).rev() {
            let mut p = vec![f.krate.as_str()];
            p.extend(&segs[..n]);
            out.push(format!("{}::{abs}", p.join("::")));
        }
        out.push(abs);
        out
    }

    /// Where a path, written in `f`, can land.
    pub fn resolve_path(&self, f: &Fun, path: &str) -> Target {
        let written = path.replace(' ', "");
        for cand in self.candidates(f, &written) {
            let c = self.follow(&cand);
            if let Some(ix) = self.free_by_path.get(&c) {
                return Target {
                    local: ix.clone(),
                    abs: c,
                    possible: false,
                };
            }
            let segs: Vec<&str> = c.split("::").collect();
            if segs.len() >= 2 {
                let key = format!("{}::{}", segs[segs.len() - 2], segs[segs.len() - 1]);
                if let Some(ix) = self.methods_by_key.get(&key) {
                    return Target {
                        local: ix.clone(),
                        abs: c,
                        possible: false,
                    };
                }
            }
        }
        let (abs, _) = absolute(&written, &f.krate, &f.module, f.self_ty.as_deref());
        let abs = self.follow(&abs);
        let segs: Vec<&str> = abs.split("::").collect();
        // A bare name no module path reaches: it can only come from a glob
        // import (`use super::*`). Anything else is a local binding, a
        // closure, a tuple struct or the prelude.
        if segs.len() == 1 {
            for g in &f.globs {
                let c = self.follow(&format!("{g}::{}", segs[0]));
                if let Some(ix) = self.free_by_path.get(&c) {
                    return Target {
                        local: ix.clone(),
                        abs: c,
                        possible: false,
                    };
                }
            }
        }
        Target {
            local: Vec::new(),
            abs,
            possible: false,
        }
    }

    /// The type a receiver expression has, when the definition says so:
    /// `self`, a parameter, a `self.field`, a typed or constructed local.
    fn receiver_type(&self, f: &Fun, recv: &str) -> Option<String> {
        let recv = recv
            .replace(' ', "")
            .trim_start_matches('&')
            .trim_start_matches('*')
            .trim_start_matches("mut")
            .to_string();
        let recv = recv.trim_start_matches('(').trim_end_matches(')');
        let base_of = |t: &str| -> String {
            let t = t
                .replace(' ', "")
                .replace("&mut", "")
                .replace('&', "")
                .replace("dyn", "")
                .replace("impl", "")
                .replace('\'', "");
            // `Box<T>`/`Rc<T>`/`Arc<T>` dereference to `T`.
            let mut t = t.as_str();
            for wrapper in ["Box<", "Rc<", "Arc<", "RefCell<", "Mutex<"] {
                if let Some(rest) = t.strip_prefix(wrapper) {
                    t = rest.trim_end_matches('>');
                }
            }
            let head = t.split('<').next().unwrap_or(t);
            head.rsplit("::").next().unwrap_or(head).to_string()
        };
        if recv == "self" {
            return f.self_ty.clone();
        }
        let (root, field) = match recv.split_once('.') {
            Some((r, rest)) => (r, Some(rest)),
            None => (recv, None),
        };
        let root_ty = if root == "self" {
            f.self_ty.clone()
        } else {
            f.params
                .iter()
                .find(|(p, _)| resolve::root_ident(p) == root)
                .map(|(_, t)| base_of(t))
                .or_else(|| {
                    f.local_types
                        .iter()
                        .rev()
                        .find(|(n, _)| n == root)
                        .map(|(_, t)| base_of(t))
                })
        }?;
        match field {
            None => Some(root_ty),
            Some(field) if !field.contains('.') && !field.contains('(') => {
                let decl = self
                    .types
                    .iter()
                    .find(|t| t.name == root_ty && t.kind == TypeKind::Struct)?;
                let (_, ty, _, _) = decl.fields.iter().find(|(n, _, _, _)| n == field)?;
                Some(base_of(ty))
            }
            _ => None,
        }
    }

    fn resolve_call(&self, f: &Fun, c: &PCall) -> Target {
        if !c.method {
            return self.resolve_path(f, &c.path);
        }
        let name = c.path.as_str();
        if let Some(t) = self.receiver_type(f, &c.receiver) {
            if let Some(ix) = self.methods_by_key.get(&format!("{t}::{name}")) {
                return Target {
                    local: ix.clone(),
                    abs: format!("{t}::{name}"),
                    possible: false,
                };
            }
            let local_type = self.types.iter().any(|d| d.name == t);
            if !local_type {
                // A standard-library or dependency type: its methods are
                // not this workspace's.
                return Target {
                    local: Vec::new(),
                    abs: format!("{t}::{name}"),
                    possible: false,
                };
            }
            // A local trait: every implementor's method of that name.
            let implementors: Vec<String> = self
                .impls
                .iter()
                .filter(|i| i.implements(&t))
                .map(|i| i.self_ty.clone())
                .collect();
            if !implementors.is_empty() {
                let local: Vec<usize> = implementors
                    .iter()
                    .filter_map(|ty| self.methods_by_key.get(&format!("{ty}::{name}")))
                    .flatten()
                    .copied()
                    .collect();
                return Target {
                    local,
                    abs: format!("{t}::{name}"),
                    possible: true,
                };
            }
        }
        match self.methods_by_name.get(name) {
            // Unknown receiver: every method of that name *that takes this
            // many arguments*. `.all(|x| ..)` on an iterator is not
            // `Ledger::all(&self)`.
            Some(ix) => Target {
                local: ix
                    .iter()
                    .copied()
                    .filter(|i| {
                        let g = &self.funs[*i];
                        let arity = g.params.iter().filter(|(p, _)| p != "self").count();
                        arity == c.args.len()
                    })
                    .collect(),
                abs: name.to_string(),
                possible: true,
            },
            None => Target {
                local: Vec::new(),
                abs: name.to_string(),
                possible: false,
            },
        }
    }

    /// Where `funs[f].calls[c]` can land.
    pub fn target(&self, f: usize, c: usize) -> &Target {
        &self.targets[f][c]
    }

    /// Whether a call (by index) resolves, after re-exports, to a path
    /// ending in `suffix` -- a local definition or an external one.
    pub fn call_reaches_path(&self, f: usize, c: usize, suffix: &str) -> bool {
        let call = &self.funs[f].calls[c];
        let t = &self.targets[f][c];
        if call.method {
            return false;
        }
        call.is(suffix)
            || resolve::path_ends_with(&t.abs, suffix)
            || t.local
                .iter()
                .any(|i| resolve::path_ends_with(&self.funs[*i].path(), suffix))
    }

    /// Whether `funs[f]` calls (or references) the definition `g`.
    pub fn calls_fn(&self, f: usize, g: usize) -> bool {
        self.edges[f].binary_search(&g).is_ok()
    }

    pub fn callees(&self, f: usize) -> &[usize] {
        &self.edges[f]
    }

    pub fn ref_targets(&self, f: usize) -> &[usize] {
        &self.ref_targets[f]
    }

    fn reverse_edges(&self) -> Rc<Vec<Vec<usize>>> {
        if let Some(r) = self.reverse.borrow().as_ref() {
            return Rc::clone(r);
        }
        let mut rev = vec![Vec::new(); self.funs.len()];
        for (f, es) in self.edges.iter().enumerate() {
            for g in es {
                rev[*g].push(f);
            }
        }
        let rev = Rc::new(rev);
        *self.reverse.borrow_mut() = Some(Rc::clone(&rev));
        rev
    }

    /// Every definition that calls `g` (or names it as a value).
    pub fn callers(&self, g: usize) -> Vec<usize> {
        self.reverse_edges()[g].clone()
    }
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

    /// Every definition with this name.
    pub fn named(&self, name: &str) -> Vec<usize> {
        self.by_name.get(name).cloned().unwrap_or_default()
    }

    pub fn defines(&self, name: &str) -> bool {
        self.by_name.contains_key(name)
    }

    /// Every definition whose absolute path ends in `suffix`
    /// (`recheck::reviewed_snapshot`, `EventBus::register`).
    pub fn defs(&self, suffix: &str) -> Vec<usize> {
        let last = suffix.rsplit("::").next().unwrap_or(suffix);
        self.named(last)
            .into_iter()
            .filter(|i| resolve::path_ends_with(&self.funs[*i].path(), suffix))
            .collect()
    }

    pub fn funs_in<'a>(&'a self, rel: &'a str) -> impl Iterator<Item = (usize, &'a Rc<Fun>)> + 'a {
        self.funs
            .iter()
            .enumerate()
            .filter(move |(_, f)| f.rel == rel)
    }

    /// A `const`/`static`'s initializer text, by name (the first
    /// definition; a name defined twice is reported by the rule that
    /// cares).
    pub fn const_value(&self, name: &str) -> Option<&str> {
        self.consts
            .get(name)
            .and_then(|ix| ix.first())
            .map(|i| self.items[*i].value.as_str())
    }

    /// Every constant with this name.
    pub fn consts_named(&self, name: &str) -> Vec<&Decl> {
        self.consts
            .get(name)
            .map(|ix| ix.iter().map(|i| &self.items[*i]).collect())
            .unwrap_or_default()
    }

    /// The string a `const NAME: &str = "..."` holds, unquoted.
    pub fn const_string(&self, name: &str) -> Option<String> {
        let d = self.consts_named(name).into_iter().next()?;
        let t = d.value.trim();
        if t.starts_with('"') && t.ends_with('"') {
            return d.literals.first().cloned();
        }
        None
    }

    /// What an expression's token text evaluates to when it is a literal
    /// or names a constant (followed through constants naming
    /// constants). `None` when it is anything else.
    pub fn eval_literal(&self, expr: &str) -> Option<String> {
        let mut e = expr.trim().trim_start_matches('&').trim().to_string();
        for _ in 0..6 {
            if e.starts_with('"') {
                return syn::parse_str::<syn::LitStr>(&e).ok().map(|l| l.value());
            }
            if e == "true" || e == "false" {
                return Some(e);
            }
            if e.chars()
                .all(|c| c.is_ascii_digit() || c == '_' || c == '.')
                && !e.is_empty()
            {
                return Some(e);
            }
            let name = e.rsplit("::").next().unwrap_or(&e).trim().to_string();
            let v = self.const_value(&name)?.trim().to_string();
            e = v.trim_start_matches('&').trim().to_string();
        }
        None
    }

    /// Every constant a definition names, followed through constants
    /// that name constants.
    pub fn consts_reached(&self, f: &Fun) -> Vec<&Decl> {
        let mut seen: HashSet<String> = HashSet::new();
        let mut out: Vec<&Decl> = Vec::new();
        let mut queue: VecDeque<String> = VecDeque::new();
        let text = format!("{} {}", f.body, f.sig);
        for tok in text.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
            if !tok.is_empty() && self.consts.contains_key(tok) {
                queue.push_back(tok.to_string());
            }
        }
        while let Some(n) = queue.pop_front() {
            if !seen.insert(n.clone()) {
                continue;
            }
            for d in self.consts_named(&n) {
                out.push(d);
                for tok in d.value.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
                    if !tok.is_empty() && self.consts.contains_key(tok) {
                        queue.push_back(tok.to_string());
                    }
                }
            }
        }
        out
    }

    pub fn type_named(&self, name: &str) -> Vec<&TypeDecl> {
        self.types.iter().filter(|t| t.name == name).collect()
    }

    /// Types that `impl <trait_name> for` them, anywhere in the program.
    pub fn implementors(&self, trait_name: &str) -> Vec<&ImplDecl> {
        self.impls
            .iter()
            .filter(|i| i.implements(trait_name))
            .collect()
    }

    /// Every module that defines a function, as `krate::module`.
    pub fn module_of(f: &Fun) -> String {
        qualify(&f.krate, &f.module)
    }

    /// The file module (no inline modules) a relative path belongs to.
    pub fn file_module(rel: &str) -> String {
        let (k, m) = crate_and_module(rel);
        qualify(&k, &m)
    }

    pub fn module_name(rel: &str) -> String {
        rel.rsplit('/')
            .next()
            .unwrap_or(rel)
            .trim_end_matches(".rs")
            .to_string()
    }
}

// ---------------------------------------------------------------------
// Derived sets. None of these is a list of this workspace's names.
// ---------------------------------------------------------------------

impl Program {
    /// Every definition that directly satisfies `pred` on one of its
    /// calls, plus every definition that transitively calls one of them.
    /// Definitions in `stop` never join the set (the bounded primitives a
    /// rule exempts), so a caller of one is not dragged in through it.
    ///
    /// Memoised under `key` for the life of this program.
    pub fn closure(
        &self,
        key: &str,
        stop: &HashSet<usize>,
        pred: impl Fn(&Fun, &PCall) -> bool,
    ) -> Rc<HashSet<usize>> {
        let mut k = key.to_string();
        if !stop.is_empty() {
            let mut s: Vec<&usize> = stop.iter().collect();
            s.sort();
            k.push_str(&format!("/{s:?}"));
        }
        if let Some(hit) = self.derived.borrow().get(&k) {
            return Rc::clone(hit);
        }
        let direct: HashSet<usize> = self
            .funs
            .iter()
            .enumerate()
            .filter(|(i, f)| !stop.contains(i) && f.calls.iter().any(|c| pred(f, c)))
            .map(|(i, _)| i)
            .collect();
        let set = Rc::new(self.upward(&direct, stop));
        self.derived.borrow_mut().insert(k, Rc::clone(&set));
        set
    }

    /// `seeds` plus every definition that transitively calls one.
    pub fn upward(&self, seeds: &HashSet<usize>, stop: &HashSet<usize>) -> HashSet<usize> {
        let rev = self.reverse_edges();
        let mut set = seeds.clone();
        let mut queue: VecDeque<usize> = seeds.iter().copied().collect();
        while let Some(g) = queue.pop_front() {
            for f in &rev[g] {
                if !stop.contains(f) && set.insert(*f) {
                    queue.push_back(*f);
                }
            }
        }
        set
    }

    /// Every definition reachable **from** `entries`, following calls and
    /// value references. Definitions in `stop` are reached but not
    /// entered.
    pub fn reachable(&self, entries: &[usize], stop: &HashSet<usize>) -> HashSet<usize> {
        let mut set: HashSet<usize> = entries.iter().copied().collect();
        let mut queue: VecDeque<usize> = entries.iter().copied().collect();
        while let Some(f) = queue.pop_front() {
            if stop.contains(&f) {
                continue;
            }
            for g in &self.edges[f] {
                if set.insert(*g) {
                    queue.push_back(*g);
                }
            }
        }
        set
    }

    /// A shortest call chain from `from` to a member of `direct`, through
    /// members of `within`, for a failure message.
    pub fn chain(
        &self,
        from: usize,
        within: &HashSet<usize>,
        direct: impl Fn(usize) -> bool,
    ) -> String {
        let mut prev: HashMap<usize, usize> = HashMap::new();
        let mut queue: VecDeque<usize> = VecDeque::from([from]);
        let mut seen: HashSet<usize> = HashSet::from([from]);
        while let Some(f) = queue.pop_front() {
            if direct(f) {
                let mut names = vec![self.funs[f].name.clone()];
                let mut cur = f;
                while let Some(p) = prev.get(&cur) {
                    names.push(self.funs[*p].name.clone());
                    cur = *p;
                }
                names.reverse();
                return names.join(" -> ");
            }
            for g in &self.edges[f] {
                if within.contains(g) && seen.insert(*g) {
                    prev.insert(*g, f);
                    queue.push_back(*g);
                }
            }
        }
        self.funs[from].name.clone()
    }

    /// Destroys, replaces or rewrites bytes (see [`destructive_call`]),
    /// transitively.
    pub fn destructive(&self) -> Rc<HashSet<usize>> {
        self.closure("destructive", &HashSet::new(), destructive_call)
    }

    /// Anything that changes the filesystem or runs a process:
    /// destructive, plus creating directories.
    pub fn mutating(&self) -> Rc<HashSet<usize>> {
        self.closure("mutating", &HashSet::new(), |f, c| {
            destructive_call(f, c)
                || (!c.method && (c.is("fs::create_dir") || c.is("fs::create_dir_all")))
        })
    }

    /// Enumerates a directory, transitively, except through `stop`.
    pub fn traversal(&self, stop: &HashSet<usize>) -> Rc<HashSet<usize>> {
        self.closure("traversal", stop, |_, c| traversal_call(c))
    }

    /// Reads a whole file, transitively, except through `stop`.
    pub fn unbounded_reads(&self, stop: &HashSet<usize>) -> Rc<HashSet<usize>> {
        self.closure("unbounded", stop, |_, c| unbounded_read(c))
    }

    /// Puts bytes on a terminal or a log, transitively.
    pub fn emitters(&self) -> Rc<HashSet<usize>> {
        let direct: HashSet<usize> = self
            .funs
            .iter()
            .enumerate()
            .filter(|(_, f)| {
                f.calls.iter().any(|c| emitter_call(c)) || f.macros.iter().any(macro_emits)
            })
            .map(|(i, _)| i)
            .collect();
        let key = "emitters".to_string();
        if let Some(hit) = self.derived.borrow().get(&key) {
            return Rc::clone(hit);
        }
        let set = Rc::new(self.upward(&direct, &HashSet::new()));
        self.derived.borrow_mut().insert(key, Rc::clone(&set));
        set
    }

    /// Reads the process environment, transitively.
    pub fn env_readers(&self) -> Rc<HashSet<usize>> {
        self.closure("env", &HashSet::new(), |_, c| env_call(c))
    }

    /// Spawns a subprocess, transitively.
    pub fn spawners(&self) -> Rc<HashSet<usize>> {
        self.closure("spawn", &HashSet::new(), |_, c| spawn_call(c))
    }

    /// Every module under `agents/` and `build_adapters/`: the per-tool
    /// identification code, **including the shared family and
    /// bounded-read mechanics**. `mod.rs` is the shared model,
    /// `registry.rs` the static registration and `matrix.rs` the tool
    /// catalog; those are not per-tool mechanics.
    ///
    /// `vscode_family.rs`, `pi_family.rs` and `bounded_io.rs` used to be
    /// exempt as "neutral helpers". They are the per-tool mechanics for
    /// Cursor, Windsurf, Continue, Cline, Pi and Oh My Pi and the reader
    /// every adapter's content access goes through, so a violation
    /// written in one of them is a violation in six adapters at once --
    /// which is exactly where re-review 3 put two of its mutations.
    pub fn adapter_files(&self) -> Vec<String> {
        self.files
            .iter()
            .map(|f| f.rel.clone())
            .filter(|rel| {
                (rel.starts_with("crates/core/src/agents/")
                    || rel.starts_with("crates/core/src/build_adapters/"))
                    && {
                        let name = rel.rsplit('/').next().unwrap_or(rel);
                        !["mod.rs", "registry.rs", "matrix.rs"].contains(&name)
                    }
            })
            .collect()
    }

    /// The adapter modules that *declare a tool*: a `*_TOOL_ID` constant
    /// or an `Adapter` type. Derived, so a new adapter is in scope the
    /// moment it exists -- re-review 3 added `sweep_tool.rs` with both
    /// and no tests at all. The shared family mechanics declare no tool.
    pub fn tool_modules(&self) -> Vec<String> {
        let files = self.adapter_files();
        let mut out: Vec<String> = files
            .into_iter()
            .filter(|rel| {
                self.items.iter().any(|d| {
                    d.rel == *rel && d.kind == DeclKind::Const && d.name.ends_with("_TOOL_ID")
                }) || self
                    .types
                    .iter()
                    .any(|t| t.rel == *rel && t.name == "Adapter" && t.kind != TypeKind::Trait)
            })
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// Every definition in an adapter module.
    pub fn adapter_funs(&self) -> Vec<usize> {
        let files = self.adapter_files();
        self.funs
            .iter()
            .enumerate()
            .filter(|(_, f)| files.contains(&f.rel))
            .map(|(i, _)| i)
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

// ---------------------------------------------------------------------
// Primitive capabilities. These name the standard library and the
// tool's dependencies, never this workspace's own functions.
// ---------------------------------------------------------------------

/// Replaces or removes bytes, or runs a process: `std::fs` primitives,
/// `File::create`, `OpenOptions` opened for writing, truncation or
/// appending, `trash`, a persisted temp file, and any subprocess.
///
/// `OpenOptions::new().write(true).truncate(true).open(p)` destroys
/// exactly what `fs::write` destroys and was on no list.
pub fn destructive_call(f: &Fun, c: &PCall) -> bool {
    const FS: &[&str] = &[
        "fs::rename",
        "fs::remove_file",
        "fs::remove_dir",
        "fs::remove_dir_all",
        "fs::write",
        "fs::copy",
        "fs::hard_link",
        "fs::set_permissions",
        "fs::soft_link",
        "unix::fs::symlink",
        "File::create",
        "File::create_new",
    ];
    if !c.method {
        if FS.iter().any(|p| c.is(p)) || c.path.starts_with("trash::") || spawn_call(c) {
            return true;
        }
        return false;
    }
    match c.path.as_str() {
        "open" => {
            let recv = c.receiver.replace(' ', "");
            let body = f.body.replace(' ', "");
            let writing = |t: &str| {
                ["write", "truncate", "append"]
                    .iter()
                    .any(|m| t.contains(&format!(".{m}(true)")))
            };
            (recv.contains("OpenOptions") && writing(&recv))
                || (body.contains("OpenOptions") && writing(&body))
        }
        "persist" | "persist_noclobber" => {
            f.body.contains("tempfile") || f.body.contains("NamedTempFile")
        }
        _ => false,
    }
}

/// Starts a subprocess.
pub fn spawn_call(c: &PCall) -> bool {
    !c.method && (c.is("process::Command::new") || c.is("Command::new"))
}

/// Enumerates a directory, however it is spelled.
pub fn traversal_call(c: &PCall) -> bool {
    if c.method {
        return c.path == "read_dir";
    }
    c.is("fs::read_dir")
        || c.path.starts_with("walkdir::")
        || c.path.starts_with("jwalk::")
        || c.is("WalkDir::new")
        || c.path == "read_dir"
}

/// Reads a whole file. The free function, the method and the `std::io`
/// variant are the same read; the old list had two of the three.
pub fn unbounded_read(c: &PCall) -> bool {
    if c.method {
        return ["read_to_string", "read_to_end"].contains(&c.path.as_str());
    }
    c.is("fs::read")
        || c.is("fs::read_to_string")
        || c.is("io::read_to_string")
        || c.callee() == "read_to_string"
        || c.callee() == "read_to_end"
        || c.is("serde_json::from_reader")
        || c.is("BufReader::new")
}

/// A call that puts bytes where a person or a log can see them.
///
/// `expect`/`unwrap_or_else(|e| panic!(..))` write to stderr exactly as
/// `eprintln!` does; they count when the message can carry content (a
/// formatted message, a `display()`), which is what the adapter
/// guardrail keeps out of logs.
pub fn emitter_call(c: &PCall) -> bool {
    if !c.method {
        return c.path.starts_with("log::")
            || c.path.starts_with("tracing::")
            || c.is("io::stdout")
            || c.is("io::stderr");
    }
    if c.path == "expect" {
        return c.args.iter().any(|a| carries_content(a));
    }
    false
}

/// A macro invocation that emits.
pub fn macro_emits(m: &MacroUse) -> bool {
    const PRINTS: &[&str] = &["println", "print", "eprintln", "eprint", "dbg"];
    const PANICS: &[&str] = &[
        "panic",
        "unreachable",
        "todo",
        "unimplemented",
        "assert",
        "assert_eq",
        "assert_ne",
    ];
    if PRINTS.contains(&m.name.as_str()) {
        return true;
    }
    if m.path.starts_with("log::") || m.path.starts_with("tracing::") {
        return true;
    }
    if ["write", "writeln"].contains(&m.name.as_str()) {
        let target = m.tokens.split(',').next().unwrap_or("");
        return target.contains("stdout") || target.contains("stderr");
    }
    // A panic message is stderr output. It carries *content* when it
    // formats something in.
    PANICS.contains(&m.name.as_str())
        && (m.literals.iter().any(|l| l.contains("{}")) || m.tokens.contains("display ()"))
}

/// Whether a message argument can carry a path, a session id or file
/// content into the output.
fn carries_content(arg: &str) -> bool {
    let a = arg.replace(' ', "");
    a.contains("{}") || a.contains("{:") || a.contains("display()") || a.contains("format!")
}

/// Reads the process environment or the user's home.
pub fn env_call(c: &PCall) -> bool {
    !c.method
        && (c.path.starts_with("std::env::")
            || c.path.starts_with("env::")
            || c.path.starts_with("dirs::")
            || c.path.starts_with("home::")
            || c.callee() == "home_dir")
}

/// Turns a value into JSON bytes, in any of `serde_json`'s spellings
/// (the `_pretty` variants produce the same bytes).
pub fn serializes_json(c: &PCall) -> bool {
    !c.method
        && (c.path.starts_with("serde_json::to_")
            || c.path.starts_with("serde_json::Serializer")
            || c.is("serde_json::ser::to_writer"))
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

    #[test]
    fn paths_normalise_against_the_writing_module() {
        assert_eq!(
            absolute("super::current_path", "swamp_core", "growth::sweep", None).0,
            "swamp_core::growth::current_path"
        );
        assert_eq!(
            absolute("crate::walk::x", "swamp_core", "report", None).0,
            "swamp_core::walk::x"
        );
        assert_eq!(
            crate_and_module("crates/core/src/agents/mod.rs"),
            ("swamp_core".to_string(), "agents".to_string())
        );
        assert_eq!(
            crate_and_module("crates/tui/src/app.rs"),
            ("swamp_tui".to_string(), "app".to_string())
        );
    }

    #[test]
    fn a_token_match_respects_identifier_boundaries() {
        assert!(contains_token("a :: rename (x)", "rename"));
        assert!(!contains_token("a :: rename_all (x)", "rename"));
        assert!(contains_token("fs :: read_dir (d)", "fs :: read_dir"));
    }
}
