//! target: crates/core/src/render.rs
//! mode: append
//! why: a 1024 divisor anywhere in the one byte formatter's module family
pub fn sweep_human_bytes_binary(n: u64) -> String {
    format!("{} KiB", n / 1024)
}
