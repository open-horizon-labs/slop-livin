//! target: crates/tui/src/app.rs
//! mode: append
//! by: audit:gate_paths_only_inside_gates
//! source: re-review 5, finding 6
//! why: `#[allow(clippy::style)]` lowers every lint in the group, `disallowed_methods` and `disallowed_types` among them
/// Sweep 5: the whole style group off for one item.
#[allow(clippy::style)]
pub(crate) fn sweep5_styleless() {}
