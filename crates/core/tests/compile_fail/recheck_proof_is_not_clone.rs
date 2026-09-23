//! execution-sinks-recheck-live-state: a proof is not `Clone`, so one
//! recheck cannot be copied to license a second move.
use swamp_core::recheck::RecheckProof;

fn spare(proof: &RecheckProof) -> RecheckProof {
    proof.clone()
}

fn main() {}
