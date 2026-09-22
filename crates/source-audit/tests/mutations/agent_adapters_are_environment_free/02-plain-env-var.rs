//! target: crates/core/src/agents/opencode.rs
//! why: the original forbidden shape must still be rejected
pub fn sweep_plain_env() -> String {
    std::env::var("XDG_STATE_HOME").unwrap_or_default()
}
