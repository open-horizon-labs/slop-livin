//! The build-artifact adapter guardrails (`GUARDRAILS_SPEC.md` section
//! 18), written against the structural causes re-review 3 found rather
//! than against the shapes their author had in mind
//! (`review/REVIEW-STACK-3.md` section 1: 43 of 45 audits accepted a
//! compiling, harmful mutation, and 31 of the 43 slips were "a list that
//! does not contain the thing").
//!
//! Every set these rules range over is **derived**, never written down:
//!
//! * **Governed modules**: every `.rs` file under
//!   `crates/core/src/build_adapters/` -- including the shared model
//!   (`mod.rs`), the registry, the matrix, the neutral helper
//!   (`jvm_common.rs`) and the bounded reader (`bounded_io.rs`) -- plus
//!   *any file anywhere in the workspace* that contains an
//!   `impl BuildAdapter for ..`. No module is exempt by name. Exempting
//!   the shared helpers is how re-review 3 got a `$HOME` read into
//!   `vscode_family.rs` and a recursive walk into `bounded_io.rs`.
//! * **Adapters**: the files holding `impl BuildAdapter for T`, with `T`
//!   read from the impl, so an adapter cannot escape by living outside
//!   the directory or by naming its type something other than `Adapter`.
//! * **Adapter ids**: the string literal each adapter's `fn id` returns.
//! * **The registry**: the governed file that defines `with_builtins`.
//! * **The builder**: `NestedUnitBuilder`; its own inherent `impl` block
//!   is the one place a unit's fields are written.
//! * **Unit fields**: the field names of `NestedArtifact` (and of its
//!   `ArtifactCoverage`), parsed from `crates/core/src/artifact.rs`, so a
//!   field added tomorrow is governed tomorrow.
//! * **Reachability**: the whole-workspace call graph -- free functions,
//!   calls inside known macros' arguments, and method calls resolved to
//!   the `impl` methods of the types the calling function names. A
//!   governed function fails if it *reaches* a forbidden primitive
//!   through any number of calls in any file.
//!
//! The primitive sets are still lists -- there is no way to avoid naming
//! `std::fs::remove_dir_all` -- but they are lists of resolved targets
//! reached transitively, and where a whole namespace is the hazard
//! (`Command`, `OpenOptions`, `crate::actions`, `walkdir`) the rule
//! matches the namespace, not one of its members. Where a list must be
//! an allow-list (the methods a governed function may call on a unit's
//! field), it fails closed: an unlisted method is a violation.
//!
//! Limits (also in each guardrail's Limits section): the graph is
//! lexical. Trait-object dispatch (`adapter.identify(..)` through
//! `dyn BuildAdapter`), function pointers and closures stored in a struct
//! are not followed; a method call is resolved only to the `impl`s of
//! types the calling function names in its signature or body. The
//! runtime complements are the per-adapter contract tests and
//! `crates/core/tests/build_adapter_{contract,cost,history}.rs`.

use crate::ast;
use quote::ToTokens;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, Mutex, OnceLock};
use syn::visit::Visit;

const BUILD_ADAPTERS_DIR: &str = "crates/core/src/build_adapters";
const ARTIFACT_RS: &str = "crates/core/src/artifact.rs";
const BUILDER: &str = "NestedUnitBuilder";
const CACHE_TYPE: &str = "ContainerCache";

/// `(file, function, cap constant)`: the functions allowed to do the
/// bounded thing, because they *are* the bound. Each is checked to still
/// name its cap in its body, so "exempt" cannot quietly become "exempt
/// and unbounded", and each is a cut point of the call graph. The one
/// hand-written list in this module, and a list of exemptions that are
/// themselves audited.
const BOUNDED_PRIMITIVES: &[(&str, &str, &str)] = &[
    (
        "crates/core/src/build_adapters/bounded_io.rs",
        "read_manifest",
        "MAX_MANIFEST_BYTES",
    ),
    (
        "crates/core/src/build_adapters/bounded_io.rs",
        "read_whole_manifest",
        "MAX_MANIFEST_BYTES",
    ),
    (
        "crates/core/src/locations/mod.rs",
        "shallow_list",
        "SHALLOW_LIST_CAP",
    ),
    (
        "crates/core/src/locations/mod.rs",
        "shallow_dir_names",
        "SHALLOW_LIST_CAP",
    ),
];

// ---------------------------------------------------------------------
// Parsing, cached by exact text
// ---------------------------------------------------------------------

/// A parsed file. The cache is keyed on the file's exact text, so a
/// mutated copy is a miss and a verdict never comes from stale
/// structure; it exists because the mutation corpus re-runs these rules
/// over a whole workspace once per fixture.
struct Parsed {
    text: String,
    ast: syn::File,
}

thread_local! {
    static PARSED: std::cell::RefCell<HashMap<(String, String), Rc<Parsed>>> =
        std::cell::RefCell::new(HashMap::new());
}

fn load(root: &Path, rel: &str) -> Option<Rc<Parsed>> {
    let text = std::fs::read_to_string(root.join(rel)).ok()?;
    let key = (rel.to_string(), text.clone());
    if let Some(hit) = PARSED.with(|c| c.borrow().get(&key).cloned()) {
        return Some(hit);
    }
    let ast = syn::parse_file(&text).ok()?;
    let parsed = Rc::new(Parsed { text, ast });
    PARSED.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() > 4096 {
            c.clear();
        }
        c.insert(key, Rc::clone(&parsed));
    });
    Some(parsed)
}

fn load_or_err(root: &Path, rel: &str) -> Result<Rc<Parsed>, String> {
    load(root, rel).ok_or_else(|| format!("{rel}: missing or not valid Rust"))
}

fn is_cfg_test(attrs: &[syn::Attribute]) -> bool {
    attrs
        .iter()
        .any(|a| a.path().is_ident("cfg") && a.to_token_stream().to_string().contains("test"))
}

// ---------------------------------------------------------------------
// Derived sets
// ---------------------------------------------------------------------

struct Adapter {
    rel: String,
    module: String,
    ty: String,
    id: Option<String>,
}

struct Derived {
    governed: Vec<String>,
    adapters: Vec<Adapter>,
    registry: Option<String>,
}

fn module_name(rel: &str) -> String {
    let file = rel.rsplit('/').next().unwrap_or(rel);
    if file == "mod.rs" {
        rel.trim_end_matches("/mod.rs")
            .rsplit('/')
            .next()
            .unwrap_or(file)
            .to_string()
    } else {
        file.trim_end_matches(".rs").to_string()
    }
}

/// The literal a `fn id(&self) -> &'static str { "x" }` returns.
fn adapter_id(file: &syn::File, ty: &str) -> Option<String> {
    for item in &file.items {
        let syn::Item::Impl(i) = item else { continue };
        let syn::Type::Path(tp) = &*i.self_ty else {
            continue;
        };
        if tp
            .path
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .as_deref()
            != Some(ty)
        {
            continue;
        }
        for it in &i.items {
            if let syn::ImplItem::Fn(f) = it
                && f.sig.ident == "id"
            {
                let text = f.block.to_token_stream().to_string();
                let start = text.find('"')?;
                let rest = &text[start + 1..];
                let end = rest.find('"')?;
                return Some(rest[..end].to_string());
            }
        }
    }
    None
}

fn derive(root: &Path) -> Derived {
    let mut governed: BTreeSet<String> =
        crate::resolve::rust_files_recursive(root, BUILD_ADAPTERS_DIR)
            .into_iter()
            .collect();
    let mut adapters = Vec::new();
    for rel in crate::resolve::workspace_files(root) {
        let Some(f) = load(root, &rel) else { continue };
        let impls = ast::impls_of(&f.ast, "BuildAdapter");
        if impls.is_empty() {
            continue;
        }
        governed.insert(rel.clone());
        for ty in impls {
            adapters.push(Adapter {
                rel: rel.clone(),
                module: module_name(&rel),
                id: adapter_id(&f.ast, &ty),
                ty,
            });
        }
    }
    let registry = governed
        .iter()
        .find(|rel| {
            load(root, rel).is_some_and(|f| {
                ast::functions(&f.ast)
                    .iter()
                    .any(|func| func.name == "with_builtins")
            })
        })
        .cloned();
    Derived {
        governed: governed.into_iter().collect(),
        adapters,
        registry,
    }
}

/// Adapters must exist before any rule means anything; saying so is the
/// first failure rather than a vacuous pass over an empty set.
fn derived_or_err(root: &Path) -> Result<Derived, String> {
    let d = derive(root);
    if d.adapters.is_empty() {
        return Err(
            "no `impl BuildAdapter` anywhere in the workspace: section 18 requires a \
             `BuildAdapter` trait with a static registry and one module per ecosystem family \
             (Cargo ported onto the trait, then Node, Gradle and Maven)"
                .into(),
        );
    }
    Ok(d)
}

/// Field names of `NestedArtifact`, plus those of its `ArtifactCoverage`
/// (whose `supported`/`complete` overstate when written), parsed from
/// `artifact.rs`. `ArtifactVariant`'s fields are deliberately not
/// included: an adapter assembles a variant field by field and then
/// hands it to the builder, and a variant is identity evidence, not a
/// claim about bytes, support or action.
fn unit_fields(root: &Path) -> HashSet<String> {
    let mut out = HashSet::new();
    let Some(f) = load(root, ARTIFACT_RS) else {
        return out;
    };
    let structs: HashMap<String, &syn::ItemStruct> = f
        .ast
        .items
        .iter()
        .filter_map(|i| match i {
            syn::Item::Struct(s) => Some((s.ident.to_string(), s)),
            _ => None,
        })
        .collect();
    let Some(unit) = structs.get("NestedArtifact") else {
        return out;
    };
    for field in &unit.fields {
        let Some(name) = &field.ident else { continue };
        out.insert(name.to_string());
        if field.ty.to_token_stream().to_string() == "ArtifactCoverage"
            && let Some(s) = structs.get("ArtifactCoverage")
        {
            for sub in &s.fields {
                if let Some(n) = &sub.ident {
                    out.insert(n.to_string());
                }
            }
        }
    }
    out
}

fn bounded_primitives_still_bounded(root: &Path) -> Result<(), String> {
    let mut problems = Vec::new();
    for (rel, func, cap) in BOUNDED_PRIMITIVES {
        let Some(f) = load(root, rel) else {
            problems.push(format!(
                "{rel} is missing, but its `{func}` is exempted as a bounded primitive"
            ));
            continue;
        };
        let Some(body) = ast::functions(&f.ast).into_iter().find(|x| &x.name == func) else {
            problems.push(format!(
                "{rel} no longer defines the bounded primitive `{func}`"
            ));
            continue;
        };
        // The cap is named in the body, or the body delegates to
        // another bounded primitive that does (`read_whole_manifest`
        // wraps `read_manifest`).
        let delegates = BOUNDED_PRIMITIVES.iter().any(|(r, other, _)| {
            r == rel && other != func && body.body.contains(&format!("{other} ("))
        });
        if !body.body.contains(cap) && !delegates {
            problems.push(format!(
                "{rel}::{func} is exempted as a bounded primitive but its body does not name its \
                 cap `{cap}`"
            ));
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("\n  "))
    }
}

// ---------------------------------------------------------------------
// Per-file facts for the whole-workspace graph
// ---------------------------------------------------------------------

type Node = (String, String);

/// One call edge's raw material.
#[derive(Clone)]
struct Call {
    func: String,
    path: String,
    written: String,
    method: bool,
}

