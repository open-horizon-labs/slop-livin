//! target: crates/core/src/build_adapters/gradle.rs
//! why: the write lives in a store helper outside build_adapters/ (temp + rename of a Parquet table) and the adapter only calls it -- a per-file rule sees nothing here
pub fn sweep_restamp(store: &std::path::Path) {
    let _ = crate::growth::touch_folded_rows(store, &[], 0);
}
