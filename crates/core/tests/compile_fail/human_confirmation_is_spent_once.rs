//! human-only-authorization (re-review 5, finding 2): a confirmation is
//! single-use. `approve_confirmed` takes it by value, and it is not
//! `Clone`, so approving twice -- or approving a second plan -- with one
//! human keypress does not compile.
use std::path::Path;
use swamp_core::authority::HumanConfirmed;

fn twice(dir: &Path, confirmed: HumanConfirmed) {
    let _ = swamp_core::actions::approve_confirmed(dir, "plan-a", confirmed);
    let _ = swamp_core::actions::approve_confirmed(dir, "plan-b", confirmed);
}

fn main() {}
