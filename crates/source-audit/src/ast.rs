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
    pub ast: syn::File,
}

pub fn parse(root: &Path, rel: &str) -> Result<SourceFile, String> {
    let path = root.join(rel);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("read {rel}: {e}"))?;
    let ast = syn::parse_file(&text).map_err(|e| format!("{rel} is not valid Rust: {e}"))?;
    Ok(SourceFile {
        rel: rel.to_string(),
        text,
        ast,
    })
}

pub fn rust_files_under(root: &Path, rel_dir: &str) -> Vec<String> {
    let dir = root.join(rel_dir);
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return out;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) == Some("rs") {
            out.push(format!(
                "{rel_dir}/{}",
                p.file_name().unwrap().to_string_lossy()
            ));
        }
    }
    out.sort();
    out
}

/// Every free function and impl method in the file, with its name and
/// body token string (test modules excluded).
pub struct Func {
    pub name: String,
    pub body: String,
    pub stmts: Vec<String>,
}

pub fn functions(file: &syn::File) -> Vec<Func> {
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

/// Idents that appear as the last segment of a called path, method
/// name, or `use` path anywhere outside test modules: what the file
/// *reaches for*.
pub fn referenced_idents(file: &syn::File) -> Vec<String> {
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

/// Full paths (`a::b::c`) of every call expression outside tests.
pub fn call_paths(file: &syn::File) -> Vec<String> {
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

/// Every string literal outside test modules.
pub fn string_literals(file: &syn::File) -> Vec<String> {
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
    let mut out = Vec::new();
    for item in &file.items {
        if let syn::Item::Impl(i) = item
            && let Some((_, path, _)) = &i.trait_
            && path.segments.last().is_some_and(|s| s.ident == trait_name)
            && let syn::Type::Path(tp) = &*i.self_ty
        {
            out.push(tp.path.segments.last().unwrap().ident.to_string());
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
pub fn pub_struct_fields(file: &syn::File) -> Vec<PubField> {
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
                ty: f.ty.to_token_stream().to_string(),
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
