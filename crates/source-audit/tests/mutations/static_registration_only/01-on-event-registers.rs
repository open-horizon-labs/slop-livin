//! target: crates/core/src/consumers/tracking.rs
//! why: sweep slip -- consumer registration happening at event time instead of statically
pub struct SweepLateConsumer;

impl SweepLateConsumer {
    pub fn on_event(&self, bus: &mut SweepBus) {
        bus.register(SweepLateConsumer);
    }
}

pub struct SweepBus;
impl SweepBus {
    pub fn register(&mut self, _c: SweepLateConsumer) {}
}