#[derive(Default)]
struct FileFacts {
    calls: Vec<Call>,
    /// Functions defined here: name, the self type of their `impl` (if
    /// any), and the type-like identifiers their signature and body name.
    defs: Vec<(String, Option<String>, BTreeSet<String>)>,
}

type FactCache = Mutex<HashMap<(String, u64), Arc<FileFacts>>>;
static FACTS: OnceLock<FactCache> = OnceLock::new();

fn text_hash(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

/// Macro arguments parsed as comma-separated expressions, when they
/// are (`vec![..]`, `format!(..)`'s arguments, `assert!(..)`).
fn macro_exprs(tokens: &proc_macro2::TokenStream) -> Vec<syn::Expr> {
    let parser = syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated;
    syn::parse::Parser::parse2(parser, tokens.clone())
        .map(|p| p.into_iter().collect())
        .unwrap_or_default()
}

/// Calls written inside macro arguments, which `syn::visit` does not
/// descend into (`vec![std::fs::read_dir(p)]`).
fn macro_calls(file: &syn::File, res: &crate::resolve::Resolver) -> Vec<Call> {
    struct Outer<'a> {
        func: String,
        in_test: usize,
        res: &'a crate::resolve::Resolver,
        out: Vec<Call>,
    }
    struct Inner<'a, 'b> {
        outer: &'b mut Outer<'a>,
    }
    impl<'ast> Visit<'ast> for Inner<'_, '_> {
        fn visit_expr_call(&mut self, c: &'ast syn::ExprCall) {
            if let syn::Expr::Path(p) = &*c.func {
                let written = p.path.to_token_stream().to_string().replace(' ', "");
                self.outer.out.push(Call {
                    func: self.outer.func.clone(),
                    path: self.outer.res.resolve(&written),
                    written,
                    method: false,
                });
            }
            syn::visit::visit_expr_call(self, c);
        }
        fn visit_expr_method_call(&mut self, m: &'ast syn::ExprMethodCall) {
            self.outer.out.push(Call {
                func: self.outer.func.clone(),
                path: m.method.to_string(),
                written: m.method.to_string(),
                method: true,
            });
            syn::visit::visit_expr_method_call(self, m);
        }
        fn visit_macro(&mut self, m: &'ast syn::Macro) {
            for e in macro_exprs(&m.tokens) {
                self.visit_expr(&e);
            }
        }
    }
    impl<'ast> Visit<'ast> for Outer<'_> {
        fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
            let test = is_cfg_test(&m.attrs);
            self.in_test += usize::from(test);
            syn::visit::visit_item_mod(self, m);
            self.in_test -= usize::from(test);
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
        fn visit_macro(&mut self, m: &'ast syn::Macro) {
            if self.in_test > 0 {
                return;
            }
            let exprs = macro_exprs(&m.tokens);
            let mut inner = Inner { outer: self };
            for e in &exprs {
                inner.visit_expr(e);
            }
        }
    }
    let mut v = Outer {
        func: String::new(),
        in_test: 0,
        res,
        out: Vec::new(),
    };
    v.visit_file(file);
    v.out
}

fn type_idents(text: &str) -> BTreeSet<String> {
    text.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|t| t.chars().next().is_some_and(|c| c.is_ascii_uppercase()))
        .map(str::to_string)
        .collect()
}

fn facts_of(root: &Path, rel: &str) -> Option<Arc<FileFacts>> {
    let f = load(root, rel)?;
    let key = (rel.to_string(), text_hash(&f.text));
    let cache = FACTS.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(hit) = cache.lock().unwrap().get(&key).cloned() {
        return Some(hit);
    }
    let res = crate::resolve::resolver(&f.ast);
    let mut facts = FileFacts::default();
    for c in crate::resolve::production_calls(&f.ast) {
        facts.calls.push(Call {
            func: c.func,
            path: c.path,
            written: c.written,
            method: c.method,
        });
    }
    facts.calls.extend(macro_calls(&f.ast, &res));
    let mut impl_of: HashMap<String, String> = HashMap::new();
    for item in &f.ast.items {
        if let syn::Item::Impl(i) = item
            && let syn::Type::Path(tp) = &*i.self_ty
            && let Some(last) = tp.path.segments.last()
        {
            for it in &i.items {
                if let syn::ImplItem::Fn(m) = it {
                    impl_of.insert(m.sig.ident.to_string(), last.ident.to_string());
                }
            }
        }
    }
    for func in ast::functions(&f.ast) {
        let types = type_idents(&format!("{} {}", func.sig, func.body));
        facts
            .defs
            .push((func.name.clone(), impl_of.get(&func.name).cloned(), types));
    }
    let facts = Arc::new(facts);
    let mut c = cache.lock().unwrap();
    if c.len() > 8192 {
        c.clear();
    }
    c.insert(key, Arc::clone(&facts));
    Some(facts)
}

