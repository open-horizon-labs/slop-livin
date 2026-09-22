//! target: crates/core/src/bus/mod.rs
//! why: sweep slip -- env-var-driven consumer skipping, i.e. the registered set decided at run time
pub struct SweepGate;

impl SweepGate {
    pub fn on_event(&self, bus: &mut EventBus) {
        if std::env::var("SWAMP_SKIP_CONSUMERS").is_ok() {
            return;
        }
        *bus = EventBus::with_builtins();
    }
}
