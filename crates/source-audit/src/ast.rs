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

/// Struct-literal expressions whose path ends in `Enum::Variant` (or a
/// bare name), each with the names of the `if let` conditions enclosing
/// it and the function it sits in. Used for "X may only be built under
/// condition Y" audits.
pub struct StructSite {
    pub func: String,
    pub enclosing_if_let_calls: Vec<String>,
}

pub fn struct_literal_sites(file: &syn::File, path_suffix: &str) -> Vec<StructSite> {
    struct V<'s> {
        suffix: &'s str,
        func: String,
        conds: Vec<String>,
        out: Vec<StructSite>,
    }
    impl<'ast> Visit<'ast> for V<'_> {
        fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
            let prev = std::mem::replace(&mut self.func, f.sig.ident.to_string());
            syn::visit::visit_item_fn(self, f);
            self.func = prev;
        }
        fn visit_expr_if(&mut self, i: &'ast syn::ExprIf) {
            let mut pushed = 0;
            if let syn::Expr::Let(l) = &*i.cond
                && let syn::Expr::Call(c) = &*l.expr
                && let syn::Expr::Path(p) = &*c.func
            {
                self.conds
                    .push(p.path.segments.last().unwrap().ident.to_string());
                pushed = 1;
            }
            // Only the then-branch is guarded by the condition.
            self.visit_block(&i.then_branch);
            for _ in 0..pushed {
                self.conds.pop();
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
            if path.ends_with(self.suffix) {
                self.out.push(StructSite {
                    func: self.func.clone(),
                    enclosing_if_let_calls: self.conds.clone(),
                });
            }
            syn::visit::visit_expr_struct(self, s);
        }
    }
    let mut v = V {
        suffix: path_suffix,
        func: String::new(),
        conds: Vec::new(),
        out: Vec::new(),
    };
    v.visit_file(file);
    v.out
}

pub fn enum_has_variant(file: &syn::File, enum_name: &str, variant: &str) -> bool {
    file.items.iter().any(|i| {
        matches!(i, syn::Item::Enum(e) if e.ident == enum_name && e.variants.iter().any(|v| v.ident == variant))
    })
}
