//! target: crates/core/src/consumers/gate.rs
//! why: the composed guardrail's second half -- registration decided at event time
pub struct SweepGateLate;

impl SweepGateLate {
    pub fn on_event(&self, bus: &mut SweepGateBus) {
        bus.register(());
    }
}

pub struct SweepGateBus;
impl SweepGateBus {
    pub fn register(&mut self, _c: ()) {}
}
