//! Small helpers over `syn` so every audit reads structure, not text:
//! functions with their bodies, call paths, struct literals, string
//! literals. Token strings are used only *within* a function that the
//! AST already located, never over a whole file.

use quote::ToTokens;
use std::path::Path;
use syn::visit::Visit;

pub struct SourceFile {
    pub rel: String,
    pub text: String,
    pub ast: std::rc::Rc<CachedAst>,
}

/// A parsed file plus the identity the derived caches key on.
///
/// The id is assigned once per distinct `(path, contents)` and never
/// reused, which is what makes it safe to memoise an expensive
/// derivation (`functions`) against it: two `CachedAst`s with the same
/// id are the same bytes parsed once, and a different parse of the same
/// path gets a different id. Derefs to the `syn::File`, so every rule
/// that only wants the AST keeps reading it exactly as before.
pub struct CachedAst {
    id: u64,
    file: syn::File,
}

impl CachedAst {
    /// The identity of these exact bytes, parsed once. Never reused, so a
    /// derivation keyed on it can only be returned for the same contents.
    pub fn id(&self) -> u64 {
        self.id
    }
}

impl std::ops::Deref for CachedAst {
    type Target = syn::File;
    fn deref(&self) -> &syn::File {
        &self.file
    }
}

pub fn parse(root: &Path, rel: &str) -> Result<SourceFile, String> {
    let path = root.join(rel);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("read {rel}: {e}"))?;
    let ast = parse_cached(rel, &text)?;
    Ok(SourceFile {
        rel: rel.to_string(),
        text,
        ast,
    })
}

/// `syn::parse_file`, memoised on the file's **exact contents**.
///
/// Parsing and the derivations over it are what an audit run costs:
/// every rule reads the whole workspace, and the mutation corpus runs
/// one audit per fixture over a copy of that workspace, so the same
/// unchanged files were re-parsed and re-analysed 136 times over. The
/// cache is keyed on `(rel, text)` and compares the text exactly, never
/// a stamp or a digest, so a cached parse can only ever be returned for
/// input that is byte-for-byte the file being parsed. Reading the file
/// still happens every time; only the parse is skipped.
///
/// Thread-local, so no audit run can observe another thread's entries
/// and no lock is taken on the hot path.
pub fn parse_cached(rel: &str, text: &str) -> Result<std::rc::Rc<CachedAst>, String> {
    let key = (rel.to_string(), text.to_string());
    if let Some(hit) = PARSE_CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return Ok(hit);
    }
    let file = syn::parse_file(text).map_err(|e| format!("{rel} is not valid Rust: {e}"))?;
    let id = NEXT_AST_ID.with(|n| {
        let v = n.get();
        n.set(v + 1);
        v
    });
    let ast = std::rc::Rc::new(CachedAst { id, file });
    PARSE_CACHE.with(|c| c.borrow_mut().insert(key, ast.clone()));
    Ok(ast)
}

/// Empties this thread's parse and derivation caches. Exists so a test
/// can prove they change nothing: run an audit cold, clear, run it
/// again, and compare the two results. Both caches are cleared together
/// -- a derived entry outliving the parse it was derived from is the one
/// way this could answer for the wrong file.
pub fn clear_parse_cache() {
    PARSE_CACHE.with(|c| c.borrow_mut().clear());
    DERIVED_CACHE.with(|c| c.borrow_mut().clear());
}

