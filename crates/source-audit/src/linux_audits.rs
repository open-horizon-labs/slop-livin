//! Audits for the Linux track's part 2 (#85 Trash, #86 occupancy),
//! written in the derived-set form `review/REVIEW-STACK-3.md` §1 asks
//! for and on the same workspace model as `platform_audits`: every
//! definition parsed once, call edges resolved to definitions.
//!
//! * `trash_backend_owns_every_move` -- the destructive sinks are
//!   *derived* (every production definition that consults occupancy,
//!   i.e. calls something returning `OccupancyState`), and nothing they
//!   reach may move existing data with `rename` except the Trash backend
//!   module, which is found by module path. The backend itself may not
//!   copy, recursively delete, or delete a source it moves, and may not
//!   discard a move's result.
//! * `occupancy_gaps_are_unknown_never_free` -- the probes are *derived*
//!   (every definition returning `OccupancyState`, plus what they call
//!   exactly), and inside them a failed read of process or filesystem
//!   state may not be turned into "nothing there": no error-discarding
//!   adapter on a read's result, no `if let Ok` without an `else`, no
//!   `let Ok(..) = .. else` or `Err` arm whose outcome is absence unless
//!   it is guarded by a "the thing is gone" test.
//!
//! What they cannot see is in the guardrails' Limits sections, with the
//! runtime tests that cover it.

use crate::platform_audits::{Graph, Node, Workspace, call_graph, closure, last_segment, show};
use crate::resolve::{self, Honoured};
use quote::ToTokens;
use std::collections::BTreeSet;
use std::path::Path;
use syn::visit::Visit;

// ---------------------------------------------------------------------
// #85: every move into a Trash goes through the backend
// ---------------------------------------------------------------------

/// Resolved paths that move a directory entry. Short, and not the whole
/// rule: the backend-side half below is inverted over `std::fs`.
const MOVERS: &[&str] = &[
    "std::fs::rename",
    "libc::rename",
    "libc::renameat",
    "libc::renameat2",
];

/// Calls that create a file a function may then publish with a rename
/// (write-then-rename is how every control file here is written
/// atomically). Only the *first argument* of one of these, in the same
/// function and earlier, makes a later rename a publish rather than a
/// move of existing data.
const CREATORS: &[&str] = &[
    "std::fs::write",
    "std::fs::File::create",
    "std::fs::File::create_new",
    "File::create",
];

/// `std::fs` calls the backend may make. **Everything else under
/// `std::fs::` is refused there** -- `copy`, `remove_dir_all`,
/// `remove_dir`, `hard_link`, `write` -- so a fallback that duplicates
/// or destroys fails closed without anyone listing it.
const BACKEND_FS_ALLOWED: &[&str] = &[
    "read",
    "read_to_string",
    "read_dir",
    "read_link",
    "metadata",
    "symlink_metadata",
    "canonicalize",
    "exists",
    "try_exists",
    "create_dir",
    "create_dir_all",
    "set_permissions",
    "rename",
    "remove_file",
    "Permissions::from_mode",
    "OpenOptions::new",
];

fn is_mover(path: &str) -> bool {
    MOVERS
        .iter()
        .any(|m| resolve::path_ends_with(path, m) && path.ends_with(last_segment(m)))
        && (path.starts_with("std::fs::") || path.starts_with("libc::") || path.starts_with("fs::"))
}

fn is_creator(path: &str) -> bool {
    CREATORS.iter().any(|c| resolve::path_ends_with(path, c))
}

