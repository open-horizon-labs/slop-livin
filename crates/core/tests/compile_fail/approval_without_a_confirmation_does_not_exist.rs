//! human-only-authorization: approving a plan or minting a standing grant
//! takes a `&HumanConfirmed`. The old string-actor entry points exist only
//! under the `testing` feature, which production builds cannot enable.
use std::path::Path;

fn main() {
    let _ = swamp_core::actions::approve(Path::new("/tmp/store"), "plan-id");
    let _ = swamp_core::actions::add_standing_grant(Path::new("/tmp/store"), "kind:cargo");
}
