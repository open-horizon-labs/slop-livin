use std::{fs, path::Path};
use syn::visit::Visit;

struct Audit {
    names: Vec<String>,
}
impl<'ast> Visit<'ast> for Audit {
    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        self.names.push(node.sig.ident.to_string());
        syn::visit::visit_item_fn(self, node);
    }
}
fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let store = fs::read_to_string(root.join("core/src/store.rs")).unwrap();
    let scan = fs::read_to_string(root.join("core/src/scan.rs")).unwrap();
    let grants = fs::read_to_string(root.join("core/src/grants.rs")).unwrap();
    for (name, src) in [
        ("store", store.as_str()),
        ("scan", scan.as_str()),
        ("grants", grants.as_str()),
    ] {
        syn::parse_file(src).unwrap_or_else(|e| panic!("{name} is not valid Rust: {e}"));
    }
    assert!(
        store.contains("ArrowWriter"),
        "store must write real Parquet"
    );
    assert!(
        store.contains("WriterProperties"),
        "Parquet writer must declare compression"
    );
    assert!(
        scan.contains("canonical_roots"),
        "roots must be canonicalized at the boundary"
    );
    assert!(
        grants.contains("created_outside_index"),
        "grants need non-index provenance"
    );
    let mut audit = Audit { names: vec![] };
    audit.visit_file(&syn::parse_file(&scan).unwrap());
    assert!(
        audit.names.iter().any(|n| n == "scan"),
        "scan function must remain discoverable"
    );
}