pub fn trash_backend_owns_every_move(root: &Path) -> Result<(), String> {
    let ws = Workspace::load(root);

    let backend: BTreeSet<String> = ws
        .files
        .iter()
        .filter(|f| f.mods.len() >= 2 && f.mods[0] == "platform" && f.mods[1] == "trash")
        .map(|f| f.rel.clone())
        .collect();
    if backend.is_empty() {
        return Err(
            "no `platform::trash` module under crates/*/src: there is no Trash backend for the \
             destructive sinks to go through"
                .into(),
        );
    }

    // The occupancy gate: every definition returning `OccupancyState`.
    let gate: BTreeSet<Node> = ws
        .files
        .iter()
        .flat_map(|f| {
            f.funcs
                .iter()
                .filter(|func| returns(&func.sig, "OccupancyState"))
                .map(|func| (f.rel.clone(), func.name.clone()))
        })
        .collect();
    if gate.is_empty() {
        return Err(
            "no definition returns `OccupancyState`: nothing can gate a destructive \
                    action on whether its unit is in use"
                .into(),
        );
    }
    // The sinks: production definitions that consult the gate and are
    // not themselves part of it.
    let mut sinks: BTreeSet<Node> = BTreeSet::new();
    for (rel, c) in &ws.calls {
        if c.func.is_empty() {
            continue;
        }
        let from = (rel.clone(), c.func.clone());
        if gate.contains(&from) {
            continue;
        }
        if ws.targets(rel, c).iter().any(|t| gate.contains(t)) {
            sinks.insert(from);
        }
    }
    if sinks.is_empty() {
        return Err(
            "no production code consults occupancy before acting: the destructive \
                    sinks this rule guards could not be derived"
                .into(),
        );
    }

    let walk: Graph = call_graph(&ws, false);
    let reach = closure(&sinks, &walk);

    // Rule 1: outside the backend, a rename reachable from a sink is
    // only a publish of a file the same function just wrote.
    for (rel, c) in &ws.calls {
        if c.method || !is_mover(&c.path) || backend.contains(rel) {
            continue;
        }
        let node = (rel.clone(), c.func.clone());
        if !reach.contains(&node) {
            continue;
        }
        let source = c
            .args
            .first()
            .map(|a| resolve::root_ident(a))
            .unwrap_or_default();
        let published = ws.calls.iter().any(|(r2, w)| {
            r2 == rel
                && w.func == c.func
                && w.stmt < c.stmt
                && !w.method
                && is_creator(&w.path)
                && w.args.first().map(|a| resolve::root_ident(a)).as_deref() == Some(&source)
        });
        if !published {
            return Err(format!(
                "{}: `{}` moves `{source}` outside the Trash backend, on a path a destructive \
                 action reaches. Every move into a Trash goes through `platform::trash` \
                 (`move_item`/`Envelope`), which is where \"a rename or nothing, never a copy, \
                 never a permanent fallback\" and the restore record are kept.",
                show(&node),
                c.written
            ));
        }
    }

    // Rules 2-4: inside the backend.
    let backend_movers: BTreeSet<Node> = {
        // Definitions in the backend that move (directly, or through
        // another backend definition that does).
        let mut m: BTreeSet<Node> = ws
            .calls
            .iter()
            .filter(|(rel, c)| backend.contains(rel) && !c.method && is_mover(&c.path))
            .map(|(rel, c)| (rel.clone(), c.func.clone()))
            .collect();
        loop {
            let before = m.len();
            for (rel, c) in &ws.calls {
                if !backend.contains(rel) || c.func.is_empty() {
                    continue;
                }
                if ws.targets(rel, c).iter().any(|t| m.contains(t)) {
                    m.insert((rel.clone(), c.func.clone()));
                }
            }
            if m.len() == before {
                break;
            }
        }
        m
    };
    for (rel, c) in &ws.calls {
        if !backend.contains(rel) || c.in_test {
            continue;
        }
        let node = (rel.clone(), c.func.clone());
        if !c.method
            && let Some(rest) = c.path.strip_prefix("std::fs::")
            && !BACKEND_FS_ALLOWED
                .iter()
                .any(|a| resolve::path_ends_with(rest, a))
        {
            return Err(format!(
                "{}: the Trash backend calls `{}`. It may only rename: a copy, a recursive \
                 delete or a write of an item's bytes is how a Trash fallback loses or \
                 duplicates data (the `trash` crate's copy + remove_dir_all on EXDEV is the \
                 example this rule was written against).",
                show(&node),
                c.written
            ));
        }
        if resolve::path_ends_with(&c.path, "std::io::copy") {
            return Err(format!(
                "{}: the Trash backend copies bytes (`{}`); it may only rename.",
                show(&node),
                c.written
            ));
        }
        let moves = (!c.method && is_mover(&c.path))
            || ws
                .targets(rel, c)
                .iter()
                .any(|t| backend_movers.contains(t) && *t != node);
        if moves && c.honoured == Honoured::Discarded {
            return Err(format!(
                "{}: discards the result of `{}`. A move whose failure is ignored reports an \
                 item as trashed that is still at its source -- or, worse, one that is in \
                 neither place.",
                show(&node),
                c.written
            ));
        }
    }
    // Rule 3: a function that moves `x` must not also delete `x`.
    for (rel, c) in &ws.calls {
        if !backend.contains(rel) || c.in_test || c.method {
            continue;
        }
        let moves = is_mover(&c.path)
            || ws
                .targets(rel, c)
                .iter()
                .any(|t| backend_movers.contains(t));
        if !moves {
            continue;
        }
        let Some(source) = c.args.first().map(|a| resolve::root_ident(a)) else {
            continue;
        };
        if let Some((_, d)) = ws.calls.iter().find(|(r2, d)| {
            r2 == rel
                && d.func == c.func
                && !d.method
                && d.path.starts_with("std::fs::remove")
                && d.args.first().map(|a| resolve::root_ident(a)).as_deref() == Some(&source)
        }) {
            return Err(format!(
                "{}::{}: removes `{source}` (`{}`), the same path it moves into the Trash. A \
                 failed move must leave the source where it was; deleting it is the permanent \
                 fallback the backend never has.",
                rel, c.func, d.written
            ));
        }
    }
    Ok(())
}

