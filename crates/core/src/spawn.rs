//! The one way swamp builds a subprocess.
//!
//! Every spawn is counted by `work_counters`, which is what lets a test
//! assert "this observation ran no subprocess" with swamp's own
//! instrumentation. Re-review 3 (F2) found that pairing to be a habit:
//! every `Command::new` in core happened to sit under a
//! `record_spawn()`, three in the TUI did not, and nothing required
//! either. Building a `Command` only here makes the count structural;
//! the source audit `every_spawn_is_counted` rejects a `Command::new`
//! anywhere else.

use std::ffi::OsStr;
use std::process::Command;

/// A `Command` for `program`, with the spawn counted.
///
/// Counted at construction: every caller in this workspace builds a
/// command in order to run it, and counting here cannot be skipped by a
/// caller that runs it through `status`, `output` or `spawn`.
pub fn command(program: impl AsRef<OsStr>) -> Command {
    crate::work_counters::record_spawn();
    Command::new(program)
}

#[cfg(test)]
mod tests {
    #[test]
    fn building_a_command_counts_one_spawn() {
        let (_, counted) = crate::work_counters::measured(|| super::command("true"));
        assert_eq!(counted.subprocess_spawns, 1);
    }
}