/// A forbidden target: a resolved path suffix, a whole namespace, or a
/// method name for primitives only ever reached as methods.
#[derive(Clone, Copy)]
enum Target {
    /// `fs::read_dir` matches `std::fs::read_dir`, segment-wise.
    Path(&'static str),
    /// Any path with this segment sequence before its last segment:
    /// `Command` matches `std::process::Command::new`; `actions` matches
    /// `crate::actions::anything`.
    Namespace(&'static str),
    Method(&'static str),
}

fn target_matches(t: Target, call: &Call) -> bool {
    match t {
        Target::Path(p) => !call.method && crate::resolve::path_ends_with(&call.path, p),
        Target::Method(m) => call.method && call.path == m,
        Target::Namespace(ns) => {
            if call.method {
                return false;
            }
            let segs: Vec<&str> = call.path.split("::").filter(|s| !s.is_empty()).collect();
            let want: Vec<&str> = ns.split("::").collect();
            if segs.len() <= want.len() {
                return false;
            }
            segs[..segs.len() - 1]
                .windows(want.len())
                .any(|w| w == want.as_slice())
        }
    }
}

fn target_label(t: Target) -> &'static str {
    match t {
        Target::Path(p) | Target::Namespace(p) | Target::Method(p) => p,
    }
}

/// Which functions reach a forbidden call (or a seeded function), over
/// the whole workspace.
struct Reachability {
    tainted: HashMap<Node, String>,
}

type Edges = HashMap<Node, BTreeSet<Node>>;

impl Reachability {
    fn build(root: &Path, forbidden: &[(Target, &str)], seeds: &[(Node, String)]) -> Self {
        let (edges, mut tainted) = Self::graph(root, forbidden);
        tainted.extend(seeds.iter().cloned());
        Self::propagate(&edges, tainted)
    }

    /// Several seed sets over one call graph: the graph is the expensive
    /// part, and the mutation corpus runs every rule once per fixture.
    fn build_many(root: &Path, seed_sets: &[&[(Node, String)]]) -> Vec<Self> {
        let (edges, base) = Self::graph(root, &[]);
        seed_sets
            .iter()
            .map(|seeds| {
                let mut tainted = base.clone();
                tainted.extend(seeds.iter().cloned());
                Self::propagate(&edges, tainted)
            })
            .collect()
    }

    fn graph(root: &Path, forbidden: &[(Target, &str)]) -> (Edges, HashMap<Node, String>) {
        let files = crate::resolve::workspace_files(root);
        let facts: Vec<(String, Arc<FileFacts>)> = files
            .iter()
            .filter_map(|rel| facts_of(root, rel).map(|f| (rel.clone(), f)))
            .collect();
        let cut: HashSet<Node> = BOUNDED_PRIMITIVES
            .iter()
            .map(|(f, n, _)| ((*f).to_string(), (*n).to_string()))
            .collect();
        type Def<'a> = (Node, Option<&'a str>);
        let mut by_name: HashMap<&str, Vec<Def>> = HashMap::new();
        let mut types_of: HashMap<Node, BTreeSet<String>> = HashMap::new();
        for (rel, f) in &facts {
            for (name, self_ty, types) in &f.defs {
                let node = (rel.clone(), name.clone());
                by_name
                    .entry(name.as_str())
                    .or_default()
                    .push((node.clone(), self_ty.as_deref()));
                types_of
                    .entry(node)
                    .or_default()
                    .extend(types.iter().cloned());
            }
        }
        let mut tainted: HashMap<Node, String> = HashMap::new();
        let mut edges: Edges = HashMap::new();
        for (rel, f) in &facts {
            for c in &f.calls {
                let node = (rel.clone(), c.func.clone());
                if cut.contains(&node) {
                    continue;
                }
                if let Some((t, why)) = forbidden.iter().find(|(t, _)| target_matches(*t, c)) {
                    tainted.entry(node.clone()).or_insert_with(|| {
                        format!("{} (resolved {}, which {why})", c.written, target_label(*t))
                    });
                    continue;
                }
                let segments: Vec<&str> = c.path.split("::").collect();
                let callee = segments.last().copied().unwrap_or_default();
                let module = (segments.len() >= 2).then(|| segments[segments.len() - 2]);
                let caller_types = types_of.get(&node);
                for (target, self_ty) in by_name.get(callee).into_iter().flatten() {
                    if cut.contains(target) || target == &node {
                        continue;
                    }
                    let reachable = if c.method {
                        // A method resolves to the impls of the types the
                        // caller names (`ctx: &BuildCtx` -> BuildCtx::list).
                        self_ty.is_some_and(|t| caller_types.is_some_and(|ts| ts.contains(t)))
                    } else {
                        let same_file = target.0 == *rel;
                        let in_module = module.is_some_and(|m| {
                            target.0.ends_with(&format!("/{m}.rs"))
                                || target.0.ends_with(&format!("/{m}/mod.rs"))
                                || self_ty.is_some_and(|t| t == m)
                        });
                        same_file || in_module
                    };
                    if reachable {
                        edges
                            .entry(node.clone())
                            .or_default()
                            .insert(target.clone());
                    }
                }
            }
        }
        (edges, tainted)
    }

    fn propagate(edges: &Edges, mut tainted: HashMap<Node, String>) -> Self {
        loop {
            let mut grew = false;
            for (caller, callees) in edges {
                if tainted.contains_key(caller) {
                    continue;
                }
                if let Some(c) = callees.iter().find(|c| tainted.contains_key(*c)) {
                    let why = tainted[c].clone();
                    tainted.insert(caller.clone(), format!("{}::{} -> {why}", c.0, c.1));
                    grew = true;
                }
            }
            if !grew {
                break;
            }
        }
        Self { tainted }
    }

    fn why(&self, rel: &str, func: &str) -> Option<&String> {
        self.tainted.get(&(rel.to_string(), func.to_string()))
    }
}

/// Forbidden targets written as text inside a macro's tokens that did
/// not parse as expressions (the parsed ones are already call edges).
fn macro_token_hits(f: &syn::File, forbidden: &[(Target, &str)]) -> Vec<String> {
    let mut out = Vec::new();
    for site in crate::resolve::macro_sites(f) {
        if site.in_test {
            continue;
        }
        for (t, why) in forbidden {
            let needle = match t {
                Target::Path(p) | Target::Namespace(p) => p.replace("::", " :: "),
                Target::Method(m) => format!(". {m} ("),
            };
            if site.tokens.contains(&needle) {
                out.push(format!(
                    "{}: the `{}!` macro's tokens reach `{}`, which {why}",
                    site.func,
                    site.name,
                    target_label(*t)
                ));
            }
        }
    }
    out
}

fn unknown_macro_problems(rel: &str, f: &syn::File) -> Vec<String> {
    crate::resolve::unknown_macros(f)
        .into_iter()
        .map(|(func, name)| {
            format!(
                "{rel}::{func} uses the macro `{name}!`, which this layer cannot see through: a \
                 governed module may not hide a call inside a macro no rule can read"
            )
        })
        .collect()
}

fn finish(mut problems: Vec<String>) -> Result<(), String> {
    if problems.is_empty() {
        Ok(())
    } else {
        problems.sort();
        problems.dedup();
        Err(problems.join("\n  "))
    }
}

/// The shared shape of the three primitive rules.
fn primitive_rule(root: &Path, forbidden: &[(Target, &str)], advice: &str) -> Result<(), String> {
    let d = derived_or_err(root)?;
    bounded_primitives_still_bounded(root)?;
    let reach = Reachability::build(root, forbidden, &[]);
    let cut: HashSet<Node> = BOUNDED_PRIMITIVES
        .iter()
        .map(|(f, n, _)| ((*f).to_string(), (*n).to_string()))
        .collect();
    let mut problems: Vec<String> = Vec::new();
    for rel in &d.governed {
        let f = load_or_err(root, rel)?;
        for func in ast::functions(&f.ast) {
            if cut.contains(&(rel.clone(), func.name.clone())) {
                continue;
            }
            if let Some(why) = reach.why(rel, &func.name) {
                problems.push(format!("{rel}::{} reaches `{why}`: {advice}", func.name));
            }
        }
        for hit in macro_token_hits(&f.ast, forbidden) {
            problems.push(format!("{rel}::{hit}: {advice}"));
        }
        // Referenced at all, not only called: a primitive taken as a
        // function pointer (`let list = std::fs::read_dir; list(p)`) or
        // named in a type is never a call edge.
        for (func, path) in referenced_paths_by_fn(rel, &f.ast) {
            if cut.contains(&(rel.clone(), func.clone())) {
                continue;
            }
            let as_call = Call {
                func: func.clone(),
                path: path.clone(),
                written: path.clone(),
                method: false,
            };
            if let Some((t, why)) = forbidden
                .iter()
                .find(|(t, _)| !matches!(t, Target::Method(_)) && target_matches(*t, &as_call))
            {
                problems.push(format!(
                    "{rel}::{} names `{path}` (matching `{}`, which {why}): {advice}",
                    if func.is_empty() { "<item>" } else { &func },
                    target_label(*t)
                ));
            }
        }
        problems.extend(unknown_macro_problems(rel, &f.ast));
    }
    finish(problems)
}

// ---------------------------------------------------------------------
// The primitive sets
// ---------------------------------------------------------------------

/// Anything that changes the filesystem, runs someone else's code, or
/// reaches the plan/authorization/execution layer.
const DESTRUCTIVE: &[(Target, &str)] = &[
    (Target::Path("fs::rename"), "renames"),
    (Target::Path("fs::remove_file"), "deletes"),
    (Target::Path("fs::remove_dir"), "deletes"),
    (Target::Path("fs::remove_dir_all"), "deletes a whole tree"),
    (Target::Path("fs::write"), "overwrites"),
    (Target::Path("fs::create_dir"), "creates"),
    (Target::Path("fs::create_dir_all"), "creates"),
    (Target::Path("fs::set_permissions"), "changes permissions"),
    (Target::Path("fs::copy"), "writes"),
    (Target::Path("fs::hard_link"), "writes"),
    (Target::Path("fs::soft_link"), "writes"),
    (Target::Path("fs::symlink"), "writes"),
    (Target::Path("File::create"), "truncates and writes"),
    (Target::Path("File::create_new"), "writes"),
    (Target::Namespace("OpenOptions"), "opens a file for writing"),
    (Target::Namespace("Command"), "runs another program"),
    (
        Target::Namespace("process"),
        "runs or controls another process",
    ),
    (Target::Namespace("trash"), "moves to Trash"),
    (
        Target::Namespace("actions"),
        "reaches the plan/execution layer",
    ),
    (Target::Namespace("grants"), "mints authorization"),
    (Target::Namespace("ledger"), "records an execution"),
    (
        Target::Namespace("execution"),
        "reaches the execution layer",
    ),
    (
        Target::Namespace("cargo_cleanup"),
        "reaches a cleanup planner",
    ),
];

/// Anything that reads a file without a cap.
const UNBOUNDED_READS: &[(Target, &str)] = &[
    (Target::Path("fs::read_to_string"), "reads a whole file"),
    (Target::Path("fs::read"), "reads a whole file"),
    (Target::Path("io::read_to_string"), "reads a whole reader"),
    (Target::Path("File::open"), "opens an uncapped handle"),
    (Target::Namespace("OpenOptions"), "opens an uncapped handle"),
    (Target::Path("BufReader::new"), "wraps an uncapped reader"),
    (
        Target::Path("serde_json::from_reader"),
        "reads a whole reader",
    ),
    (Target::Method("read_to_end"), "reads to the end"),
    (Target::Method("read_to_string"), "reads to the end"),
    (Target::Method("read_line"), "reads without a cap"),
];

/// Anything that enumerates a directory or starts a walk -- the report
/// path's own walkers included, by namespace.
const TRAVERSALS: &[(Target, &str)] = &[
    (Target::Path("fs::read_dir"), "enumerates a directory"),
    (Target::Namespace("walkdir"), "walks a tree"),
    (Target::Namespace("WalkDir"), "walks a tree"),
    (Target::Namespace("jwalk"), "walks a tree"),
    (Target::Namespace("glob"), "enumerates by pattern"),
    (Target::Namespace("walk"), "starts the report walk"),
    (
        Target::Namespace("attribution"),
        "starts an attribution pass",
    ),
    (
        Target::Namespace("folded_measurement"),
        "re-measures a tree",
    ),
];

// ---------------------------------------------------------------------
// Paths a file references, resolved
// ---------------------------------------------------------------------

/// Every path a file's production items reference -- `use` trees
/// (including `as` renames and `{..}` groups), expression and type
/// paths, item-level consts and statics, and `a::b` paths inside macro
/// tokens -- resolved through the file's symbol table, with `super::` and
/// `self::` made absolute for files in the core crate. Test modules
/// excluded.
fn referenced_paths(rel: &str, file: &syn::File) -> Vec<String> {
    referenced_paths_by_fn(rel, file)
        .into_iter()
        .map(|(_, p)| p)
        .collect()
}

/// [`referenced_paths`], with the enclosing function (`""` at item level).
fn referenced_paths_by_fn(rel: &str, file: &syn::File) -> Vec<(String, String)> {
    struct V<'a> {
        res: &'a crate::resolve::Resolver,
        in_test: usize,
        func: String,
        out: Vec<(String, String)>,
    }
    fn use_paths(tree: &syn::UseTree, prefix: &str, out: &mut Vec<String>) {
        let join = |p: &str, s: &str| {
            if p.is_empty() {
                s.to_string()
            } else {
                format!("{p}::{s}")
            }
        };
        match tree {
            syn::UseTree::Path(p) => use_paths(&p.tree, &join(prefix, &p.ident.to_string()), out),
            syn::UseTree::Name(n) => out.push(join(prefix, &n.ident.to_string())),
            syn::UseTree::Rename(r) => out.push(join(prefix, &r.ident.to_string())),
            syn::UseTree::Glob(_) => out.push(join(prefix, "*")),
            syn::UseTree::Group(g) => {
                for t in &g.items {
                    use_paths(t, prefix, out);
                }
            }
        }
    }
    impl<'ast> Visit<'ast> for V<'_> {
        fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
            let test = is_cfg_test(&m.attrs);
            self.in_test += usize::from(test);
            syn::visit::visit_item_mod(self, m);
            self.in_test -= usize::from(test);
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
        fn visit_item_use(&mut self, u: &'ast syn::ItemUse) {
            if self.in_test == 0 {
                let mut paths = Vec::new();
                use_paths(&u.tree, "", &mut paths);
                let func = self.func.clone();
                self.out
                    .extend(paths.into_iter().map(|p| (func.clone(), p)));
            }
        }
        fn visit_path(&mut self, p: &'ast syn::Path) {
            if self.in_test == 0 {
                let written = p.to_token_stream().to_string().replace(' ', "");
                self.out
                    .push((self.func.clone(), self.res.resolve(&written)));
            }
            syn::visit::visit_path(self, p);
        }
        fn visit_macro(&mut self, m: &'ast syn::Macro) {
            if self.in_test == 0 {
                let compact = m.tokens.to_string().replace(" :: ", "::");
                for piece in compact.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == ':'))
                {
                    if piece.contains("::") {
                        self.out.push((self.func.clone(), self.res.resolve(piece)));
                    }
                }
            }
            syn::visit::visit_macro(self, m);
        }
        // Attributes (doc comments included) are metadata.
        fn visit_attribute(&mut self, _a: &'ast syn::Attribute) {}
    }
    let res = crate::resolve::resolver(file);
    let mut v = V {
        res: &res,
        in_test: 0,
        func: String::new(),
        out: Vec::new(),
    };
    v.visit_file(file);
    let module_path: Vec<String> = rel
        .strip_prefix("crates/core/src/")
        .map(|r| {
            let mut parts: Vec<String> = r
                .trim_end_matches(".rs")
                .split('/')
                .map(str::to_string)
                .collect();
            if parts.last().is_some_and(|p| p == "mod") {
                parts.pop();
            }
            parts
        })
        .unwrap_or_default();
    v.out
        .into_iter()
        .map(|(func, p)| {
            let mut segs: Vec<&str> = p.split("::").collect();
            let mut base = module_path.clone();
            let mut rewritten = false;
            while segs.first() == Some(&"super") {
                segs.remove(0);
                base.pop();
                rewritten = true;
            }
            if segs.first() == Some(&"self") {
                segs.remove(0);
                rewritten = true;
            }
            let path = if rewritten {
                let mut out = vec!["crate".to_string()];
                out.extend(base);
                out.extend(segs.iter().map(|s| s.to_string()));
                out.join("::")
            } else {
                p.replace("swamp_core::", "crate::")
            };
            (func, path)
        })
        .collect()
}

/// Whether a resolved path names adapter module `m`: under
/// `build_adapters`, or (for an adapter defined elsewhere) as a
/// top-level core module.
fn names_module(path: &str, m: &str) -> bool {
    let segs: Vec<&str> = path.split("::").collect();
    segs.windows(2)
        .any(|w| w[0] == "build_adapters" && w[1] == m)
        || (segs.len() >= 3 && segs[0] == "crate" && segs[1] == m)
}

// ---------------------------------------------------------------------
// build_adapters_are_pluggable
// ---------------------------------------------------------------------

