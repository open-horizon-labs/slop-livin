//! target: crates/core/src/consumers/walk.rs
//! why: the same serial walk moved into a consumer, where a hand-written file list would not look
pub fn sweep_serial_consumer(root: &std::path::Path) {
    let _ = crate::attribution::attribute(root, &[], 1_000);
}