fn returns(sig: &str, ty: &str) -> bool {
    sig.split_once("->").is_some_and(|(_, ret)| {
        ret.split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .any(|t| t == ty)
    })
}

// ---------------------------------------------------------------------
// #86: a gap in what the probe can see is Unknown, never Free
// ---------------------------------------------------------------------

/// `std::fs` reads. A method of the same name on a path (`p.read_dir()`)
/// is a read too.
const FS_READS: &[&str] = &[
    "read",
    "read_to_string",
    "read_dir",
    "read_link",
    "metadata",
    "symlink_metadata",
    "canonicalize",
    "File::open",
];

/// The `Result`/`Iterator` methods that turn an error into nothing:
/// the part of the standard library's API that discards the `Err`. This
/// is std's surface, not a list of this project's habits, so it does not
/// rot as the code changes.
const DISCARDING: &[&str] = &[
    "ok",
    "err",
    "flatten",
    "unwrap_or",
    "unwrap_or_default",
    "unwrap_or_else",
    "map_or",
    "map_or_else",
    "is_ok",
    "is_ok_and",
    "is_err",
    "is_err_and",
    "filter_map",
    "flat_map",
    "into_iter",
    "iter",
];

pub fn occupancy_gaps_are_unknown_never_free(root: &Path) -> Result<(), String> {
    let ws = Workspace::load(root);
    let probes: BTreeSet<Node> = ws
        .files
        .iter()
        .filter(|f| f.rel.starts_with("crates/core/"))
        .flat_map(|f| {
            f.funcs
                .iter()
                .filter(|func| returns(&func.sig, "OccupancyState"))
                .map(|func| (f.rel.clone(), func.name.clone()))
        })
        .collect();
    if probes.is_empty() {
        return Err(
            "no definition in crates/core returns `OccupancyState`: there is no \
                    occupancy probe for this rule to hold"
                .into(),
        );
    }
    // What the probes call, exactly: a helper one call away that reads
    // /proc is part of the probe.
    let exact: Graph = call_graph(&ws, true);
    let set = closure(&probes, &exact);

    // Fallible sources: std reads, plus any definition in the set that
    // returns a `Result` (its `Err` is a read that failed further down).
    let fallible_helpers: BTreeSet<String> = ws
        .files
        .iter()
        .flat_map(|f| {
            f.funcs
                .iter()
                .filter(|func| {
                    set.contains(&(f.rel.clone(), func.name.clone()))
                        && returns(&func.sig, "Result")
                })
                .map(|func| func.name.clone())
        })
        .collect();

    for file in &ws.files {
        let names: BTreeSet<String> = set
            .iter()
            .filter(|(r, _)| *r == file.rel)
            .map(|(_, n)| n.clone())
            .collect();
        if names.is_empty() {
            continue;
        }
        let Some(parsed) = resolve::maybe(root, &file.rel) else {
            continue;
        };
        let res = resolve::resolver(&parsed.ast);
        let mut v = GapVisitor {
            res: &res,
            names: &names,
            helpers: &fallible_helpers,
            rel: &file.rel,
            func: None,
            tainted: BTreeSet::new(),
            in_test: 0,
            problem: None,
        };
        v.visit_file(&parsed.ast);
        if let Some(p) = v.problem {
            return Err(p);
        }
    }
    Ok(())
}