/// Statement: a build adapter is one line in a static registry, names no
/// other adapter, and nothing dispatches on an adapter's identity -- no
/// `match` over `*_ADAPTER_ID`, and no comparison of an adapter field
/// against an adapter id literal -- anywhere in the source. The registry
/// is the one file that may name adapter modules; the matrix is the one
/// table that may list their ids.
pub fn build_adapters_are_pluggable(root: &Path) -> Result<(), String> {
    let d = derived_or_err(root)?;
    let mut problems: Vec<String> = Vec::new();
    let ids: BTreeSet<String> = d.adapters.iter().filter_map(|a| a.id.clone()).collect();

    // (1) No governed file other than the registry names an adapter
    // module: adapters naming each other, and a "neutral" helper
    // reaching into an adapter (a back door between two adapters).
    for rel in &d.governed {
        if Some(rel) == d.registry.as_ref() {
            continue;
        }
        let me = module_name(rel);
        let f = load_or_err(root, rel)?;
        let paths = referenced_paths(rel, &f.ast);
        for a in &d.adapters {
            if a.module == me {
                continue;
            }
            if let Some(p) = paths.iter().find(|p| names_module(p, &a.module)) {
                problems.push(format!(
                    "{rel} reaches into adapter module `{}` (resolved `{p}`): an adapter names no \
                     other adapter, and a shared helper names none at all -- shared parsing goes \
                     in a neutral module no adapter's code passes through",
                    a.module
                ));
            }
        }
    }

    // (2) No dispatch on adapter identity, anywhere in the workspace.
    for rel in crate::resolve::workspace_files(root) {
        if Some(&rel) == d.registry.as_ref() || rel == format!("{BUILD_ADAPTERS_DIR}/matrix.rs") {
            continue;
        }
        let Some(f) = load(root, &rel) else { continue };
        for arm in crate::resolve::match_arms(&f.ast) {
            for a in &d.adapters {
                let upper = format!("{}_ADAPTER_ID", a.module.to_ascii_uppercase());
                if arm.pattern.contains(&upper) || arm.scrutinee.contains(&upper) {
                    problems.push(format!(
                        "{rel}::{} matches on `{upper}`: dispatch goes through \
                         `build_adapters::Registry`, never a central ecosystem match",
                        arm.func
                    ));
                }
            }
            if arm.scrutinee.contains("adapter")
                && let Some(id) = ids
                    .iter()
                    .find(|id| arm.pattern.contains(&format!("\"{id}\"")))
            {
                problems.push(format!(
                    "{rel}::{} matches an adapter field against the id \"{id}\": presentation and \
                     behaviour follow the unit's roles and the registry's capabilities, never one \
                     adapter's id",
                    arm.func
                ));
            }
        }
        // A const holding an adapter id is the id by another name.
        let aliases: Vec<(String, String)> = f
            .ast
            .items
            .iter()
            .filter_map(|i| match i {
                syn::Item::Const(c) => {
                    let v = c.expr.to_token_stream().to_string();
                    ids.iter()
                        .find(|id| v == format!("\"{id}\""))
                        .map(|id| (c.ident.to_string(), id.clone()))
                }
                _ => None,
            })
            .collect();
        for (func, stmt) in adapter_comparisons(&f.ast) {
            let by_alias = aliases.iter().find(|(name, _)| {
                stmt.split(|c: char| !(c.is_alphanumeric() || c == '_'))
                    .any(|t| t == name)
            });
            if let Some((_, id)) = by_alias {
                problems.push(format!(
                    "{rel}::{func} compares an adapter against a const holding the id \"{id}\": \
                     the id by another name is still a dispatch on one adapter"
                ));
            }
            if let Some(id) = ids.iter().find(|id| stmt.contains(&format!("\"{id}\""))) {
                problems.push(format!(
                    "{rel}::{func} compares an adapter against the id \"{id}\": special-casing one \
                     adapter by id outside the registry is the central dispatch section 13 \
                     removed from the agent side"
                ));
            }
        }
    }

    // (3) Registry <-> adapter set, exactly once each.
    let Some(registry_rel) = d.registry.clone() else {
        return Err(
            "no governed file defines `with_builtins`: section 18 requires a static \
             `build_adapters::Registry::with_builtins()`"
                .into(),
        );
    };
    let registry = load_or_err(root, &registry_rel)?;
    let body = ast::functions(&registry.ast)
        .into_iter()
        .find(|f| f.name == "with_builtins")
        .map(|f| f.body)
        .unwrap_or_default();
    for a in &d.adapters {
        let needle = format!("{} :: {}", a.module, a.ty);
        let count = body
            .match_indices(&needle)
            .filter(|(at, _)| {
                body[..*at]
                    .chars()
                    .next_back()
                    .is_none_or(|c| !c.is_alphanumeric() && c != '_')
            })
            .count();
        if count != 1 {
            problems.push(format!(
                "{registry_rel}::with_builtins registers `{}::{}` {count} times; exactly once (an \
                 unregistered adapter identifies nothing and no test notices, and a duplicate \
                 identifies twice)",
                a.module, a.ty
            ));
        }
        if a.id.is_none() {
            problems.push(format!(
                "{}: `impl BuildAdapter for {}` has no literal `fn id`; the id is how the matrix \
                 and the docs are joined to the code",
                a.rel, a.ty
            ));
        }
    }

    // (4) Adapter ids == the matrix's implemented ids.
    let implemented = matrix_implemented_ids(root)?;
    for id in ids.difference(&implemented) {
        problems.push(format!(
            "build_adapters/matrix.rs has no implemented entry for adapter `{id}`: an adapter with \
             no capability row is an undocumented support claim"
        ));
    }
    for id in implemented.difference(&ids) {
        problems.push(format!(
            "build_adapters/matrix.rs marks `{id}` implemented and no adapter has that id: a \
             documented family with no code behind it"
        ));
    }
    finish(problems)
}

/// Comparisons that could be a dispatch on an adapter's identity: an
/// `==`/`!=` whose own operands, or the receiver of a method chain it
/// sits inside (`u.adapter.as_deref().filter(|a| *a != "x")`), mention
/// `adapter`; an `if let <pattern> = <expr mentioning adapter>`; and a
/// macro (`matches!`) whose tokens mention `adapter`. Returned as the
/// token text to search for an adapter id literal.
fn adapter_comparisons(file: &syn::File) -> Vec<(String, String)> {
    struct V {
        func: String,
        in_test: usize,
        receivers: Vec<String>,
        out: Vec<(String, String)>,
    }
    impl<'ast> Visit<'ast> for V {
        fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
            let test = is_cfg_test(&m.attrs);
            self.in_test += usize::from(test);
            syn::visit::visit_item_mod(self, m);
            self.in_test -= usize::from(test);
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
        fn visit_expr_method_call(&mut self, m: &'ast syn::ExprMethodCall) {
            self.visit_expr(&m.receiver);
            self.receivers
                .push(m.receiver.to_token_stream().to_string());
            for a in &m.args {
                self.visit_expr(a);
            }
            self.receivers.pop();
        }
        fn visit_expr_binary(&mut self, b: &'ast syn::ExprBinary) {
            if self.in_test == 0 && matches!(b.op, syn::BinOp::Eq(_) | syn::BinOp::Ne(_)) {
                let own = b.to_token_stream().to_string();
                if own.contains("adapter") || self.receivers.iter().any(|r| r.contains("adapter")) {
                    self.out.push((self.func.clone(), own));
                }
            }
            syn::visit::visit_expr_binary(self, b);
        }
        fn visit_expr_let(&mut self, l: &'ast syn::ExprLet) {
            if self.in_test == 0 && l.expr.to_token_stream().to_string().contains("adapter") {
                self.out
                    .push((self.func.clone(), l.pat.to_token_stream().to_string()));
            }
            syn::visit::visit_expr_let(self, l);
        }
        fn visit_macro(&mut self, m: &'ast syn::Macro) {
            let t = m.tokens.to_string();
            if self.in_test == 0 && t.contains("adapter") {
                self.out.push((self.func.clone(), t));
            }
        }
    }
    let mut v = V {
        func: String::new(),
        in_test: 0,
        receivers: Vec::new(),
        out: Vec::new(),
    };
    v.visit_file(file);
    v.out
}

/// `MatrixEntry { id: "x", status: Status::Implemented, .. }` ids, read
/// from the struct literals in the matrix file.
fn matrix_implemented_ids(root: &Path) -> Result<BTreeSet<String>, String> {
    let f = load_or_err(root, &format!("{BUILD_ADAPTERS_DIR}/matrix.rs"))?;
    struct V {
        out: BTreeSet<String>,
    }
    impl<'ast> Visit<'ast> for V {
        fn visit_expr_struct(&mut self, s: &'ast syn::ExprStruct) {
            let mut id = None;
            let mut implemented = false;
            for field in &s.fields {
                let name = field.member.to_token_stream().to_string();
                let value = field.expr.to_token_stream().to_string();
                if name == "id" {
                    id = Some(value.trim_matches('"').to_string());
                }
                if name == "status" && value.contains("Implemented") {
                    implemented = true;
                }
            }
            if implemented && let Some(id) = id {
                self.out.insert(id);
            }
            syn::visit::visit_expr_struct(self, s);
        }
    }
    let mut v = V {
        out: BTreeSet::new(),
    };
    v.visit_file(&f.ast);
    Ok(v.out)
}

// ---------------------------------------------------------------------
// The three primitive rules
// ---------------------------------------------------------------------

pub fn build_adapters_are_inspection_only(root: &Path) -> Result<(), String> {
    primitive_rule(
        root,
        DESTRUCTIVE,
        "build identification reads metadata and nothing else. It never writes, never deletes, \
         never runs `npm`, `gradle`, `mvn` or `cargo` -- each of those evaluates the project's own \
         build definition -- and never reaches the plan, grant or execution layer. An adapter \
         declares an action capability; only the shared sink executes anything",
    )
}

pub fn build_adapters_read_bounded_manifests_only(root: &Path) -> Result<(), String> {
    let mut problems = Vec::new();
    match load(root, &format!("{BUILD_ADAPTERS_DIR}/bounded_io.rs")) {
        None => problems.push(
            "build_adapters/bounded_io.rs is missing: build adapters need one shared capped \
             manifest reader (`bounded_io::read_manifest`)"
                .to_string(),
        ),
        Some(b) if !b.text.contains("MAX_MANIFEST_BYTES") => problems.push(
            "build_adapters/bounded_io.rs has no `MAX_MANIFEST_BYTES` cap constant".to_string(),
        ),
        Some(_) => {}
    }
    if let Err(e) = primitive_rule(
        root,
        UNBOUNDED_READS,
        "the only content access in build identification is \
         `build_adapters::bounded_io::read_manifest(path, cap)`: capped at 256 KiB, counted, and \
         for named manifest and metadata files",
    ) {
        problems.push(e);
    }
    finish(problems)
}

pub fn build_adapters_do_not_traverse(root: &Path) -> Result<(), String> {
    primitive_rule(
        root,
        TRAVERSALS,
        "directory structure reaches build identification through the folded walk rows in its \
         context, or through the capped `locations::shallow_list`",
    )
}

// ---------------------------------------------------------------------
// build_units_built_through_builder
// ---------------------------------------------------------------------

/// Methods a governed function may call on a unit's field. An
/// allow-list that fails closed: a method not named here, called on a
/// unit field, is a violation, because it may take `&mut self` (`push`,
/// `retain`, `clear`, `get_or_insert_with`, `iter_mut`, ...).
const READ_ONLY_FIELD_METHODS: &[&str] = &[
    "clone",
    "as_deref",
    "as_ref",
    "as_str",
    "as_path",
    "iter",
    "len",
    "is_empty",
    "contains",
    "starts_with",
    "ends_with",
    "file_name",
    "file_stem",
    "parent",
    "join",
    "display",
    "to_path_buf",
    "to_str",
    "to_string",
    "to_string_lossy",
    "to_owned",
    "ancestors",
    "components",
    "extension",
    "exists",
    "is_dir",
    "is_file",
    "strip_prefix",
    "label",
    "family",
    "title",
    "is_some",
    "is_none",
    "is_some_and",
    "is_none_or",
    "unwrap_or",
    "unwrap_or_default",
    "get",
    "first",
    "last",
    "eq",
    "ne",
    "cmp",
    "partial_cmp",
    "max",
    "min",
    "saturating_sub",
    "checked_sub",
    "split",
    "rsplit_once",
    "split_once",
    "chars",
    "trim",
    "borrow",
    "cloned",
    "copied",
];