/// `(path, contents)` → the one parse of those bytes.
type ParseCache = std::collections::HashMap<(String, String), std::rc::Rc<CachedAst>>;
/// `(derivation name, file id)` → that derivation's cached result.
type DerivedCache = std::collections::HashMap<(&'static str, u64), std::rc::Rc<dyn std::any::Any>>;

thread_local! {
    static PARSE_CACHE: std::cell::RefCell<ParseCache> =
        std::cell::RefCell::new(std::collections::HashMap::new());
    static DERIVED_CACHE: std::cell::RefCell<DerivedCache> =
        std::cell::RefCell::new(std::collections::HashMap::new());
    static NEXT_AST_ID: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Memoises one derivation over one parsed file.
///
/// `what` names the derivation and `file.id` the exact bytes it was
/// derived from, so two entries can only collide if they are the same
/// derivation over the same contents. Cleared with the parse cache, so
/// a derived entry can never outlive the parse it came from.
pub fn memoised<T: Clone + 'static>(
    what: &'static str,
    file: &CachedAst,
    compute: impl FnOnce() -> T,
) -> T {
    let key = (what, file.id);
    if let Some(hit) = DERIVED_CACHE.with(|c| c.borrow().get(&key).cloned())
        && let Ok(typed) = hit.downcast::<T>()
    {
        return (*typed).clone();
    }
    let value = compute();
    DERIVED_CACHE.with(|c| {
        c.borrow_mut().insert(
            key,
            std::rc::Rc::new(value.clone()) as std::rc::Rc<dyn std::any::Any>,
        )
    });
    value
}

/// Every `.rs` file at or below `rel_dir`, relative to `root`.
///
/// This used to list one directory level. Moving a violation into
/// `agents/`, `bus/`, `consumers/` or `locations/` was therefore
/// invisible unless an audit remembered to name the subdirectory by
/// hand -- slip class 3 in `review/AUDIT-MUTATION-SWEEP.md`.
pub fn rust_files_under(root: &Path, rel_dir: &str) -> Vec<String> {
    crate::resolve::rust_files_recursive(root, rel_dir)
}

/// Every free function and impl method in the file, with its name and
/// body token string (test modules excluded).
#[derive(Clone)]
pub struct Func {
    pub name: String,
    pub body: String,
    pub stmts: Vec<String>,
    /// The declaration's own token text (generics, parameter types,
    /// return type). A body-only rule cannot see a coupling written as
    /// `fn f(d: &dyn locations::Detector)`, which is how
    /// `agent_adapters_do_not_reach_detectors` was bypassed.
    pub sig: String,
}

/// Rewrites a token-text body so every path is the path it *resolves*
/// to, and every macro's literal content is visible.
///
/// Slip classes 1 and 4 in `review/AUDIT-MUTATION-SWEEP.md`: `use
/// std::fs::metadata as stat_path_inner` walked past
/// `symlinks_never_followed`, `use actions::add_standing_grant as mint`
/// walked past `human_only_authorization`, and `concat!("can", " be
/// deleted")` walked past the verdict-vocabulary audit. Every audit
/// reads function bodies through this function, so the whole set is
/// alias- and macro-aware rather than each audit remembering to be.
///
/// The rewrite is textual over `syn`'s spaced token rendering: an alias
/// identifier standing alone becomes its full path (`stat_path_inner` ->
/// `std :: fs :: metadata`). It can over-replace a local binding that
/// shares a name with an import, which makes an audit stricter, never
/// laxer.
pub fn resolve_body(res: &crate::resolve::Resolver, body: &str) -> String {
    let mut out = body.to_string();
    for (alias, full) in res.alias_pairs() {
        if alias == full {
            continue;
        }
        let spelled = full.replace("::", " :: ");
        out = replace_token(&out, &alias, &spelled);
    }
    out
}

/// Replaces `needle` where it stands as a whole token in `haystack`
/// (surrounded by non-identifier characters), never as part of a longer
/// identifier.
fn replace_token(haystack: &str, needle: &str, with: &str) -> String {
    fn ident_char(c: char) -> bool {
        c.is_alphanumeric() || c == '_'
    }
    let mut out = String::with_capacity(haystack.len());
    let bytes: Vec<char> = haystack.chars().collect();
    let n: Vec<char> = needle.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        let matches = i + n.len() <= bytes.len()
            && bytes[i..i + n.len()] == n[..]
            && (i == 0 || !ident_char(bytes[i - 1]))
            && (i + n.len() == bytes.len() || !ident_char(bytes[i + n.len()]));
        if matches {
            out.push_str(with);
            i += n.len();
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}

/// Every macro's literal content in this function, appended to its body
/// so a literal-matching audit sees through `concat!`/`format!`/`json!`.
///
/// `body` is *this* definition's own token text, and a macro site only
/// contributes when its tokens appear in it. Matching on the function
/// name alone silently merged two same-named definitions: a second,
/// wrong `files_schema` in a submodule inherited the real one's
/// `vec![Field::new("mod_time_min", ..)]` literal and so satisfied an
/// audit that the mutation had in fact broken (found by the mutation
/// corpus, 2026-09-22, on `dir_mtime_int32_minutes` and
/// `legacy_invariants`).
fn macro_literal_text(file: &syn::File, func: &str, body: &str) -> String {
    let mut out = String::new();
    for m in crate::resolve::macro_sites(file) {
        if m.in_test || m.func != func || !body.contains(&m.tokens) {
            continue;
        }
        for l in &m.literals {
            out.push_str(" \"");
            out.push_str(l);
            out.push_str("\" ");
        }
        if m.literals.len() > 1 {
            out.push_str(" \"");
            out.push_str(&m.literals.concat());
            out.push_str("\" ");
        }
    }
    out
}

/// Every function in the file with its **resolved** body, memoised on
/// the file's identity.
///
/// This is the single most expensive thing the audits do -- fifty-one
/// call sites, each rewriting every path in every body through the
/// resolver -- and thirty rules do it to the same unchanged files. The
/// cache is keyed on [`CachedAst`]'s id, which is assigned once per
/// distinct `(path, contents)` and never reused, and it is cleared
/// together with the parse cache.
pub fn functions(file: &CachedAst) -> Vec<Func> {
    memoised("functions", file, || functions_uncached(file))
}

fn functions_uncached(file: &syn::File) -> Vec<Func> {
    let res = crate::resolve::resolver(file);
    let raw = functions_raw(file);
    raw.into_iter()
        .map(|f| {
            let extra = macro_literal_text(file, &f.name, &f.body);
            Func {
                body: format!("{}{extra}", resolve_body(&res, &f.body)),
                stmts: f.stmts.iter().map(|s| resolve_body(&res, s)).collect(),
                sig: resolve_body(&res, &f.sig),
                name: f.name,
            }
        })
        .collect()
}

/// The unresolved bodies, for the few rules that genuinely want the
/// source's own spelling (a doc/comment check, a formatting rule).
pub fn functions_raw(file: &syn::File) -> Vec<Func> {
    struct V {
        out: Vec<Func>,
        in_tests: usize,
    }
    fn is_test_mod(m: &syn::ItemMod) -> bool {
        m.attrs
            .iter()
            .any(|a| a.path().is_ident("cfg") && a.to_token_stream().to_string().contains("test"))
    }
    impl<'ast> Visit<'ast> for V {
        fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
            if is_test_mod(m) {
                self.in_tests += 1;
                syn::visit::visit_item_mod(self, m);
                self.in_tests -= 1;
            } else {
                syn::visit::visit_item_mod(self, m);
            }
        }
        fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
            if self.in_tests == 0 {
                self.out.push(Func {
                    name: f.sig.ident.to_string(),
                    body: f.block.to_token_stream().to_string(),
                    sig: f.sig.to_token_stream().to_string(),
                    stmts: f
                        .block
                        .stmts
                        .iter()
                        .map(|s| s.to_token_stream().to_string())
                        .collect(),
                });
            }
            syn::visit::visit_item_fn(self, f);
        }
        fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
            if self.in_tests == 0 {
                self.out.push(Func {
                    name: f.sig.ident.to_string(),
                    body: f.block.to_token_stream().to_string(),
                    sig: f.sig.to_token_stream().to_string(),
                    stmts: f
                        .block
                        .stmts
                        .iter()
                        .map(|s| s.to_token_stream().to_string())
                        .collect(),
                });
            }
            syn::visit::visit_impl_item_fn(self, f);
        }
    }
    let mut v = V {
        out: Vec::new(),
        in_tests: 0,
    };
    v.visit_file(file);
    v.out
}

