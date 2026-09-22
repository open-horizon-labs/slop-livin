//! Source audits as a library, so the mutation corpus under `tests/` can
//! run each audit by name against a mutated copy of the real workspace.
//!
//! Splitting the binary into `lib.rs` + a thin `main.rs` is what makes
//! `GUARDRAILS_SPEC.md` section 17's mutation corpus possible: an
//! integration test cannot reach into a `bin` target, and an audit that
//! cannot be run against a deliberately broken tree cannot be shown to
//! reject anything.

pub mod ast;
pub mod audits;
pub mod platform_audits;
pub mod repair_audits;
pub mod resolve;
pub mod tui_nonblocking;
