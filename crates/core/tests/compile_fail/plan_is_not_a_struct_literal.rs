//! human-only-authorization (re-review 5, finding 1): a `Plan` is built
//! only by a propose path or by the store loader that verified its keyed
//! binding. Its fields are private to `actions`, so a hand-built plan --
//! one naming a unit nobody proposed -- does not compile.
use std::path::PathBuf;
use swamp_core::actions::Plan;

fn main() {
    let _plan = Plan {
        id: String::from("forged"),
        root: PathBuf::from("/"),
        units: Vec::new(),
    };
}
