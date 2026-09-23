//! target: crates/core/src/actions.rs
//! mode: append
//! by: audit:gate_paths_only_inside_gates
//! source: re-review 5, finding 6
//! why: `#[allow(clippy::disallowed_methods)]` outside the gate: the crate root denies the gate's lints so an ungated call does not compile; lowering one on an item re-opens it for everything in it
/// Sweep 5: a quiet corner in a sink.
#[allow(clippy::disallowed_methods)]
pub(crate) fn sweep5_quiet() {}
