//! Source audits as a library, so the mutation corpus under `tests/` can
//! run each audit by name against a mutated copy of the real workspace.
//!
//! `program.rs` is the whole-crate model; `rules/` holds every rule,
//! written only against it; `audits.rs` registers them by name.

pub mod ast;
pub mod audits;
pub mod program;
pub mod resolve;
pub mod rules;
