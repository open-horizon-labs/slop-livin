//! target: crates/core/src/consumer_wiring.rs
//! why: wiring matching on a detector id constant, so adding a detector means editing a table
pub fn sweep_wire_by_id(id: &str) -> bool {
    id == crate::locations::CARGO_HOME_DETECTOR_ID
}
