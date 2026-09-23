//! human-only-authorization (re-review 5, finding 1): no `Plan` comes
//! from bytes except through `actions::load_plan`, which checks the
//! record's binding under the store's authority key first. `Plan` is not
//! `Deserialize`, so parsing a plan file yourself -- and skipping that
//! check -- does not compile.
use swamp_core::actions::Plan;

fn main() {
    let _plan: Plan = serde_json::from_str("{}").unwrap();
}
