//! target: crates/core/src/report.rs
//! why: the history stage written straight from the report path, bypassing the consumer that owns it
pub fn sweep_direct_history(dir: &std::path::Path) {
    let _ = crate::growth::observe_and_annotate_dirs(dir, &[], 1_000, 30);
}