/// Every way a function in `file` writes a unit's field: assignment,
/// compound assignment, `&mut` borrow (so `mem::replace`/`mem::take`),
/// a non-read-only method on the field, a `ref mut` binding -- including
/// inside macro arguments -- outside the builder's own inherent `impl`
/// and outside tests.
fn field_writes(file: &syn::File, fields: &HashSet<String>) -> Vec<(String, String)> {
    struct V<'a> {
        fields: &'a HashSet<String>,
        func: String,
        in_test: usize,
        in_builder: usize,
        /// The self type of the enclosing `impl`: inside `impl BuildCtx`,
        /// `self.coverage` is the context's own field, not a unit's.
        self_ty: Vec<String>,
        out: Vec<(String, String)>,
    }
    /// The unit field a place expression writes through, if any: the
    /// members of a field chain, minus the first one when the chain
    /// starts at `self` inside an `impl` of some other type.
    fn unit_field(
        e: &syn::Expr,
        fields: &HashSet<String>,
        self_ty: Option<&str>,
    ) -> Option<String> {
        let mut members: Vec<String> = Vec::new();
        let mut cur = e;
        loop {
            match cur {
                syn::Expr::Field(f) => {
                    members.push(f.member.to_token_stream().to_string());
                    cur = &f.base;
                }
                syn::Expr::Paren(p) => cur = &p.expr,
                syn::Expr::Index(i) => cur = &i.expr,
                _ => break,
            }
        }
        members.reverse();
        let on_self = matches!(cur, syn::Expr::Path(p) if p.path.is_ident("self"));
        if on_self && self_ty.is_some_and(|t| t != "NestedArtifact") && !members.is_empty() {
            members.remove(0);
        }
        members.into_iter().find(|m| fields.contains(m))
    }
    impl V<'_> {
        fn unit_field(&self, e: &syn::Expr) -> Option<String> {
            unit_field(e, self.fields, self.self_ty.last().map(String::as_str))
        }
        fn flag(&mut self, what: String) {
            if self.in_test == 0 && self.in_builder == 0 {
                self.out.push((self.func.clone(), what));
            }
        }
    }
    impl<'ast> Visit<'ast> for V<'_> {
        fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
            let test = is_cfg_test(&m.attrs);
            self.in_test += usize::from(test);
            syn::visit::visit_item_mod(self, m);
            self.in_test -= usize::from(test);
        }
        fn visit_item_impl(&mut self, i: &'ast syn::ItemImpl) {
            let ty = i.self_ty.to_token_stream().to_string();
            let builder = i.trait_.is_none() && ty == BUILDER;
            self.in_builder += usize::from(builder);
            self.self_ty.push(ty);
            syn::visit::visit_item_impl(self, i);
            self.self_ty.pop();
            self.in_builder -= usize::from(builder);
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
        fn visit_expr_assign(&mut self, a: &'ast syn::ExprAssign) {
            if let Some(field) = self.unit_field(&a.left) {
                self.flag(format!(
                    "assigns `{}` (the unit field `{field}`)",
                    a.left.to_token_stream()
                ));
            }
            syn::visit::visit_expr_assign(self, a);
        }
        fn visit_expr_binary(&mut self, b: &'ast syn::ExprBinary) {
            let compound = matches!(
                b.op,
                syn::BinOp::AddAssign(_)
                    | syn::BinOp::SubAssign(_)
                    | syn::BinOp::MulAssign(_)
                    | syn::BinOp::DivAssign(_)
                    | syn::BinOp::RemAssign(_)
                    | syn::BinOp::BitAndAssign(_)
                    | syn::BinOp::BitOrAssign(_)
                    | syn::BinOp::BitXorAssign(_)
                    | syn::BinOp::ShlAssign(_)
                    | syn::BinOp::ShrAssign(_)
            );
            if compound && let Some(field) = self.unit_field(&b.left) {
                self.flag(format!(
                    "compound-assigns `{}` (the unit field `{field}`)",
                    b.left.to_token_stream()
                ));
            }
            syn::visit::visit_expr_binary(self, b);
        }
        fn visit_expr_reference(&mut self, r: &'ast syn::ExprReference) {
            if r.mutability.is_some()
                && let Some(field) = self.unit_field(&r.expr)
            {
                self.flag(format!(
                    "takes `&mut {}` (the unit field `{field}`)",
                    r.expr.to_token_stream()
                ));
            }
            syn::visit::visit_expr_reference(self, r);
        }
        fn visit_expr_method_call(&mut self, m: &'ast syn::ExprMethodCall) {
            let method = m.method.to_string();
            if let Some(field) = self.unit_field(&m.receiver)
                && !READ_ONLY_FIELD_METHODS.contains(&method.as_str())
            {
                self.flag(format!(
                    "calls `.{method}(..)` on `{}` (the unit field `{field}`), which may mutate it",
                    m.receiver.to_token_stream()
                ));
            }
            syn::visit::visit_expr_method_call(self, m);
        }
        fn visit_pat_ident(&mut self, p: &'ast syn::PatIdent) {
            if p.by_ref.is_some()
                && p.mutability.is_some()
                && self.fields.contains(&p.ident.to_string())
            {
                self.flag(format!("binds `ref mut {}` to a unit field", p.ident));
            }
            syn::visit::visit_pat_ident(self, p);
        }
        fn visit_macro(&mut self, m: &'ast syn::Macro) {
            for e in macro_exprs(&m.tokens) {
                self.visit_expr(&e);
            }
        }
    }
    let mut v = V {
        fields,
        func: String::new(),
        in_test: 0,
        in_builder: 0,
        self_ty: Vec::new(),
        out: Vec::new(),
    };
    v.visit_file(file);
    v.out
}

/// Statement: build units come from `NestedUnitBuilder` and only from
/// it -- no `NestedArtifact { .. }`, `ArtifactCoverage { .. }` or
/// `NestedActionCapability::Unsupported { .. }` literal outside the
/// builder's own `impl`, and no write to a unit's field after `build()`
/// in any governed function or any function a governed function reaches.
/// Enrichment re-opens the unit with `NestedUnitBuilder::amend(unit)` and
/// goes through named methods.
pub fn build_units_built_through_builder(root: &Path) -> Result<(), String> {
    let d = derived_or_err(root)?;
    let fields = unit_fields(root);
    if fields.is_empty() {
        return Err(format!(
            "{ARTIFACT_RS} defines no `NestedArtifact` fields to govern; the rule cannot speak \
             about a type it cannot read"
        ));
    }
    let mut problems: Vec<String> = Vec::new();
    let mut builder_defined = false;
    for rel in &d.governed {
        let f = load_or_err(root, rel)?;
        builder_defined |= f
            .ast
            .items
            .iter()
            .any(|i| matches!(i, syn::Item::Struct(s) if s.ident == BUILDER));
        let res = crate::resolve::resolver(&f.ast);
        let builder_fns: HashSet<String> = f
            .ast
            .items
            .iter()
            .filter_map(|i| match i {
                syn::Item::Impl(imp)
                    if imp.trait_.is_none()
                        && imp.self_ty.to_token_stream().to_string() == BUILDER =>
                {
                    Some(imp.items.iter().filter_map(|it| match it {
                        syn::ImplItem::Fn(m) => Some(m.sig.ident.to_string()),
                        _ => None,
                    }))
                }
                _ => None,
            })
            .flatten()
            .collect();
        for (func, path) in ast::struct_literal_sites(&f.ast) {
            if builder_fns.contains(&func) {
                continue;
            }
            let resolved = res.resolve(&path);
            for what in [
                "NestedArtifact",
                "ArtifactCoverage",
                "NestedActionCapability::Unsupported",
                "CandidateNestedUnit",
            ] {
                if crate::resolve::path_ends_with(&resolved, what) {
                    problems.push(format!(
                        "{rel}::{func} builds a `{what} {{ .. }}` literal (written `{path}`): units \
                         are built with `NestedUnitBuilder::new(container, role, path)`, whose \
                         defaults understate -- a literal can get every one of them wrong in the \
                         direction that inflates a total or promises an action"
                    ));
                }
            }
        }
        for (func, what) in field_writes(&f.ast, &fields) {
            problems.push(format!(
                "{rel}::{func} {what} after the unit was built: re-open it with \
                 `NestedUnitBuilder::amend(unit)` and change it through a named builder method, so \
                 the change is visible in the diff"
            ));
        }
        for c in crate::resolve::production_calls(&f.ast) {
            if matches!(
                c.path.as_str(),
                "supported_with_reason" | "acts_with_reason" | "no_action_because"
            ) && c.args.iter().all(|x| {
                let x = x.replace(' ', "");
                x.is_empty() || x == "\"\""
            }) {
                problems.push(format!(
                    "{rel}::{} calls `{}` with no stated reason",
                    c.func, c.path
                ));
            }
        }
    }
    if !builder_defined {
        problems.push(format!("no governed module defines `struct {BUILDER}`"));
    }

    // Writes one call away: a function anywhere else in the workspace
    // whose signature takes a unit mutably (`&mut NestedArtifact`, `&mut
    // [NestedArtifact]`, `&mut self` on `impl NestedArtifact`) and which
    // writes a unit field is a seed, and every governed function that
    // reaches it fails.
    let mut seeds: Vec<(Node, String)> = Vec::new();
    for rel in crate::resolve::workspace_files(root) {
        if d.governed.contains(&rel) {
            continue;
        }
        let Some(f) = load(root, &rel) else { continue };
        if !f.text.contains("NestedArtifact") {
            continue;
        }
        let res = crate::resolve::resolver(&f.ast);
        for (func, path) in ast::struct_literal_sites(&f.ast) {
            if crate::resolve::path_ends_with(&res.resolve(&path), "NestedArtifact") {
                seeds.push((
                    (rel.clone(), func.clone()),
                    format!("{rel}::{func} builds a `NestedArtifact {{ .. }}` literal"),
                ));
            }
        }
        let writes = field_writes(&f.ast, &fields);
        if writes.is_empty() {
            continue;
        }
        let unit_methods: HashSet<String> = f
            .ast
            .items
            .iter()
            .filter_map(|i| match i {
                syn::Item::Impl(imp)
                    if imp.self_ty.to_token_stream().to_string() == "NestedArtifact" =>
                {
                    Some(imp.items.iter().filter_map(|it| match it {
                        syn::ImplItem::Fn(m) => Some(m.sig.ident.to_string()),
                        _ => None,
                    }))
                }
                _ => None,
            })
            .flatten()
            .collect();
        for func in ast::functions(&f.ast) {
            let takes_unit_mutably = (func.sig.contains("mut")
                && func.sig.contains("NestedArtifact"))
                || (unit_methods.contains(&func.name) && func.sig.contains("mut self"));
            if takes_unit_mutably
                && let Some((_, what)) = writes.iter().find(|(n, _)| *n == func.name)
            {
                seeds.push((
                    (rel.clone(), func.name.clone()),
                    format!("{rel}::{} {what}", func.name),
                ));
            }
        }
    }
    if !seeds.is_empty() {
        let reach = Reachability::build(root, &[], &seeds);
        for rel in &d.governed {
            let f = load_or_err(root, rel)?;
            for func in ast::functions(&f.ast) {
                if let Some(why) = reach.why(rel, &func.name) {
                    problems.push(format!(
                        "{rel}::{} reaches a function that writes a unit's field ({why}): the \
                         builder bypass one call away is still a builder bypass",
                        func.name
                    ));
                }
            }
        }
    }
    finish(problems)
}

