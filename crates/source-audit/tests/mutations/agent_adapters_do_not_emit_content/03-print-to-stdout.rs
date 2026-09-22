//! target: crates/core/src/agents/aider.rs
//! why: an adapter returns data; the shared layer renders it through the redaction-aware path
pub fn sweep_emit_stdout(session: &str) {
    use std::io::Write;
    let mut out = std::io::stdout();
    let _ = out.write_all(session.as_bytes());
}
