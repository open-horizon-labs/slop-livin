//! human-only-authorization (re-review 5, finding 1): a loaded plan is
//! read-only outside `actions`. Adding a unit after approval -- which an
//! approval by plan id alone used to cover -- does not compile; and on
//! disk the plan's keyed binding and the grant's content digest refuse
//! the same edit made to the file.
use std::path::Path;

fn main() {
    let mut plan = swamp_core::actions::load_plan(Path::new("/tmp/store"), "p").unwrap();
    let extra = plan.units()[0].clone();
    plan.units.push(extra);
    swamp_core::actions::save_plan(Path::new("/tmp/store"), &plan).unwrap();
}
