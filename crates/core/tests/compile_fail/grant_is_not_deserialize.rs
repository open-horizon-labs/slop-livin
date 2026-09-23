//! human-only-authorization (re-review 5, finding 1): grants come back
//! from disk only through `actions::list_grants`, which refuses a record
//! whose keyed binding does not match. `Grant` is not `Deserialize`, so a
//! grant parsed from a hand-edited `grants.json` does not compile.
use swamp_core::actions::Grant;

fn main() {
    let _grants: Vec<Grant> = serde_json::from_str("[]").unwrap();
}
