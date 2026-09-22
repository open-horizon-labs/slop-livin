//! target: crates/core/src/consumers/growth.rs
//! why: a consumer naming the bus -- the coupling the bus exists to remove
pub fn sweep_register_self(bus: &mut crate::bus::EventBus) {
    let _ = bus;
}