// ---------------------------------------------------------------------
// build_adapter_test_contract
// ---------------------------------------------------------------------

const REQUIRED_BUILD_TESTS: &[&str] = &[
    "unknown_layout_is_explicit_not_empty",
    "identification_reads_no_more_than_manifest_cap",
    "no_project_or_build_code_is_executed",
    "variants_never_collapse_by_basename",
    "age_is_not_obsolescence",
];

/// One test function as `syn` sees it: a real `fn` item (a name in a
/// comment does not count), inside a `#[cfg(test)]` module, carrying
/// `#[test]`, not `#[ignore]`d, and asserting something.
struct TestFn {
    name: String,
    in_cfg_test: bool,
    is_test: bool,
    ignored: bool,
    asserts: bool,
}

fn test_fns(file: &syn::File) -> Vec<TestFn> {
    struct V {
        in_cfg_test: usize,
        out: Vec<TestFn>,
    }
    fn attr(attrs: &[syn::Attribute], name: &str) -> bool {
        attrs.iter().any(|a| a.path().is_ident(name))
    }
    fn asserts(block: &syn::Block) -> bool {
        let t = block.to_token_stream().to_string();
        ["assert !", "assert_eq !", "assert_ne !", "contract ::"]
            .iter()
            .any(|n| t.contains(n))
    }
    impl<'ast> Visit<'ast> for V {
        fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
            let test = is_cfg_test(&m.attrs);
            self.in_cfg_test += usize::from(test);
            syn::visit::visit_item_mod(self, m);
            self.in_cfg_test -= usize::from(test);
        }
        fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
            self.out.push(TestFn {
                name: f.sig.ident.to_string(),
                in_cfg_test: self.in_cfg_test > 0,
                is_test: attr(&f.attrs, "test"),
                ignored: attr(&f.attrs, "ignore"),
                asserts: asserts(&f.block),
            });
            syn::visit::visit_item_fn(self, f);
        }
    }
    let mut v = V {
        in_cfg_test: 0,
        out: Vec::new(),
    };
    v.visit_file(file);
    v.out
}

pub fn build_adapter_test_contract(root: &Path) -> Result<(), String> {
    let d = derived_or_err(root)?;
    let mut missing: Vec<String> = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    for a in &d.adapters {
        if !seen.insert(a.rel.as_str()) {
            continue;
        }
        let f = load_or_err(root, &a.rel)?;
        let defined = test_fns(&f.ast);
        let absent: Vec<String> = REQUIRED_BUILD_TESTS
            .iter()
            .filter_map(|want| match defined.iter().find(|t| t.name == *want) {
                None => Some(format!(
                    "{want} (no such function -- a name in a comment is not a test)"
                )),
                Some(t) if !t.in_cfg_test => {
                    Some(format!("{want} (not inside a `#[cfg(test)]` module)"))
                }
                Some(t) if !t.is_test => Some(format!("{want} (no `#[test]` attribute)")),
                Some(t) if t.ignored => Some(format!("{want} (`#[ignore]`d)")),
                Some(t) if !t.asserts => Some(format!("{want} (asserts nothing)")),
                Some(_) => None,
            })
            .collect();
        if !absent.is_empty() {
            missing.push(format!("{}: {}", a.rel, absent.join(", ")));
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "every build adapter proves the same five things about itself; these do not:\n  {}",
            missing.join("\n  ")
        ))
    }
}

// ---------------------------------------------------------------------
// build_adapter_matrix_matches_docs
// ---------------------------------------------------------------------

/// Statement: `docs/build-artifacts.md`'s support table, the code's
/// matrix and the registered adapters describe the same set of
/// implemented adapters, in both directions, and no row in either table
/// claims an action. The executable column-by-column equality check is
/// `crates/core/tests/build_adapter_contract.rs::docs_table_equals_the_capability_matrix`;
/// this rule is its structural half.
pub fn build_adapter_matrix_matches_docs(root: &Path) -> Result<(), String> {
    let d = derived_or_err(root)?;
    let doc = std::fs::read_to_string(root.join("docs/build-artifacts.md")).map_err(|e| {
        format!(
            "docs/build-artifacts.md: {e}; section 18 requires a published capability matrix the \
             code is checked against"
        )
    })?;
    let ids: BTreeSet<String> = d.adapters.iter().filter_map(|a| a.id.clone()).collect();
    let implemented = matrix_implemented_ids(root)?;
    let mut problems = Vec::new();
    let mut doc_implemented: BTreeSet<String> = BTreeSet::new();
    for line in doc.lines() {
        if !line.starts_with('|') {
            continue;
        }
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        if cells.len() < 4 {
            continue;
        }
        let Some(first) = cells[1].strip_prefix('`') else {
            continue;
        };
        let Some(id) = first.split('`').next() else {
            continue;
        };
        let status = cells[2].to_ascii_lowercase();
        if status != "implemented" && status != "planned" {
            continue;
        }
        if status == "implemented" {
            doc_implemented.insert(id.to_string());
        }
        let actions = cells[cells.len() - 2].to_ascii_lowercase();
        if actions != "inspection only" {
            problems.push(format!(
                "docs/build-artifacts.md row `{id}` claims `{actions}`: no build adapter \
                 implements an action, so any claim but `inspection only` is unkept"
            ));
        }
    }
    for id in ids.difference(&doc_implemented) {
        problems.push(format!(
            "docs/build-artifacts.md has no implemented support row for adapter `{id}`"
        ));
    }
    for id in doc_implemented.difference(&ids) {
        problems.push(format!(
            "docs/build-artifacts.md marks `{id}` implemented and no adapter has that id"
        ));
    }
    for id in ids.difference(&implemented) {
        problems.push(format!(
            "build_adapters/matrix.rs has no implemented entry for `{id}`"
        ));
    }
    for id in implemented.difference(&ids) {
        problems.push(format!(
            "build_adapters/matrix.rs marks `{id}` implemented and no adapter has that id"
        ));
    }
    let matrix = load_or_err(root, &format!("{BUILD_ADAPTERS_DIR}/matrix.rs"))?;
    struct V {
        out: Vec<String>,
    }
    impl<'ast> Visit<'ast> for V {
        fn visit_expr_struct(&mut self, s: &'ast syn::ExprStruct) {
            for field in &s.fields {
                if field.member.to_token_stream().to_string() == "actions" {
                    let v = field.expr.to_token_stream().to_string();
                    if v != "INSPECTION_ONLY" && v != "\"inspection only\"" {
                        self.out.push(v);
                    }
                }
            }
            syn::visit::visit_expr_struct(self, s);
        }
    }
    let mut v = V { out: Vec::new() };
    v.visit_file(&matrix.ast);
    for claim in v.out {
        problems.push(format!(
            "build_adapters/matrix.rs claims `{claim}`: no build adapter implements an action"
        ));
    }
    let defines_inspection_only = matrix.ast.items.iter().any(|i| {
        matches!(i, syn::Item::Const(c)
            if c.ident == "INSPECTION_ONLY"
                && c.expr.to_token_stream().to_string() == "\"inspection only\"")
    });
    if !defines_inspection_only {
        problems.push(
            "build_adapters/matrix.rs no longer defines `INSPECTION_ONLY` as \"inspection only\"; \
             a redefined constant is a claimed action by another name"
                .into(),
        );
    }
    finish(problems)
}

// ---------------------------------------------------------------------
// build_adapters_reuse_under_event_coverage
// ---------------------------------------------------------------------

/// Statement: stored build units are replayed only when this pass's
/// `EventCoverage` vouches for the container, and that answer is
/// honoured; no governed function decides reuse from a stamp.
///
/// Derived: the *replay sites* are the governed functions outside the
/// cache type's own `impl` that read the cache's storage
/// (`<..>.cache.entries`). Every one of them must reach, within its file,
/// a call to `unchanged_since` whose result is not discarded -- a
/// `let _ = coverage.unchanged_since(..)` is a removed gate. Separately,
/// a governed function that mentions reuse or replay and reads an
/// `mtime` with no honoured coverage call anywhere it reaches fails.
pub fn build_adapters_reuse_under_event_coverage(root: &Path) -> Result<(), String> {
    let d = derived_or_err(root)?;
    let mut problems = Vec::new();
    let mut replay_sites = 0usize;
    // Accessors on the cache type: every inherent method that does not
    // construct one. Reading stored units through one of them is a
    // replay site exactly as reading its storage field is.
    let mut cache_readers: Vec<String> = Vec::new();
    for rel in &d.governed {
        let f = load_or_err(root, rel)?;
        for item in &f.ast.items {
            if let syn::Item::Impl(imp) = item
                && imp.self_ty.to_token_stream().to_string() == CACHE_TYPE
            {
                for it in &imp.items {
                    if let syn::ImplItem::Fn(m) = it {
                        let ret = m.sig.output.to_token_stream().to_string();
                        if !ret.contains("Self") && !ret.contains(CACHE_TYPE) {
                            cache_readers.push(m.sig.ident.to_string());
                        }
                    }
                }
            }
        }
    }
    for rel in &d.governed {
        let f = load_or_err(root, rel)?;
        let funcs = ast::functions(&f.ast);
        let calls = crate::resolve::production_calls(&f.ast);
        let by_name: HashMap<&str, &ast::Func> =
            funcs.iter().map(|x| (x.name.as_str(), x)).collect();
        let cache_impl: HashSet<String> = f
            .ast
            .items
            .iter()
            .filter_map(|i| match i {
                syn::Item::Impl(imp) if imp.self_ty.to_token_stream().to_string() == CACHE_TYPE => {
                    Some(imp.items.iter().filter_map(|it| match it {
                        syn::ImplItem::Fn(m) => Some(m.sig.ident.to_string()),
                        _ => None,
                    }))
                }
                _ => None,
            })
            .flatten()
            .collect();
        let reach = |start: &str| -> HashSet<String> {
            let mut seen: HashSet<String> = HashSet::new();
            let mut stack = vec![start.to_string()];
            while let Some(n) = stack.pop() {
                if !seen.insert(n.clone()) {
                    continue;
                }
                for c in calls.iter().filter(|c| c.func == n) {
                    let callee = c.path.rsplit("::").next().unwrap_or(&c.path).to_string();
                    if by_name.contains_key(callee.as_str()) {
                        stack.push(callee);
                    }
                }
            }
            seen
        };
        let gated_in = |n: &str| {
            calls.iter().any(|c| {
                c.func == n
                    && c.path.rsplit("::").next() == Some("unchanged_since")
                    && c.honoured != crate::resolve::Honoured::Discarded
            })
        };
        for func in &funcs {
            let reachable = reach(&func.name);
            let gated = reachable.iter().any(|n| gated_in(n));
            let reads_cache = func.body.contains("cache . entries")
                || cache_readers
                    .iter()
                    .any(|m| func.body.contains(&format!(". {m} (")));
            if reads_cache && !cache_impl.contains(&func.name) {
                replay_sites += 1;
                if !gated {
                    problems.push(format!(
                        "{rel}::{} reads stored build units and nothing it reaches honours \
                         `EventCoverage::unchanged_since`: replay is gated on trusted event \
                         coverage, and a discarded answer is no gate",
                        func.name
                    ));
                }
            }
            let surface: String = reachable
                .iter()
                .filter_map(|n| by_name.get(n.as_str()))
                .map(|x| format!("{} {} {}", x.name, x.sig, x.body))
                .collect::<Vec<_>>()
                .join(" ");
            let decides_reuse = surface.contains("reuse") || surface.contains("replay");
            let uses_stamp = surface.contains("mtime") || surface.contains("mod_time");
            if decides_reuse && uses_stamp && !gated {
                problems.push(format!(
                    "{rel}::{} decides reuse from a modification stamp with no coverage gate \
                     anywhere it reaches: a directory's own stamp does not move when a file inside \
                     a subdirectory changes",
                    func.name
                ));
            }
        }
    }
    if replay_sites == 0 {
        problems.push(
            "no governed function reads the container cache: section 18 requires unchanged \
             containers to be replayed (zero listings), gated on EventCoverage"
                .into(),
        );
    }
    finish(problems)
}