struct GapVisitor<'a> {
    res: &'a resolve::Resolver,
    names: &'a BTreeSet<String>,
    helpers: &'a BTreeSet<String>,
    rel: &'a str,
    func: Option<String>,
    tainted: BTreeSet<String>,
    in_test: usize,
    problem: Option<String>,
}

impl GapVisitor<'_> {
    fn active(&self) -> bool {
        self.in_test == 0 && self.func.is_some() && self.problem.is_none()
    }

    fn fail(&mut self, what: String) {
        if self.problem.is_none() {
            self.problem = Some(format!(
                "{}::{}: {what}. In an occupancy probe a read that fails is a question that \
                 went unanswered, which is `OccupancyState::Unknown` -- never \"nothing holds \
                 it\" (`.oh/guardrails/occupancy-gaps-are-unknown-never-free.md`).",
                self.rel,
                self.func.as_deref().unwrap_or("")
            ));
        }
    }

    /// Whether `e` reads process or filesystem state that can fail, or
    /// is derived from such a read.
    fn fallible(&self, e: &syn::Expr) -> bool {
        struct F<'b> {
            v: &'b GapVisitor<'b>,
            hit: bool,
        }
        impl<'ast> Visit<'ast> for F<'_> {
            fn visit_expr_call(&mut self, c: &'ast syn::ExprCall) {
                if let syn::Expr::Path(p) = &*c.func {
                    let resolved = self.v.res.resolve(&p.path.to_token_stream().to_string());
                    let is_read = resolved.starts_with("std::fs::")
                        && FS_READS
                            .iter()
                            .any(|r| resolve::path_ends_with(&resolved, r))
                        || resolve::path_ends_with(&resolved, "File::open");
                    if is_read || self.v.helpers.contains(last_segment(&resolved)) {
                        self.hit = true;
                    }
                }
                syn::visit::visit_expr_call(self, c);
            }
            fn visit_expr_method_call(&mut self, m: &'ast syn::ExprMethodCall) {
                let name = m.method.to_string();
                if [
                    "read_dir",
                    "read_link",
                    "metadata",
                    "symlink_metadata",
                    "canonicalize",
                    "read_to_string",
                    "read_to_end",
                ]
                .contains(&name.as_str())
                {
                    self.hit = true;
                }
                syn::visit::visit_expr_method_call(self, m);
            }
            fn visit_expr_path(&mut self, p: &'ast syn::ExprPath) {
                if let Some(id) = p.path.get_ident()
                    && self.v.tainted.contains(&id.to_string())
                {
                    self.hit = true;
                }
            }
        }
        let mut f = F {
            v: self,
            hit: false,
        };
        f.visit_expr(e);
        f.hit
    }

    fn taint_pat(&mut self, pat: &syn::Pat) {
        struct N(Vec<String>);
        impl<'ast> Visit<'ast> for N {
            fn visit_pat_ident(&mut self, i: &'ast syn::PatIdent) {
                self.0.push(i.ident.to_string());
                syn::visit::visit_pat_ident(self, i);
            }
        }
        let mut n = N(Vec::new());
        n.visit_pat(pat);
        self.tainted.extend(n.0);
    }
}

