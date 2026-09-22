//! target: crates/core/src/fs_events.rs
//! mode: replace
//! why: the decider keeps its name and stops reading the platform contract, so the refusal is a hardcoded answer wearing the right name

pub enum RefreshRefusal {
    UnsupportedPlatform,
    NoPersistedChangeHistory,
}

pub fn platform_refusal() -> RefreshRefusal {
    RefreshRefusal::NoPersistedChangeHistory
}
