//! target: crates/cli/src/main.rs
//! mode: substitute
//! find: let g = swamp_core::actions::approve_confirmed(&swamp_dir(), plan_id, &confirmed)?;
//! expect: accept
//! source: review-4 sweep (A group, reviewer_counterexamples_stack4_sweep.rs), A3
//! ported: 2026-09-22, to the confirmed-approval API (`approve_confirmed` + `HumanConfirmed`); the shape -- the reviewed call extracted into a helper beside `cmd_approve` -- is the reviewer's
//! why: A3: the reviewed `cmd_approve` handler with its approve call extracted into a helper beside it (the `helper` accept-operator, applied to a real reviewed caller)
//! blind-spot (old model): the reviewed callers are a `(file, fn)` list; a behaviour-preserving extraction moves the call out of a listed name -- the operator harness asserts `helper` keeps legitimate shapes accepted, but no seed exercises it
let g = sweep4_a3_approve(plan_id, &confirmed)?;
//! file: crates/cli/src/main.rs
//! mode: append
/// Sweep 4 A3: extracted from `cmd_approve`, called only by it.
fn sweep4_a3_approve(
    plan_id: &str,
    confirmed: &swamp_core::authority::HumanConfirmed,
) -> Result<swamp_core::actions::Grant> {
    swamp_core::actions::approve_confirmed(&swamp_dir(), plan_id, confirmed)
}
