//! target: crates/core/src/report.rs
//! why: sweep slip -- an `#[allow(dead_code)]` caller counted as a caller for a dead public API
#[allow(dead_code)]
fn sweep_fake_caller() {
    let _ = crate::activity::sweep_dead_api();
}
