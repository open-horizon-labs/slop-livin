//! target: crates/core/src/build_adapters/node.rs
//! why: alias/rename variant -- importing another adapter under a new name still couples them
use crate::build_adapters::cargo as other_adapter;
pub fn sweep_alias(p: &std::path::Path) -> bool {
    let _ = other_adapter::LABEL;
    crate::build_adapters::cargo::looks_like_target_triple(&p.display().to_string())
}
