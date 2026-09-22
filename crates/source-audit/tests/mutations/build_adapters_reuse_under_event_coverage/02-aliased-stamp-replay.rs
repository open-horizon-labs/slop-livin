//! target: crates/core/src/build_adapters/mod.rs
//! why: alias/rename variant -- the same stamp-only decision written as a replay over `mod_time`
pub fn sweep_replay_rows(rows: &[(String, i32)], mod_time_seen: i32) -> Vec<String> {
    rows.iter()
        .filter(|(_, mod_time)| *mod_time <= mod_time_seen)
        .map(|(id, _)| id.clone())
        .collect()
}
