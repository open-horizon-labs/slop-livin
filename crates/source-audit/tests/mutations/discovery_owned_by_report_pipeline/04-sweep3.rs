//! target: crates/core/src/report.rs
//! mode: append
//! why: re-review 3 sweep -- a second discovery pass inside report.rs, in a function that is not `observe_scope` (blind spot: the caller scan explicitly excludes `DISCOVERY_OWNER.0` (report.rs) as a whole file, so any other function there may run its own pass)
/// Sweep: a second pass over the shared history table.
pub fn sweep_refresh_units(
    scope: &crate::scope::EffectiveScope,
    store_dir: Option<&std::path::Path>,
    observed_at: u64,
) {
    let events = crate::fs_events::EventCoverage::default();
    let _ = crate::external::discover_and_measure(
        scope, store_dir, false, observed_at, 0, 0, &events,
    );
    let _ = crate::agents::discover_and_measure(
        scope, &[], store_dir, false, observed_at, 0, 0, &events,
    );
}