/// An expression whose value is "nothing there": the outcomes a failed
/// read must not quietly become.
fn absent(e: &syn::Expr) -> bool {
    match e {
        syn::Expr::Continue(_) | syn::Expr::Break(_) => true,
        syn::Expr::Tuple(t) => t.elems.is_empty(),
        syn::Expr::Block(b) => match b.block.stmts.as_slice() {
            [] => true,
            [syn::Stmt::Expr(inner, _)] => absent(inner),
            _ => false,
        },
        syn::Expr::Return(r) => r.expr.as_deref().is_none_or(absent),
        syn::Expr::Lit(l) => matches!(&l.lit, syn::Lit::Bool(b) if !b.value),
        syn::Expr::Path(p) => p
            .path
            .segments
            .last()
            .is_some_and(|s| s.ident == "Free" || s.ident == "None"),
        syn::Expr::Call(c) => {
            let callee = c.func.to_token_stream().to_string().replace(' ', "");
            (callee == "Ok" || callee == "Some") && c.args.len() == 1 && c.args.iter().all(absent)
                || callee.ends_with("default") && c.args.is_empty()
                || callee.ends_with("Vec::new") && c.args.is_empty()
        }
        _ => false,
    }
}

/// A match-arm guard that tests "the thing is gone" (a process exited,
/// an entry vanished): it names `NotFound`/`ENOENT`/`ESRCH`, directly or
/// in the one function it calls.
fn gone_guard(guard: &syn::Expr, file_text: &str) -> bool {
    let t = guard.to_token_stream().to_string();
    let names_gone =
        |s: &str| s.contains("NotFound") || s.contains("ENOENT") || s.contains("ESRCH");
    if names_gone(&t) {
        return true;
    }
    // `gone(&e)`: read that function's own body in the same file.
    if let syn::Expr::Call(c) = guard
        && let syn::Expr::Path(p) = &*c.func
        && let Some(id) = p.path.get_ident()
    {
        let needle = format!("fn {id}");
        if let Some(at) = file_text.find(&needle) {
            let body = &file_text[at..(at + 400).min(file_text.len())];
            return names_gone(body);
        }
    }
    false
}