pub fn function<'a>(funcs: &'a [Func], name: &str) -> Result<&'a Func, String> {
    funcs
        .iter()
        .find(|f| f.name == name)
        .ok_or_else(|| format!("function `{name}` not found"))
}

/// **Every** definition with this name, not the first one found.
///
/// Slip class 3 in `review/AUDIT-MUTATION-SWEEP.md`: an audit that reads
/// `function(&funcs, "dirs_schema")` is satisfied by the first
/// definition and blind to a second, wrong one in a submodule -- which
/// is how "seconds in the minutes column" walked past
/// `dir_mtime_int32_minutes`. An audit that says "the function named X
/// must have property P" means every X.
pub fn functions_named<'a>(funcs: &'a [Func], name: &str) -> Result<Vec<&'a Func>, String> {
    let found: Vec<&Func> = funcs.iter().filter(|f| f.name == name).collect();
    if found.is_empty() {
        return Err(format!("function `{name}` not found"));
    }
    Ok(found)
}

/// Idents that appear as the last segment of a called path, method
/// name, or `use` path anywhere outside test modules: what the file
/// *reaches for*.
///
/// Aliases are expanded: a file that writes `use crate::x::forbidden as
/// ok; ok()` reaches for `forbidden`, and this reports it.
pub fn referenced_idents(file: &syn::File) -> Vec<String> {
    let res = crate::resolve::resolver(file);
    let mut out = referenced_idents_raw(file);
    for ident in out.clone() {
        let resolved = res.resolve(&ident);
        for seg in resolved.split("::") {
            if !seg.is_empty() {
                out.push(seg.to_string());
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

fn referenced_idents_raw(file: &syn::File) -> Vec<String> {
    struct V {
        out: Vec<String>,
        in_tests: usize,
    }
    impl<'ast> Visit<'ast> for V {
        fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
            let test = m.attrs.iter().any(|a| {
                a.path().is_ident("cfg") && a.to_token_stream().to_string().contains("test")
            });
            if test {
                self.in_tests += 1;
                syn::visit::visit_item_mod(self, m);
                self.in_tests -= 1;
            } else {
                syn::visit::visit_item_mod(self, m);
            }
        }
        fn visit_path(&mut self, p: &'ast syn::Path) {
            if self.in_tests == 0 {
                for seg in &p.segments {
                    self.out.push(seg.ident.to_string());
                }
            }
            syn::visit::visit_path(self, p);
        }
        fn visit_expr_method_call(&mut self, m: &'ast syn::ExprMethodCall) {
            if self.in_tests == 0 {
                self.out.push(m.method.to_string());
            }
            syn::visit::visit_expr_method_call(self, m);
        }
        fn visit_use_path(&mut self, u: &'ast syn::UsePath) {
            if self.in_tests == 0 {
                self.out.push(u.ident.to_string());
            }
            syn::visit::visit_use_path(self, u);
        }
        fn visit_use_name(&mut self, u: &'ast syn::UseName) {
            if self.in_tests == 0 {
                self.out.push(u.ident.to_string());
            }
        }
    }
    let mut v = V {
        out: Vec::new(),
        in_tests: 0,
    };
    v.visit_file(file);
    v.out
}

/// Full paths (`a::b::c`) of every call expression outside tests,
/// **resolved** through the file's `use` table: `use std::fs::metadata
/// as stat_path_inner` then `stat_path_inner(p)` is reported as
/// `std::fs::metadata`. That alias is how the mutation sweep walked past
/// `symlinks_never_followed`.
pub fn call_paths(file: &CachedAst) -> Vec<String> {
    memoised("call_paths", file, || call_paths_uncached(file))
}

fn call_paths_uncached(file: &syn::File) -> Vec<String> {
    let res = crate::resolve::resolver(file);
    call_paths_raw(file)
        .into_iter()
        .map(|p| res.resolve(&p))
        .collect()
}

/// The paths exactly as written.
pub fn call_paths_raw(file: &syn::File) -> Vec<String> {
    struct V {
        out: Vec<String>,
        in_tests: usize,
    }
    impl<'ast> Visit<'ast> for V {
        fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
            let test = m.attrs.iter().any(|a| {
                a.path().is_ident("cfg") && a.to_token_stream().to_string().contains("test")
            });
            if test {
                self.in_tests += 1;
                syn::visit::visit_item_mod(self, m);
                self.in_tests -= 1;
            } else {
                syn::visit::visit_item_mod(self, m);
            }
        }
        fn visit_expr_call(&mut self, c: &'ast syn::ExprCall) {
            if self.in_tests == 0
                && let syn::Expr::Path(p) = &*c.func
            {
                self.out.push(
                    p.path
                        .segments
                        .iter()
                        .map(|s| s.ident.to_string())
                        .collect::<Vec<_>>()
                        .join("::"),
                );
            }
            syn::visit::visit_expr_call(self, c);
        }
    }
    let mut v = V {
        out: Vec::new(),
        in_tests: 0,
    };
    v.visit_file(file);
    v.out
}

/// Every string a production function in this file can produce: plain
/// literals *and* every literal inside `concat!`/`format!`/`json!`/
/// `write!`, plus the concatenation of a multi-literal macro's pieces.
///
/// `concat!("can", " be deleted")` was how the sweep hid a verdict from
/// `agent_interface_facts_not_verdicts`, and `format!("project-{}.json")`
/// was how it hid a JSON sidecar from `store_data_is_parquet_not_json_sidecars`.
pub fn string_literals(file: &CachedAst) -> Vec<String> {
    memoised("string_literals", file, || string_literals_uncached(file))
}

fn string_literals_uncached(file: &syn::File) -> Vec<String> {
    let mut out = string_literals_raw(file);
    for (_, l) in crate::resolve::literals(file) {
        out.push(l);
    }
    out.sort();
    out.dedup();
    out
}

/// Plain `syn::LitStr` nodes only.
pub fn string_literals_raw(file: &syn::File) -> Vec<String> {
    struct V {
        out: Vec<String>,
        in_tests: usize,
    }
    impl<'ast> Visit<'ast> for V {
        fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
            let test = m.attrs.iter().any(|a| {
                a.path().is_ident("cfg") && a.to_token_stream().to_string().contains("test")
            });
            if test {
                self.in_tests += 1;
                syn::visit::visit_item_mod(self, m);
                self.in_tests -= 1;
            } else {
                syn::visit::visit_item_mod(self, m);
            }
        }
        fn visit_lit_str(&mut self, l: &'ast syn::LitStr) {
            if self.in_tests == 0 {
                self.out.push(l.value());
            }
        }
        // Doc comments are `#[doc = "..."]` literals; prose about a word
        // is not use of the word.
        fn visit_attribute(&mut self, _a: &'ast syn::Attribute) {}
    }
    let mut v = V {
        out: Vec::new(),
        in_tests: 0,
    };
    v.visit_file(file);
    v.out
}

/// Names of types that `impl <Trait> for <Type>` in this file.
pub fn impls_of(file: &syn::File, trait_name: &str) -> Vec<String> {
    // Resolved, not spelled: `use crate::bus::Consumer as Stage; impl
    // Stage for X` is an impl of `Consumer`, and a trait-name needle
    // alone does not see it (sweep slip class 1).
    let res = crate::resolve::resolver(file);
    let mut out = Vec::new();
    for item in &file.items {
        if let syn::Item::Impl(i) = item
            && let Some((_, path, _)) = &i.trait_
            && let syn::Type::Path(tp) = &*i.self_ty
        {
            let written = path.to_token_stream().to_string();
            let named = path
                .segments
                .last()
                .map(|s| s.ident.to_string())
                .unwrap_or_default();
            if named == trait_name || res.is(&written, trait_name) {
                out.push(tp.path.segments.last().unwrap().ident.to_string());
            }
        }
    }
    out
}

/// A site where `what` is built or called (a struct literal whose path
/// ends in `what`, a call whose path ends in `what`, or a method call
/// named `what`), with the token text of every `if let` initializer
/// whose then-branch encloses it and the function it sits in. For
/// "X may only happen under condition Y" audits.
pub struct GuardedSite {
    pub func: String,
    pub enclosing_if_let_inits: Vec<String>,
}

pub fn guarded_sites(file: &syn::File, what: &str) -> Vec<GuardedSite> {
    struct V<'s> {
        what: &'s str,
        func: String,
        inits: Vec<String>,
        in_tests: usize,
        out: Vec<GuardedSite>,
    }
    impl V<'_> {
        fn hit(&mut self) {
            if self.in_tests == 0 {
                self.out.push(GuardedSite {
                    func: self.func.clone(),
                    enclosing_if_let_inits: self.inits.clone(),
                });
            }
        }
    }
    impl<'ast> Visit<'ast> for V<'_> {
        fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
            let test = m.attrs.iter().any(|a| {
                a.path().is_ident("cfg") && a.to_token_stream().to_string().contains("test")
            });
            if test {
                self.in_tests += 1;
                syn::visit::visit_item_mod(self, m);
                self.in_tests -= 1;
            } else {
                syn::visit::visit_item_mod(self, m);
            }
        }
        fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
            let prev = std::mem::replace(&mut self.func, f.sig.ident.to_string());
            syn::visit::visit_item_fn(self, f);
            self.func = prev;
        }
        fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
            let prev = std::mem::replace(&mut self.func, f.sig.ident.to_string());
            syn::visit::visit_impl_item_fn(self, f);
            self.func = prev;
        }
        fn visit_expr_if(&mut self, i: &'ast syn::ExprIf) {
            let mut pushed = false;
            if let syn::Expr::Let(l) = &*i.cond {
                self.inits.push(l.expr.to_token_stream().to_string());
                pushed = true;
            }
            // Only the then-branch is guarded by the condition.
            self.visit_block(&i.then_branch);
            if pushed {
                self.inits.pop();
            }
            if let Some((_, e)) = &i.else_branch {
                self.visit_expr(e);
            }
        }
        fn visit_expr_struct(&mut self, s: &'ast syn::ExprStruct) {
            let path = s
                .path
                .segments
                .iter()
                .map(|x| x.ident.to_string())
                .collect::<Vec<_>>()
                .join("::");
            if path.ends_with(self.what) {
                self.hit();
            }
            syn::visit::visit_expr_struct(self, s);
        }
        fn visit_expr_call(&mut self, c: &'ast syn::ExprCall) {
            if let syn::Expr::Path(p) = &*c.func
                && p.path.segments.last().is_some_and(|s| s.ident == self.what)
            {
                self.hit();
            }
            syn::visit::visit_expr_call(self, c);
        }
        fn visit_expr_method_call(&mut self, m: &'ast syn::ExprMethodCall) {
            if m.method == self.what {
                self.hit();
            }
            syn::visit::visit_expr_method_call(self, m);
        }
    }
    let mut v = V {
        what,
        func: String::new(),
        inits: Vec::new(),
        in_tests: 0,
        out: Vec::new(),
    };
    v.visit_file(file);
    v.out
}

