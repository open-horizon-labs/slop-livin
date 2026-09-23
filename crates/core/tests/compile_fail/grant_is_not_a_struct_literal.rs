//! human-only-authorization (re-review 5, finding 1): a `Grant` is built
//! only where a human confirmation is spent (`approve_confirmed`,
//! `add_standing_grant_confirmed`) or by the loader that verified it. A
//! hand-built grant with any budget and any plan does not compile.
use swamp_core::actions::Grant;

fn main() {
    let _grant = Grant {
        id: String::from("forged"),
        plan_id: Some(String::from("plan")),
        budget_bytes: Some(u64::MAX),
        revoked: false,
    };
}
