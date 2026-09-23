//! target: crates/core/src/growth.rs
//! mode: append
//! expect: reject
//! source: review-4 sweep (U group, reviewer_counterexamples_stack4_sweep.rs), U1
//! by: audit:gate_paths_only_inside_gates, compile:clippy::disallowed_methods
//! why: U1: a bare caller-supplied `remove_dir_all`, in a function whose doc comment mentions `#[cfg(test)]`
//! blind-spot (old model): `program::is_test_attr` removes spaces from every attribute's tokens and looks for `cfg(test)`; a doc comment is `#[doc = "..."]`, so the words `cfg(test)` in prose make the item test code and invisible to all 45 audits (live in the real tree: `fs_events::testing`, used by the TUI in production)
/// Sweep 4 U1: production code, not behind `#[cfg(test)]`.
pub fn sweep4_u1_prune(path: &Path) -> std::io::Result<()> {
    use std::fs as sfs;
    sfs::remove_dir_all(path)
}