/// Method calls named `method` whose receiver is itself a method call
/// named one of `receiver_methods` (e.g. `entry.path().is_dir()`,
/// `dir.join(x).exists()`): the shapes that ask the filesystem about a
/// *path* and therefore follow symlinks.
pub fn method_on_receiver_methods(
    file: &syn::File,
    method: &str,
    receiver_methods: &[&str],
) -> Vec<String> {
    struct V<'s> {
        method: &'s str,
        recv: &'s [&'s str],
        func: String,
        in_tests: usize,
        out: Vec<String>,
    }
    impl<'ast> Visit<'ast> for V<'_> {
        fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
            let test = m.attrs.iter().any(|a| {
                a.path().is_ident("cfg") && a.to_token_stream().to_string().contains("test")
            });
            if test {
                self.in_tests += 1;
                syn::visit::visit_item_mod(self, m);
                self.in_tests -= 1;
            } else {
                syn::visit::visit_item_mod(self, m);
            }
        }
        fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
            let prev = std::mem::replace(&mut self.func, f.sig.ident.to_string());
            syn::visit::visit_item_fn(self, f);
            self.func = prev;
        }
        fn visit_expr_method_call(&mut self, m: &'ast syn::ExprMethodCall) {
            if self.in_tests == 0
                && m.method == self.method
                && let syn::Expr::MethodCall(inner) = &*m.receiver
                && self.recv.iter().any(|r| inner.method == r)
            {
                self.out
                    .push(format!("{}: {}", self.func, m.to_token_stream()));
            }
            syn::visit::visit_expr_method_call(self, m);
        }
    }
    let mut v = V {
        method,
        recv: receiver_methods,
        func: String::new(),
        in_tests: 0,
        out: Vec::new(),
    };
    v.visit_file(file);
    v.out
}

