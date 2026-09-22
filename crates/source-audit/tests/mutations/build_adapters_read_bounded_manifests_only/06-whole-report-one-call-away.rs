//! target: crates/core/src/build_adapters/maven.rs
//! why: an uncapped read reached through a helper in another module (the last-report loader opens and decodes a whole file)
pub fn sweep_previous(store: &std::path::Path, root: &std::path::Path) -> bool {
    crate::report::load_last_report(store, root).is_some()
}
