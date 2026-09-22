//! target: crates/core/src/consumers/history.rs
//! why: the same runtime registration written in a different consumer file
pub struct SweepHistoryLate;

impl SweepHistoryLate {
    pub fn on_event(&self) {
        let mut registry = SweepRegistry;
        registry.register(());
    }
}

pub struct SweepRegistry;
impl SweepRegistry {
    pub fn register(&mut self, _c: ()) {}
}