// ---------------------------------------------------------------------
// build_stores_join_by_capability
// ---------------------------------------------------------------------

const LOCATIONS_DIR: &str = "crates/core/src/locations";

/// The `BuildContainer` constructors that make a *shared* container:
/// the inherent functions of `BuildContainer` in the build-adapter model
/// whose body sets `shared: true`. Derived, so a new constructor for a
/// machine-wide store is governed the day it is written.
fn shared_container_ctors(root: &Path) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let Some(f) = load(root, &format!("{BUILD_ADAPTERS_DIR}/mod.rs")) else {
        return out;
    };
    for item in &f.ast.items {
        let syn::Item::Impl(imp) = item else { continue };
        if imp.trait_.is_some() || imp.self_ty.to_token_stream().to_string() != "BuildContainer" {
            continue;
        }
        for it in &imp.items {
            if let syn::ImplItem::Fn(m) = it
                && m.block
                    .to_token_stream()
                    .to_string()
                    .contains("shared : true")
            {
                out.insert(m.sig.ident.to_string());
            }
        }
    }
    out
}

/// Every detector id, read from each `impl Detector for T`'s `fn id`
/// under `crates/core/src/locations/` -- the literal it returns, or the
/// literal of the `const` it returns. Derived, so a detector added
/// tomorrow is a forbidden join key tomorrow.
fn detector_ids(root: &Path) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for rel in crate::resolve::rust_files_recursive(root, LOCATIONS_DIR) {
        let Some(f) = load(root, &rel) else { continue };
        let consts: HashMap<String, String> = f
            .ast
            .items
            .iter()
            .filter_map(|i| match i {
                syn::Item::Const(c) => {
                    let v = c.expr.to_token_stream().to_string();
                    (v.starts_with('"') && v.ends_with('"'))
                        .then(|| (c.ident.to_string(), v.trim_matches('"').to_string()))
                }
                _ => None,
            })
            .collect();
        for ty in ast::impls_of(&f.ast, "Detector") {
            if let Some(id) = adapter_id(&f.ast, &ty) {
                out.insert(id);
                continue;
            }
            // `fn id(&self) -> &'static str { GO_DETECTOR_ID }`
            for item in &f.ast.items {
                let syn::Item::Impl(i) = item else { continue };
                if i.self_ty.to_token_stream().to_string() != ty {
                    continue;
                }
                for it in &i.items {
                    if let syn::ImplItem::Fn(m) = it
                        && m.sig.ident == "id"
                    {
                        let body = m.block.to_token_stream().to_string();
                        for (name, value) in &consts {
                            if body
                                .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                                .any(|t| t == name)
                            {
                                out.insert(value.clone());
                            }
                        }
                    }
                }
            }
        }
    }
    out
}

/// String-literal comparisons in a body: `x == "lit"`, `"lit" != x`,
/// `.ends_with("lit")`/`.starts_with("lit")`/`.contains("lit")`/`.eq("lit")`,
/// and `match` arms whose pattern is a string literal. A join that
/// decides which adapter gets a store from a path shape or a name is
/// the heuristic the capability exists to replace.
fn literal_comparisons(file: &syn::File) -> Vec<(String, String)> {
    struct V {
        func: String,
        in_test: usize,
        out: Vec<(String, String)>,
    }
    fn is_str_lit(e: &syn::Expr) -> bool {
        match e {
            syn::Expr::Lit(l) => matches!(l.lit, syn::Lit::Str(_)),
            syn::Expr::Reference(r) => is_str_lit(&r.expr),
            syn::Expr::Paren(p) => is_str_lit(&p.expr),
            _ => false,
        }
    }
    fn pat_has_str(p: &syn::Pat) -> bool {
        match p {
            syn::Pat::Lit(l) => matches!(l.lit, syn::Lit::Str(_)),
            syn::Pat::Or(o) => o.cases.iter().any(pat_has_str),
            syn::Pat::Tuple(t) => t.elems.iter().any(pat_has_str),
            syn::Pat::TupleStruct(t) => t.elems.iter().any(pat_has_str),
            syn::Pat::Reference(r) => pat_has_str(&r.pat),
            syn::Pat::Paren(p) => pat_has_str(&p.pat),
            syn::Pat::Slice(s) => s.elems.iter().any(pat_has_str),
            _ => false,
        }
    }
    impl<'ast> Visit<'ast> for V {
        fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
            let test = is_cfg_test(&m.attrs);
            self.in_test += usize::from(test);
            syn::visit::visit_item_mod(self, m);
            self.in_test -= usize::from(test);
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
        fn visit_expr_binary(&mut self, b: &'ast syn::ExprBinary) {
            if self.in_test == 0
                && matches!(b.op, syn::BinOp::Eq(_) | syn::BinOp::Ne(_))
                && (is_str_lit(&b.left) || is_str_lit(&b.right))
            {
                self.out
                    .push((self.func.clone(), b.to_token_stream().to_string()));
            }
            syn::visit::visit_expr_binary(self, b);
        }
        fn visit_expr_method_call(&mut self, m: &'ast syn::ExprMethodCall) {
            let name = m.method.to_string();
            if self.in_test == 0
                && [
                    "ends_with",
                    "starts_with",
                    "contains",
                    "eq",
                    "ne",
                    "matches",
                ]
                .contains(&name.as_str())
                && m.args.iter().any(is_str_lit)
            {
                self.out
                    .push((self.func.clone(), m.to_token_stream().to_string()));
            }
            syn::visit::visit_expr_method_call(self, m);
        }
        fn visit_expr_match(&mut self, m: &'ast syn::ExprMatch) {
            if self.in_test == 0 && m.arms.iter().any(|a| pat_has_str(&a.pat)) {
                self.out.push((
                    self.func.clone(),
                    format!("match {} {{ \"..\" => .. }}", m.expr.to_token_stream()),
                ));
            }
            syn::visit::visit_expr_match(self, m);
        }
        fn visit_macro(&mut self, m: &'ast syn::Macro) {
            for e in macro_exprs(&m.tokens) {
                self.visit_expr(&e);
            }
        }
    }
    let mut v = V {
        func: String::new(),
        in_test: 0,
        out: Vec::new(),
    };
    v.visit_file(file);
    v.out
}

type SiteCache = Mutex<HashMap<(String, u64, String), Arc<Vec<Node>>>>;
static JOIN_SITES: OnceLock<SiteCache> = OnceLock::new();

/// The functions in one file that call a shared-container constructor,
/// excluding the constructors' own `impl` in the model file.
fn join_sites_in(
    rel: &str,
    f: &Parsed,
    ctors: &BTreeSet<String>,
    ctors_key: &str,
    is_model: bool,
) -> Arc<Vec<Node>> {
    let key = (rel.to_string(), text_hash(&f.text), ctors_key.to_string());
    let cache = JOIN_SITES.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(hit) = cache.lock().unwrap().get(&key).cloned() {
        return hit;
    }
    let ctor_fns: HashSet<String> = if is_model {
        f.ast
            .items
            .iter()
            .filter_map(|i| match i {
                syn::Item::Impl(imp)
                    if imp.self_ty.to_token_stream().to_string() == "BuildContainer" =>
                {
                    Some(imp.items.iter().filter_map(|it| match it {
                        syn::ImplItem::Fn(m) => Some(m.sig.ident.to_string()),
                        _ => None,
                    }))
                }
                _ => None,
            })
            .flatten()
            .collect()
    } else {
        HashSet::new()
    };
    let mut out: Vec<Node> = Vec::new();
    // Cheap pre-filter: a file that never writes a constructor's name
    // cannot call it.
    if ctors.iter().any(|c| f.text.contains(c.as_str())) {
        for c in crate::resolve::production_calls(&f.ast) {
            let last = c.path.rsplit("::").next().unwrap_or(&c.path);
            if !c.method && ctors.contains(last) && !ctor_fns.contains(&c.func) {
                let node = (rel.to_string(), c.func.clone());
                if !out.contains(&node) {
                    out.push(node);
                }
            }
        }
    }
    let out = Arc::new(out);
    let mut c = cache.lock().unwrap();
    if c.len() > 8192 {
        c.clear();
    }
    c.insert(key, Arc::clone(&out));
    out
}

type SeedSets = (
    Vec<(Node, String)>,
    Vec<(Node, String)>,
    Vec<(Node, String)>,
);
type SeedCache = Mutex<HashMap<(String, u64, String), Arc<SeedSets>>>;
static STORE_SEEDS: OnceLock<SeedCache> = OnceLock::new();

/// One file's contribution to the three seed sets of
/// [`build_stores_join_by_capability`].
fn store_join_seeds(rel: &str, f: &Parsed, ids: &BTreeSet<String>, ids_key: &str) -> Arc<SeedSets> {
    let key = (rel.to_string(), text_hash(&f.text), ids_key.to_string());
    let cache = STORE_SEEDS.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(hit) = cache.lock().unwrap().get(&key).cloned() {
        return hit;
    }
    let rel = rel.to_string();
    let mut id_seeds: Vec<(Node, String)> = Vec::new();
    let mut kinds_seeds: Vec<(Node, String)> = Vec::new();
    let mut stores_seeds: Vec<(Node, String)> = Vec::new();
    let aliases: Vec<(String, String)> = f
        .ast
        .items
        .iter()
        .filter_map(|i| match i {
            syn::Item::Const(c) => {
                let v = c.expr.to_token_stream().to_string();
                ids.iter()
                    .find(|id| v == format!("\"{id}\""))
                    .map(|id| (c.ident.to_string(), id.clone()))
            }
            _ => None,
        })
        .collect();
    for (func, path) in referenced_paths_by_fn(&rel, &f.ast) {
        if func.is_empty() {
            continue;
        }
        let last = path.rsplit("::").next().unwrap_or(&path);
        if last.ends_with("_DETECTOR_ID") {
            id_seeds.push((
                (rel.clone(), func.clone()),
                format!("{rel}::{func} names `{path}`"),
            ));
        }
        if let Some((name, id)) = aliases.iter().find(|(n, _)| n == last) {
            id_seeds.push((
                (rel.clone(), func.clone()),
                format!("{rel}::{func} names `{name}`, a const holding the detector id \"{id}\""),
            ));
        }
    }
    for func in ast::functions(&f.ast) {
        if let Some(id) = ids
            .iter()
            .find(|id| func.body.contains(&format!("\"{id}\"")))
        {
            id_seeds.push((
                (rel.clone(), func.name.clone()),
                format!(
                    "{rel}::{} holds the detector id literal \"{id}\"",
                    func.name
                ),
            ));
        }
    }
    for c in crate::resolve::production_calls(&f.ast) {
        if c.honoured == crate::resolve::Honoured::Discarded {
            continue;
        }
        let last = c.path.rsplit("::").next().unwrap_or(&c.path);
        let node = (rel.clone(), c.func.clone());
        if last == "store_kinds" {
            kinds_seeds.push((node.clone(), "asks the adapter's store_kinds".into()));
        }
        if last == "build_stores" {
            stores_seeds.push((node, "asks the detector's build_stores".into()));
        }
    }
    let out = Arc::new((id_seeds, kinds_seeds, stores_seeds));
    let mut c = cache.lock().unwrap();
    if c.len() > 8192 {
        c.clear();
    }
    c.insert(key, Arc::clone(&out));
    out
}

