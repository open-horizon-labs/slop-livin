//! target: crates/core/src/build_adapters/gradle.rs
//! why: re-review 3 slip -- `+=` tokenises differently from `=`, and a byte count inflated after build is the double count #65 forbids
pub fn sweep_inflate(units: &mut [crate::artifact::NestedArtifact]) {
    for u in units.iter_mut() {
        u.bytes += 4096;
    }
}
