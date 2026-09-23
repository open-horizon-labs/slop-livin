//! human-only-authorization: an `Authorized` token comes only from
//! `authority::authorize` (a live grant a human minted) or
//! `authority::authorize_confirmed` (a human confirmation). A literal does
//! not compile.
use std::path::PathBuf;
use swamp_core::authority::Authorized;

fn main() {
    let _auth = Authorized {
        anchors: vec![PathBuf::from("/")],
        grant_id: String::from("forged"),
    };
}
