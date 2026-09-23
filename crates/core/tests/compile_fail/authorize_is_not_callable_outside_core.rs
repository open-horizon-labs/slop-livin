//! human-only-authorization (re-review 5, finding 1): turning a grant
//! into an `Authorized` is the plan executor's step, inside swamp-core,
//! from a plan and grant the loader verified. `authority::authorize` is
//! private to the crate, so calling it with a hand-picked plan, unit or
//! grant does not compile.
use std::path::Path;

fn main() {
    let plan = swamp_core::actions::load_plan(Path::new("/tmp/store"), "p").unwrap();
    let grants = swamp_core::actions::list_grants(Path::new("/tmp/store")).unwrap();
    let store = swamp_core::fs_gate::StoreDir::resolved();
    let _auth =
        swamp_core::authority::authorize(&plan, "digest", 0, &grants[0], &store, 0, 0, 0);
}