/// For every `for` loop in every function: the index of the first
/// top-level statement that is a *symlink discard guard* (an `if` whose
/// condition names `is_symlink` and whose body only `continue`s or
/// `return`s), and the index of the first top-level statement that
/// descends (`is_dir`). A guard nested inside the descent's own `if` is
/// not a guard: the link was already followed by then.
pub struct LoopOrder {
    pub func: String,
    pub guard: Option<usize>,
    pub descent: Option<usize>,
}

pub fn descent_guard_order(file: &syn::File) -> Vec<LoopOrder> {
    fn is_discard_guard(stmt: &syn::Stmt) -> bool {
        let syn::Stmt::Expr(syn::Expr::If(i), _) = stmt else {
            return false;
        };
        if !i.cond.to_token_stream().to_string().contains("is_symlink") {
            return false;
        }
        // A discard may do bookkeeping first (count the symlink); it
        // must end by leaving the iteration.
        matches!(
            i.then_branch.stmts.last(),
            Some(syn::Stmt::Expr(syn::Expr::Continue(_), _))
                | Some(syn::Stmt::Expr(syn::Expr::Return(_), _))
        )
    }
    struct V {
        func: String,
        in_tests: usize,
        out: Vec<LoopOrder>,
    }
    impl<'ast> Visit<'ast> for V {
        fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
            let test = m.attrs.iter().any(|a| {
                a.path().is_ident("cfg") && a.to_token_stream().to_string().contains("test")
            });
            if test {
                self.in_tests += 1;
                syn::visit::visit_item_mod(self, m);
                self.in_tests -= 1;
            } else {
                syn::visit::visit_item_mod(self, m);
            }
        }
        fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
            let prev = std::mem::replace(&mut self.func, f.sig.ident.to_string());
            syn::visit::visit_item_fn(self, f);
            self.func = prev;
        }
        fn visit_expr_for_loop(&mut self, l: &'ast syn::ExprForLoop) {
            if self.in_tests == 0 {
                let guard = l.body.stmts.iter().position(is_discard_guard);
                let descent = l
                    .body
                    .stmts
                    .iter()
                    .position(|s| s.to_token_stream().to_string().contains("is_dir"));
                self.out.push(LoopOrder {
                    func: self.func.clone(),
                    guard,
                    descent,
                });
            }
            syn::visit::visit_expr_for_loop(self, l);
        }
    }
    let mut v = V {
        func: String::new(),
        in_tests: 0,
        out: Vec::new(),
    };
    v.visit_file(file);
    v.out
}

