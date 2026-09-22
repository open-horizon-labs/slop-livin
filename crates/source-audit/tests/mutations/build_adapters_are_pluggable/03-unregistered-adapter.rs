//! target: crates/core/src/build_adapters/bun.rs
//! why: an adapter module absent from the static registry identifies nothing and no test notices
pub const BUN_ADAPTER_ID: &str = "bun";
pub struct Adapter;
pub fn sweep_identify() -> Vec<u8> {
    Vec::new()
}