/// Statement: a machine-wide store reaches a build adapter only through
/// two declared capabilities -- the detector's `build_stores()` (which
/// of its locations is which kind of store) and the adapter's
/// `store_kinds()` (which kinds it identifies) -- never through a
/// detector id, an adapter id, or a path shape.
///
/// Derived: the *join sites* are every production function in the
/// workspace that calls a shared-container constructor of
/// `BuildContainer` (the inherent constructors whose body sets
/// `shared: true`), other than those constructors themselves. For each
/// join site, over everything it reaches through the whole-workspace
/// call graph:
///
/// 1. nothing names a `*_DETECTOR_ID` path (resolved, so a `use .. as`
///    rename counts) or a detector-id literal, or a `const` holding one
///    -- the detector ids are read from every `impl Detector`'s `fn id`;
/// 2. an honoured `store_kinds` call and an honoured `build_stores` call
///    are reached -- a join that never asks, or asks and throws the
///    answer away (`let _ = a.store_kinds();`), matched on something
///    else;
/// 3. the join site itself and its same-file helpers compare no string
///    literal (`== "npm"`, `.ends_with("caches")`, a `"..." =>` arm):
///    choosing an adapter by a path suffix is the heuristic the
///    capability replaces, and a custom `GOMODCACHE` defeats it.
pub fn build_stores_join_by_capability(root: &Path) -> Result<(), String> {
    let _ = derived_or_err(root)?;
    let ctors = shared_container_ctors(root);
    if ctors.is_empty() {
        return Err(format!(
            "{BUILD_ADAPTERS_DIR}/mod.rs defines no `BuildContainer` constructor that sets \
             `shared: true`; the rule cannot say how a machine-wide store is handed to an adapter"
        ));
    }
    let ids = detector_ids(root);
    let files = crate::resolve::workspace_files(root);
    let model = format!("{BUILD_ADAPTERS_DIR}/mod.rs");

    // Join sites.
    let ctors_key = ctors.iter().cloned().collect::<Vec<_>>().join("\u{1}");
    let mut sites: Vec<Node> = Vec::new();
    for rel in &files {
        let Some(f) = load(root, rel) else { continue };
        for node in join_sites_in(rel, &f, &ctors, &ctors_key, *rel == model).iter() {
            if !sites.contains(node) {
                sites.push(node.clone());
            }
        }
    }
    if sites.is_empty() {
        return Err(
            "no production function hands a machine-wide store to a build adapter: the \
             adapters identify npm/pnpm stores, Gradle homes, Maven repositories, Go and Python \
             caches, DerivedData and the Android SDK, and none of it reaches a live report \
             until the external observation joins its measured stores to them by capability"
                .into(),
        );
    }

    // Seeds: functions that name a detector id, directly or through a
    // const alias; and functions that honour each capability question.
    // Per-file and cached by exact text: the corpus runs this rule once
    // per fixture over an otherwise unchanged workspace.
    let ids_key = ids.iter().cloned().collect::<Vec<_>>().join("\u{1}");
    let mut id_seeds: Vec<(Node, String)> = Vec::new();
    let mut kinds_seeds: Vec<(Node, String)> = Vec::new();
    let mut stores_seeds: Vec<(Node, String)> = Vec::new();
    for rel in &files {
        let Some(f) = load(root, rel) else { continue };
        let seeds = store_join_seeds(rel, &f, &ids, &ids_key);
        id_seeds.extend(seeds.0.iter().cloned());
        kinds_seeds.extend(seeds.1.iter().cloned());
        stores_seeds.extend(seeds.2.iter().cloned());
    }
    let mut graphs =
        Reachability::build_many(root, &[&id_seeds, &kinds_seeds, &stores_seeds]).into_iter();
    let (Some(by_id), Some(asks_kinds), Some(asks_stores)) =
        (graphs.next(), graphs.next(), graphs.next())
    else {
        return Err("internal: three seed sets, three graphs".into());
    };

    let mut problems = Vec::new();
    for (rel, func) in &sites {
        if let Some(why) = by_id.why(rel, func) {
            problems.push(format!(
                "{rel}::{func} hands a store to a build adapter and reaches a detector id ({why}): \
                 the join goes through the detector's declared `build_stores()` and the adapter's \
                 `store_kinds()`, never an id"
            ));
        }
        if asks_kinds.why(rel, func).is_none() {
            problems.push(format!(
                "{rel}::{func} hands a store to a build adapter without an honoured \
                 `store_kinds()` call anywhere it reaches: which adapter identifies a store is \
                 the adapter's declared capability, not the join's choice"
            ));
        }
        if asks_stores.why(rel, func).is_none() {
            problems.push(format!(
                "{rel}::{func} hands a store to a build adapter without an honoured \
                 `build_stores()` call anywhere it reaches: which location is which kind of store \
                 is the detector's declaration, not a guess from its path"
            ));
        }
    }
    // (3) literal comparisons in the join sites and their same-file
    // helpers.
    for rel in sites
        .iter()
        .map(|(r, _)| r.clone())
        .collect::<BTreeSet<_>>()
    {
        let f = load_or_err(root, &rel)?;
        let calls = crate::resolve::production_calls(&f.ast);
        let local: HashSet<String> = ast::functions(&f.ast).into_iter().map(|x| x.name).collect();
        let mut reach: HashSet<String> = HashSet::new();
        let mut stack: Vec<String> = sites
            .iter()
            .filter(|(r, _)| *r == rel)
            .map(|(_, n)| n.clone())
            .collect();
        while let Some(n) = stack.pop() {
            if !reach.insert(n.clone()) {
                continue;
            }
            for c in calls.iter().filter(|c| c.func == n) {
                let callee = c.path.rsplit("::").next().unwrap_or(&c.path).to_string();
                if local.contains(&callee) {
                    stack.push(callee);
                }
            }
        }
        for (func, text) in literal_comparisons(&f.ast) {
            if reach.contains(&func) {
                problems.push(format!(
                    "{rel}::{func} compares a string literal (`{text}`) on the way to handing a \
                     store to an adapter: a path shape or a name is exactly what a custom \
                     GOMODCACHE, GRADLE_USER_HOME or maven.repo.local defeats"
                ));
            }
        }
        problems.extend(unknown_macro_problems(&rel, &f.ast));
    }
    finish(problems)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(files: &[(&str, &str)]) -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        for (rel, body) in files {
            let p = tmp.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        }
        tmp
    }

    #[test]
    fn an_absent_adapter_set_is_the_first_failure() {
        let tmp = tempfile::tempdir().unwrap();
        let err = build_adapters_are_pluggable(tmp.path()).unwrap_err();
        assert!(err.contains("no `impl BuildAdapter`"), "{err}");
    }

    #[test]
    fn the_adapter_set_is_derived_from_the_trait_impl_anywhere() {
        let tmp = tree(&[
            (
                "crates/core/src/build_adapters/anything.rs",
                "pub struct A;\nimpl BuildAdapter for A { fn id(&self) -> &'static str { \
                 \"anything\" } }",
            ),
            (
                "crates/core/src/build_adapters/helper.rs",
                "pub fn shared() -> u8 { 1 }",
            ),
            (
                "crates/core/src/elsewhere.rs",
                "pub struct B;\nimpl BuildAdapter for B { fn id(&self) -> &'static str { \"b\" } }",
            ),
        ]);
        let d = derive(tmp.path());
        let ids: Vec<Option<String>> = d.adapters.iter().map(|a| a.id.clone()).collect();
        assert_eq!(ids, vec![Some("anything".into()), Some("b".into())]);
        assert!(
            d.governed.iter().any(|g| g.ends_with("elsewhere.rs")),
            "an adapter outside build_adapters/ is governed"
        );
        assert!(
            d.governed.iter().any(|g| g.ends_with("helper.rs")),
            "a helper is not an adapter and is still governed"
        );
    }

    #[test]
    fn no_module_is_exempt_by_name() {
        let tmp = tree(&[
            (
                "crates/core/src/build_adapters/a.rs",
                "pub struct A;\nimpl BuildAdapter for A { fn id(&self) -> &'static str { \"a\" } }",
            ),
            ("crates/core/src/build_adapters/mod.rs", "pub fn m() {}"),
            (
                "crates/core/src/build_adapters/registry.rs",
                "pub fn r() {}",
            ),
            ("crates/core/src/build_adapters/matrix.rs", "pub fn x() {}"),
            (
                "crates/core/src/build_adapters/bounded_io.rs",
                "pub fn b() {}",
            ),
        ]);
        assert_eq!(derive(tmp.path()).governed.len(), 5);
    }

    #[test]
    fn a_bounded_primitive_that_lost_its_cap_is_rejected() {
        let tmp = tree(&[
            (
                "crates/core/src/build_adapters/bounded_io.rs",
                "pub const MAX_MANIFEST_BYTES: usize = 1;\npub fn read_manifest(p: \
                 &std::path::Path, cap: usize) -> Option<String> { let _ = (p, cap); None }\npub \
                 fn read_whole_manifest() {}",
            ),
            (
                "crates/core/src/locations/mod.rs",
                "pub const SHALLOW_LIST_CAP: usize = 8;\npub fn shallow_list() { let _ = \
                 SHALLOW_LIST_CAP; }\npub fn shallow_dir_names() { let _ = SHALLOW_LIST_CAP; }",
            ),
        ]);
        let err = bounded_primitives_still_bounded(tmp.path()).unwrap_err();
        assert!(err.contains("read_manifest"), "{err}");
    }

    #[test]
    fn a_namespace_target_matches_any_member() {
        let call = |path: &str| Call {
            func: "f".into(),
            path: path.into(),
            written: path.into(),
            method: false,
        };
        assert!(target_matches(
            Target::Namespace("Command"),
            &call("std::process::Command::new")
        ));
        assert!(target_matches(
            Target::Namespace("actions"),
            &call("crate::actions::propose_for_path")
        ));
        assert!(!target_matches(
            Target::Namespace("actions"),
            &call("crate::build_adapters::actions_label")
        ));
        assert!(!target_matches(Target::Namespace("walk"), &call("walk")));
    }

    #[test]
    fn a_field_write_is_seen_in_every_form() {
        let fields: HashSet<String> = ["action", "coverage", "supported", "bytes", "limits"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let f: syn::File = syn::parse_str(
            "fn a(u: &mut U) { u.coverage.supported = true; }
             fn b(u: &mut U) { u.bytes += 1; }
             fn c(u: &mut U) { u.coverage.limits.clear(); }
             fn d(u: &mut U) { std::mem::take(&mut u.action); }
             fn e(u: &U) -> bool { u.coverage.limits.iter().any(|l| l.is_empty()) }
             fn g(u: &mut U) { let _ = vec![u.coverage.limits.pop()]; }
             impl NestedUnitBuilder { fn ok(mut self) -> Self { self.unit.bytes = 1; self } }",
        )
        .unwrap();
        let writes: Vec<String> = field_writes(&f, &fields)
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        assert_eq!(
            writes,
            vec!["a", "b", "c", "d", "g"],
            "e is a read; the builder's own impl is exempt; g is inside a macro"
        );
    }
}