/// The token text of a function's final expression/statement.
pub fn tail_of(func: &Func) -> String {
    func.stmts.last().cloned().unwrap_or_default()
}

pub fn enum_has_variant(file: &syn::File, enum_name: &str, variant: &str) -> bool {
    file.items.iter().any(|i| {
        matches!(i, syn::Item::Enum(e) if e.ident == enum_name && e.variants.iter().any(|v| v.ident == variant))
    })
}

/// One `pub` field of a `pub` struct in this file: the struct's name, the
/// field's name, and the token text of its declared type.
pub struct PubField {
    pub struct_name: String,
    pub field: String,
    pub ty: String,
    /// The field's attribute text (`#[serde(...)]` and friends), so an
    /// audit can tell a field serde always emits from one it hides.
    pub attrs: String,
}

/// Every `pub` field of every struct in the file (test modules excluded).
/// What an audit needs to ask "is this declared surface actually
/// delivered?".
/// Replaces whole-identifier occurrences of `from` with `to`, so
/// rewriting `Fact` in `Vec<Fact>` does not also rewrite `FactStatus`.
fn replace_ident(text: &str, from: &str, to: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < text.len() {
        if text[i..].starts_with(from) {
            let before_ok = i == 0 || {
                let c = bytes[i - 1] as char;
                !c.is_alphanumeric() && c != '_'
            };
            let after = i + from.len();
            let after_ok = after >= text.len() || {
                let c = bytes[after] as char;
                !c.is_alphanumeric() && c != '_'
            };
            if before_ok && after_ok {
                out.push_str(to);
                i = after;
                continue;
            }
        }
        let ch = text[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

pub fn pub_struct_fields(file: &syn::File) -> Vec<PubField> {
    // `ty` is resolved through the file's `use` renames: a field
    // declared `Vec<Fact>` where `use evidence::Evidence as Fact` is a
    // field of `Evidence`, and a rule reading the written spelling
    // alone misses it (sweep slip class 1).
    let res = crate::resolve::resolver(file);
    let mut out = Vec::new();
    for item in &file.items {
        let syn::Item::Struct(s) = item else { continue };
        let syn::Fields::Named(named) = &s.fields else {
            continue;
        };
        for f in &named.named {
            if !matches!(f.vis, syn::Visibility::Public(_)) {
                continue;
            }
            let Some(ident) = &f.ident else { continue };
            out.push(PubField {
                struct_name: s.ident.to_string(),
                field: ident.to_string(),
                ty: {
                    let written = f.ty.to_token_stream().to_string();
                    let mut resolved = written.clone();
                    for (alias, full) in res.alias_pairs() {
                        resolved = replace_ident(&resolved, &alias, &full);
                    }
                    format!("{written} {resolved}")
                },
                attrs: f
                    .attrs
                    .iter()
                    .map(|a| a.to_token_stream().to_string())
                    .collect::<Vec<_>>()
                    .join(" "),
            });
        }
    }
    out
}

/// For every struct literal in the file, the token text of the
/// initializer given to field `field` (one entry per literal that names
/// it). `..Default::default()` and shorthand `field` are returned as the
/// field name itself.
pub fn struct_field_inits(file: &syn::File, field: &str) -> Vec<String> {
    struct V<'s> {
        field: &'s str,
        in_tests: usize,
        out: Vec<String>,
    }
    impl<'ast> Visit<'ast> for V<'_> {
        fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
            let test = m.attrs.iter().any(|a| {
                a.path().is_ident("cfg") && a.to_token_stream().to_string().contains("test")
            });
            if test {
                self.in_tests += 1;
                syn::visit::visit_item_mod(self, m);
                self.in_tests -= 1;
            } else {
                syn::visit::visit_item_mod(self, m);
            }
        }
        fn visit_expr_struct(&mut self, s: &'ast syn::ExprStruct) {
            if self.in_tests == 0 {
                for fv in &s.fields {
                    if let syn::Member::Named(n) = &fv.member
                        && n == self.field
                    {
                        self.out.push(fv.expr.to_token_stream().to_string());
                    }
                }
            }
            syn::visit::visit_expr_struct(self, s);
        }
    }
    let mut v = V {
        field,
        in_tests: 0,
        out: Vec::new(),
    };
    v.visit_file(file);
    v.out
}

