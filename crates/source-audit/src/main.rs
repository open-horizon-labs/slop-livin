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
    // One byte formatter in the product: the TUI once divided by 1024
    // under a decimal "GB" label while core divided by 1000, so the same
    // number read 1.8GB in one surface and 2.0GB in the other and rows
    // visibly failed to sum to their total. The TUI re-exports core's.
    let tui_model = fs::read_to_string(root.join("tui/src/model.rs")).unwrap();
    assert!(
        !tui_model.contains("fn human_bytes(bytes: u64) -> String"),
        "the TUI must re-export core's byte formatter, never define a second one"
    );
    assert!(
        tui_model.contains("pub use slop_livin_core::render::human_bytes_pub as human_bytes"),
        "the TUI must re-export core's byte formatter"
    );
    let render = fs::read_to_string(root.join("core/src/render.rs")).unwrap();
    assert!(
        render.contains("while value >= 1000.0"),
        "byte units are decimal, matching their SI labels"
    );

    let mut audit = Audit { names: vec![] };
    audit.visit_file(&syn::parse_file(&scan).unwrap());
    assert!(
        audit.names.iter().any(|n| n == "scan"),
        "scan function must remain discoverable"
    );
}
