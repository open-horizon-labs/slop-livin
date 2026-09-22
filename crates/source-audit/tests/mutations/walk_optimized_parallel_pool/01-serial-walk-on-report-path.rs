//! target: crates/core/src/report.rs
//! why: the serial, test-only walk called on the report path, so a large tree costs what the pool exists to avoid
pub fn sweep_serial_report(root: &std::path::Path) {
    let _ = crate::attribution::attribute(root, &[], 1_000);
}
