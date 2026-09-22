//! target: crates/core/src/build_adapters/jvm_common.rs
//! why: the "neutral" helper naming an adapter makes it a back door between Gradle and Maven
pub fn sweep_back_door() -> &'static str {
    use crate::build_adapters::maven::Origin as O;
    O::Unknown.label()
}
