//! target: crates/core/src/build_adapters/maven.rs
//! why: a grouped `use` of a sibling adapter has no `super::gradle::` token sequence for a text rule to find
use super::{gradle, BuildCtx as _Ctx};
pub fn sweep_group() -> bool {
    let _ = gradle::Adapter;
    true
}
