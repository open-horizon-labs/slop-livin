//! human-only-authorization (re-review 5, finding 2): one TUI
//! confirmation authorizes one unit, once. `authorize_confirmed` takes it
//! by value, so authorizing a second path with the same confirmation does
//! not compile (and at runtime it refuses any path but the one it names).
use std::path::Path;
use swamp_core::authority::{HumanConfirmed, authorize_confirmed};

fn widen(confirmed: HumanConfirmed) {
    let _a = authorize_confirmed(confirmed, Path::new("/w/target"));
    let _b = authorize_confirmed(confirmed, Path::new("/w/src"));
}

fn main() {}
