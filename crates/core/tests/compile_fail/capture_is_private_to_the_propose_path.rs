//! execution-sinks-recheck-live-state (re-review 5, finding 3): a
//! reviewed identity is recorded when a unit is proposed, inside
//! swamp-core. `recheck::capture` is private to the crate (the TUI's
//! mark-time `capture_anchor` is pinned by the gate audit), so taking a
//! fresh "reviewed" identity at execution time does not compile.
use std::path::Path;

fn main() {
    let _now = swamp_core::recheck::capture(Path::new("/w/target"));
    let _check = swamp_core::recheck::reviewed_snapshot(Path::new("/w/target"), None);
}
