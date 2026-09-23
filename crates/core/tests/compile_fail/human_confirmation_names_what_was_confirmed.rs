//! human-only-authorization (re-review 5, finding 2): a confirmation
//! names its subject. There is no bare "a human said yes" constructor:
//! `cli_approve` needs the plan that was shown (its id and content digest
//! go into the token), `cli_grant` the terms, `cli_protect` the change.
use swamp_core::authority::HumanConfirmed;

fn main() {
    let _anything = HumanConfirmed::cli_command("human:cli");
    let _no_plan = HumanConfirmed::cli_approve("human:cli");
}
