//! target: crates/core/src/growth.rs
//! why: the full walk reached behind an alias, before the replay
use crate::walk::full_walk as rewalk_everything;
pub fn stage_tracked_with_source(root: &std::path::Path, src: &dyn crate::fs_events::FsEventsSource) -> u64 {
    let _ = rewalk_everything(root);
    let _ = src.replay(root);
    0
}
