//! target: crates/core/src/report.rs
//! why: alias/rename variant -- the same stage reached under a renamed import
use crate::growth::observe_tracked_with_source as sweep_stage;

pub fn sweep_aliased_stage(dir: &std::path::Path, root: &std::path::Path) {
    let _ = sweep_stage(dir, root, 1_000);
}
