//! target: crates/core/src/walk.rs
//! why: the walker naming consumers/, so adding a source would require changing the walker
pub fn sweep_walker_calls_consumer() {
    let _ = crate::consumers::growth::name_of_this_consumer();
}
