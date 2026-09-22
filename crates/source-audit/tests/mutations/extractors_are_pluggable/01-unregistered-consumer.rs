//! target: crates/core/src/consumers/ecosystem.rs
//! why: a new consumer that EventBus::with_builtins does not register -- a source nobody can add without editing the bus
pub struct SweepUnregisteredConsumer;

impl Consumer for SweepUnregisteredConsumer {
    fn name(&self) -> &'static str {
        "sweep-unregistered"
    }
}
