//! target: crates/core/src/build_adapters/android.rs
//! expect: accept
//! source: accept
//! why: a build adapter's own module may name its same-named detector's
//!      id (`build_adapters::android` / `locations::android`) -- the same
//!      twin relationship a tool adapter already has with its detector
//!      (`every_adapter_id_is_also_a_detector_id`), one family over.
fn sweep_accept_same_family_id() -> &'static str {
    "android"
}
