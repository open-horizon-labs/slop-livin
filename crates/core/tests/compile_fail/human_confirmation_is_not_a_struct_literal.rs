//! human-only-authorization: a `HumanConfirmed` is minted only by the CLI
//! command and TUI dialog constructors (whose call sites the gate audit
//! pins). A literal does not compile.
use swamp_core::authority::HumanConfirmed;

fn main() {
    let _c = HumanConfirmed {
        actor: String::from("agent"),
    };
}