/// Every struct-literal *expression* in the file (not a pattern), as
/// `(enclosing function, full path)`. Patterns like
/// `FactStatus::Unknown { reason }` in a `match` arm are matches, not
/// constructions, and are deliberately absent.
pub fn struct_literal_sites(file: &syn::File) -> Vec<(String, String)> {
    struct V {
        func: String,
        in_tests: usize,
        out: Vec<(String, String)>,
    }
    impl<'ast> Visit<'ast> for V {
        fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
            let test = m.attrs.iter().any(|a| {
                a.path().is_ident("cfg") && a.to_token_stream().to_string().contains("test")
            });
            if test {
                self.in_tests += 1;
                syn::visit::visit_item_mod(self, m);
                self.in_tests -= 1;
            } else {
                syn::visit::visit_item_mod(self, m);
            }
        }
        fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
            let prev = std::mem::replace(&mut self.func, f.sig.ident.to_string());
            syn::visit::visit_item_fn(self, f);
            self.func = prev;
        }
        fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
            let prev = std::mem::replace(&mut self.func, f.sig.ident.to_string());
            syn::visit::visit_impl_item_fn(self, f);
            self.func = prev;
        }
        fn visit_expr_struct(&mut self, s: &'ast syn::ExprStruct) {
            if self.in_tests == 0 {
                self.out.push((
                    self.func.clone(),
                    s.path
                        .segments
                        .iter()
                        .map(|x| x.ident.to_string())
                        .collect::<Vec<_>>()
                        .join("::"),
                ));
            }
            syn::visit::visit_expr_struct(self, s);
        }
    }
    let mut v = V {
        func: String::new(),
        in_tests: 0,
        out: Vec::new(),
    };
    v.visit_file(file);
    v.out
}
