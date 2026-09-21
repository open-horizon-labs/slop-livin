//! Conservative name-based call graph, not Rust type resolution. Follow local
//! helper calls, including ordinary closures; only a literal thread::spawn
//! closure is a background boundary. Keep the sink list explicit and reviewed.
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
use syn::visit::Visit;

const ROOTS: &[&str] = &[
    "event_loop",
    "handle_terminal_key",
    "handle_key",
    "handle_key_mod",
    "draw",
];
const SINKS: &[&str] = &[
    "mark_row",
    "mark_project",
    "mark_selected",
    "mark_all_in_view",
    "delete_here",
    "execute_plan",
    "execute_plan_progress",
    "execute_one",
    "propose",
    "free_space_bytes",
    "sleep",
    "recv",
    "recv_timeout",
    "blocking_join",
    "wait",
    "wait_with_output",
];

#[derive(Default)]
struct Calls {
    edges: BTreeSet<String>,
}

fn path_name(path: &syn::Path) -> String {
    path.segments
        .iter()
        .map(|s| s.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
}

impl<'ast> Visit<'ast> for Calls {
    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(p) = &*call.func {
            let path = path_name(&p.path);
            if path == "std::thread::spawn" {
                // Argument expressions still execute on the calling thread.
                // Only an inline closure body is skipped, not spawn(make_job()).
                for arg in &call.args {
                    if !matches!(arg, syn::Expr::Closure(_)) {
                        self.visit_expr(arg);
                    }
                }
                return;
            }
            self.edges.insert(path);
        }
        syn::visit::visit_expr_call(self, call);
    }
    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        // `Vec<String>::join` is formatting, not thread waiting. This bounded
        // audit does not have Rust type information; recognize handle receivers.
        if call.method == "join" {
            if let syn::Expr::Path(p) = &*call.receiver {
                let receiver = path_name(&p.path);
                if receiver.contains("handle") || receiver.contains("worker") {
                    self.edges.insert("blocking_join".into());
                }
            }
            self.visit_expr(&call.receiver);
            for arg in &call.args {
                self.visit_expr(arg);
            }
            return;
        }
        self.edges.insert(call.method.to_string());
        syn::visit::visit_expr_method_call(self, call);
    }
    fn visit_expr_path(&mut self, path: &'ast syn::ExprPath) {
        self.edges.insert(path_name(&path.path));
    }
}

type Graph = BTreeMap<String, Vec<(String, BTreeSet<String>)>>;

fn collect(items: &[syn::Item], file: &str, graph: &mut Graph) {
    fn aliases(tree: &syn::UseTree, file: &str, graph: &mut Graph) {
        match tree {
            syn::UseTree::Rename(r) => {
                graph
                    .entry(r.rename.to_string())
                    .or_default()
                    .push((file.into(), BTreeSet::from([r.ident.to_string()])));
            }
            syn::UseTree::Path(p) => aliases(&p.tree, file, graph),
            syn::UseTree::Group(g) => {
                for t in &g.items {
                    aliases(t, file, graph);
                }
            }
            _ => {}
        }
    }
    for item in items {
        if let syn::Item::Use(u) = item {
            aliases(&u.tree, file, graph);
        }
    }
    let mut add = |name: String, body: &syn::Block| {
        let mut calls = Calls::default();
        calls.visit_block(body);
        graph
            .entry(name)
            .or_default()
            .push((file.into(), calls.edges));
    };
    for item in items {
        match item {
            syn::Item::Fn(f) => add(f.sig.ident.to_string(), &f.block),
            syn::Item::Impl(i) => {
                for member in &i.items {
                    if let syn::ImplItem::Fn(f) = member {
                        add(f.sig.ident.to_string(), &f.block);
                    }
                }
            }
            _ => {}
        }
    }
    for item in items {
        if let syn::Item::Mod(m) = item {
            let test = m.attrs.iter().any(|a| matches!(&a.meta, syn::Meta::List(l) if l.path.is_ident("cfg") && l.tokens.to_string() == "test"));
            if !test && let Some((_, items)) = &m.content {
                collect(items, file, graph);
            }
        }
    }
}

fn check(graph: &Graph, roots: &[&str]) -> Result<(), String> {
    fn walk(
        name: &str,
        graph: &Graph,
        seen: &mut BTreeSet<String>,
        chain: &mut Vec<String>,
    ) -> Result<(), String> {
        let short = name.rsplit("::").next().unwrap_or(name);
        chain.push(name.into());
        if SINKS.contains(&short) {
            return Err(format!(
                "blocking action on UI path: {}. Dispatch work via std::thread::spawn and report progress over a channel",
                chain.join(" -> ")
            ));
        }
        if seen.insert(short.into())
            && let Some(defs) = graph.get(short)
        {
            for (file, calls) in defs {
                for call in calls {
                    walk(call, graph, seen, chain).map_err(|e| format!("{file}: {e}"))?;
                }
            }
        }
        chain.pop();
        Ok(())
    }
    for root in roots {
        if !graph.contains_key(*root) {
            return Err(format!(
                "missing UI root {root}; update the audit when entry points change"
            ));
        }
        walk(root, graph, &mut BTreeSet::new(), &mut Vec::new())?;
    }
    Ok(())
}

pub fn audit(root: &Path) -> Result<(), String> {
    let mut graph = Graph::new();
    for file in crate::ast::rust_files_under(root, "crates/tui/src") {
        let source = crate::ast::parse(root, &file)?;
        collect(&source.ast.items, &file, &mut graph);
    }
    check(&graph, ROOTS)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn check_source(source: &str) -> Result<(), String> {
        let mut graph = Graph::new();
        collect(
            &syn::parse_file(source).unwrap().items,
            "fixture.rs",
            &mut graph,
        );
        check(&graph, &["event_loop"])
    }
    #[test]
    fn rejects_direct_and_wrapped_blocking_actions() {
        for source in [
            "fn event_loop() { app.mark_row(row); }",
            "fn event_loop() { helper(); } fn helper() { actions::execute_plan(); }",
            "fn event_loop() { helper(); } fn helper() { let f = || actions::propose(); f(); }",
            "fn event_loop() { rx.recv(); }",
            "use actions::execute_plan as run; fn event_loop() { run(); }",
            "fn event_loop() { let run = actions::execute_plan; run(); }",
            "fn event_loop() { std::thread::spawn(make_job()); } fn make_job() { actions::propose(); }",
            "fn event_loop() { std::thread::spawn(move || actions::execute_plan()); handle.join(); }",
        ] {
            assert!(check_source(source).is_err(), "missed regression: {source}");
        }
    }
    #[test]
    fn permits_worker_dispatch_and_nonblocking_polling() {
        assert!(check_source("fn event_loop() { start(); rx.try_recv(); } fn start() { std::thread::spawn(move || helper()); } fn helper() { actions::execute_plan(); }").is_ok());
        assert!(check_source("fn event_loop() {} #[cfg(test)] mod tests { fn event_loop() { actions::execute_plan(); } }").is_ok());
    }
    #[test]
    fn missing_roots_fail_closed() {
        assert!(check_source("fn renamed_loop() {}").is_err());
    }

    #[test]
    fn repository_tui_event_paths_are_nonblocking() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        audit(root).unwrap();
    }
}
