//! human-only-authorization: an `Authorized` token comes only from
//! `authority::authorize` (a live grant a human minted) or
//! `authority::authorize_confirmed` (a human confirmation). A literal does
//! not compile.
use swamp_core::authority::Authorized;
use swamp_core::fs_gate::StoreDir;

fn main() {
    let _auth = Authorized {
        grant_id: String::from("forged"),
        store: StoreDir::resolved(),
    };
}
