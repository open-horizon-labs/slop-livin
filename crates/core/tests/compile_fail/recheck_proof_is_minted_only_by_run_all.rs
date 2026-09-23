//! execution-sinks-recheck-live-state: a `RecheckProof` exists only as
//! the result of `recheck::run_all`. Building one by hand -- claiming the
//! rechecks ran -- does not compile.
use std::path::PathBuf;
use swamp_core::recheck::RecheckProof;

fn main() {
    let _proof = RecheckProof {
        anchor: PathBuf::from("/Users/me/src/p/target/debug"),
        covered: vec![],
    };
}
