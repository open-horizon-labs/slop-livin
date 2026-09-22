//! target: crates/core/src/build_adapters/jvm_common.rs
//! why: alias/rename variant -- `std::mem::replace` as a local name writes the action with no `=` on a unit field
use std::mem::replace as swap_in;
pub fn sweep_swap(u: &mut crate::artifact::NestedArtifact) {
    let _ = swap_in(&mut u.action, crate::artifact::NestedActionCapability::InspectionOnly);
}