thread_local! {
    static FILE_TEXT: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

impl<'ast> Visit<'ast> for GapVisitor<'_> {
    fn visit_file(&mut self, f: &'ast syn::File) {
        FILE_TEXT.with(|t| *t.borrow_mut() = f.to_token_stream().to_string());
        syn::visit::visit_file(self, f);
    }

    fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
        let test = m
            .attrs
            .iter()
            .any(|a| a.path().is_ident("cfg") && a.to_token_stream().to_string().contains("test"));
        if test {
            self.in_test += 1;
        }
        syn::visit::visit_item_mod(self, m);
        if test {
            self.in_test -= 1;
        }
    }

    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        let name = f.sig.ident.to_string();
        if self.names.contains(&name) {
            let prev = self.func.replace(name);
            let saved = std::mem::take(&mut self.tainted);
            syn::visit::visit_item_fn(self, f);
            self.tainted = saved;
            self.func = prev;
        } else {
            syn::visit::visit_item_fn(self, f);
        }
    }

    fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
        let name = f.sig.ident.to_string();
        if self.names.contains(&name) {
            let prev = self.func.replace(name);
            let saved = std::mem::take(&mut self.tainted);
            syn::visit::visit_impl_item_fn(self, f);
            self.tainted = saved;
            self.func = prev;
        } else {
            syn::visit::visit_impl_item_fn(self, f);
        }
    }

    fn visit_local(&mut self, l: &'ast syn::Local) {
        if self.active()
            && let Some(init) = &l.init
        {
            let fallible = self.fallible(&init.expr);
            if fallible {
                if matches!(&l.pat, syn::Pat::Wild(_))
                    || matches!(&l.pat, syn::Pat::Ident(i) if i.ident.to_string().starts_with('_'))
                {
                    self.fail("discards the result of a read (`let _ = ..`)".into());
                }
                let pat = l.pat.to_token_stream().to_string().replace(' ', "");
                if let Some((_, diverge)) = &init.diverge
                    && pat.starts_with("Ok(")
                    && absent(diverge)
                {
                    self.fail(format!(
                        "`let {pat} = .. else {{ {} }}` turns a failed read into absence",
                        diverge.to_token_stream()
                    ));
                }
                self.taint_pat(&l.pat);
            }
        }
        syn::visit::visit_local(self, l);
    }

    fn visit_expr_for_loop(&mut self, f: &'ast syn::ExprForLoop) {
        if self.active() && self.fallible(&f.expr) {
            self.taint_pat(&f.pat);
        }
        syn::visit::visit_expr_for_loop(self, f);
    }

    fn visit_expr_method_call(&mut self, m: &'ast syn::ExprMethodCall) {
        if self.active() {
            let name = m.method.to_string();
            if DISCARDING.contains(&name.as_str()) && self.fallible(&m.receiver) {
                // `into_iter`/`iter` only discard when what follows
                // flattens; everything else in the list discards itself.
                let benign = matches!(name.as_str(), "into_iter" | "iter");
                if !benign {
                    self.fail(format!(
                        "`.{name}()` on the result of a read discards its error"
                    ));
                }
            }
        }
        syn::visit::visit_expr_method_call(self, m);
    }

    fn visit_expr_if(&mut self, i: &'ast syn::ExprIf) {
        if self.active()
            && let syn::Expr::Let(l) = &*i.cond
            && self.fallible(&l.expr)
        {
            let pat = l.pat.to_token_stream().to_string().replace(' ', "");
            if pat.starts_with("Ok(") && i.else_branch.is_none() {
                self.fail(format!(
                    "`if let {pat} = ..` has no `else`: the read's failure is skipped silently"
                ));
            }
            self.taint_pat(&l.pat);
        }
        syn::visit::visit_expr_if(self, i);
    }

    fn visit_expr_match(&mut self, m: &'ast syn::ExprMatch) {
        if self.active() && self.fallible(&m.expr) {
            let text = FILE_TEXT.with(|t| t.borrow().clone());
            for arm in &m.arms {
                let pat = arm.pat.to_token_stream().to_string().replace(' ', "");
                let is_err = pat.starts_with("Err(") || pat == "_";
                if !is_err {
                    self.taint_pat(&arm.pat);
                    continue;
                }
                let guarded = arm
                    .guard
                    .as_ref()
                    .is_some_and(|(_, g)| gone_guard(g, &text));
                if !guarded && absent(&arm.body) {
                    self.fail(format!(
                        "the `{pat}` arm of a match on a read answers `{}`, as if the thing \
                         read were not there; only a \"gone\" guard (NotFound/ESRCH) may",
                        arm.body.to_token_stream()
                    ));
                }
            }
        }
        syn::visit::visit_expr_match(self, m);
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        if self.active() {
            // Token trees, not text: a read named inside a string
            // literal ("cannot be read ({e})") is a message, not a call.
            fn calls_a_read(ts: proc_macro2::TokenStream) -> bool {
                let toks: Vec<proc_macro2::TokenTree> = ts.into_iter().collect();
                toks.iter().enumerate().any(|(i, t)| match t {
                    proc_macro2::TokenTree::Ident(id) => {
                        let name = id.to_string();
                        FS_READS.iter().any(|r| last_segment(r) == name)
                            && matches!(
                                toks.get(i + 1),
                                Some(proc_macro2::TokenTree::Group(g))
                                    if g.delimiter() == proc_macro2::Delimiter::Parenthesis
                            )
                    }
                    proc_macro2::TokenTree::Group(g) => calls_a_read(g.stream()),
                    _ => false,
                })
            }
            let hidden = calls_a_read(mac.tokens.clone());
            if hidden {
                self.fail(format!(
                    "a read inside `{}!` cannot be seen through; read before the macro and \
                     handle the error",
                    mac.path.to_token_stream()
                ));
            }
        }
        syn::visit::visit_macro(self, mac);
    }
}
