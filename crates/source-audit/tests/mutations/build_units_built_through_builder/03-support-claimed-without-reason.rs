//! target: crates/core/src/build_adapters/maven.rs
//! why: declaring a layout supported with an empty reason is an unreviewed support claim
pub fn sweep_support(b: super::NestedUnitBuilder) -> super::NestedUnitBuilder {
    b.supported_with_reason("")
}
