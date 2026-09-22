//! Independent re-review 3: the complete per-audit mutation sweep.
//!
//! Re-review 2's sweep was interrupted. This is the finished version:
//! for **every** registered audit, one compiling, harmful mutation that
//! is **not** in `crates/source-audit/tests/mutations/`, applied to a
//! copy of the real workspace. An audit that returns `Ok(())` on its
//! own mutation has slipped.
//!
//! Written by the reviewer, not the author. Nothing here edits
//! production code: the mutations live in this file and are applied to a
//! throwaway copy of the tree.

use std::path::{Path, PathBuf};
use swamp_source_audit::audits::AUDITS;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// Append to an existing real file, so the mutation lives beside
    /// real code.
    Append,
    /// A new file that did not exist.
    Create,
}

struct Mutation {
    audit: &'static str,
    /// What the mutation does and why it is harmful.
    why: &'static str,
    /// Which blind spot of the audit it exploits.
    blind_spot: &'static str,
    files: &'static [(&'static str, Mode, &'static str)],
}

const M: &[Mutation] = &[
    // -----------------------------------------------------------------
    Mutation {
        audit: "tui_actions_off_event_thread",
        why: "an `lsof` spawn per keystroke, hidden inside `vec![..]`",
        blind_spot: "syn::visit never descends into macro tokens, so any sink inside a macro invocation is invisible to the call graph",
        files: &[(
            "crates/tui/src/app.rs",
            Mode::Append,
            r#"
impl App {
    /// Sweep: a blocking lsof probe on the key path, inside a macro.
    pub fn handle_key_occupancy(&mut self, p: &std::path::Path) {
        let _states = vec![swamp_core::occupancy::probe_path(p)];
    }
}
"#,
        )],
    },
    Mutation {
        audit: "one_byte_formatter",
        why: "a second byte formatter dividing by 1024 with binary labels taken from a unit table",
        blind_spot: "the `labels` half requires a KiB/MiB literal *inside the format! macro*; a unit table moves the label out of the macro",
        files: &[(
            "crates/tui/src/model.rs",
            Mode::Append,
            r#"
const SWEEP_BINARY_UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];

/// Sweep: a second formatter, binary divisor, binary labels.
pub fn sweep_format_bytes(n: u64) -> String {
    let mut v = n as f64;
    let mut i = 0usize;
    while v >= 1024.0 && i + 1 < SWEEP_BINARY_UNITS.len() {
        v /= 1024.0;
        i += 1;
    }
    format!("{:.1} {}", v, SWEEP_BINARY_UNITS[i])
}
"#,
        )],
    },
    Mutation {
        audit: "legacy_invariants",
        why: "`canonical_roots` calls canonicalize and throws the answer away, returning the raw roots",
        blind_spot: "the rule is `body.contains(\"canonicalize\")` -- presence of the call, not use of its result (slip class 2, never closed for this audit)",
        files: &[(
            "crates/core/src/scan.rs",
            Mode::Append,
            r#"
/// Sweep: keeps the name and the call, drops the invariant.
pub mod sweep_boundary {
    pub fn canonical_roots(roots: Vec<std::path::PathBuf>) -> Vec<std::path::PathBuf> {
        for r in &roots {
            let _ = std::fs::canonicalize(r);
        }
        roots
    }
}
"#,
        )],
    },
    Mutation {
        audit: "fsevents_before_full_walk",
        why: "a staged replay that runs a full tree walk before consulting FSEvents, spelled `discover_and_attribute`",
        blind_spot: "the ordering rule greps statements for the literal token `full_walk`; any other spelling of a full traversal is invisible",
        files: &[(
            "crates/core/src/growth.rs",
            Mode::Append,
            r#"
/// Sweep: walks everything, then replays.
pub mod sweep_stage {
    pub struct SweepStream;
    impl SweepStream {
        pub fn replay(&self, _since: u64) -> Vec<String> {
            Vec::new()
        }
    }

    fn sweep_scan_all(dir: &std::path::Path, n: &mut usize) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            *n += 1;
            if e.path().is_dir() {
                sweep_scan_all(&e.path(), n);
            }
        }
    }

    pub fn stage_tracked_with_source(root: &std::path::Path) -> Vec<String> {
        let mut n = 0usize;
        sweep_scan_all(root, &mut n);
        let _events = SweepStream.replay(0);
        Vec::new()
    }
}
"#,
        )],
    },
    Mutation {
        audit: "column_store_parquet_zstd",
        why: "an uncompressed Parquet writer in a third file",
        blind_spot: "the audit reads exactly two files (growth.rs, store.rs); a writer anywhere else is unaudited",
        files: &[(
            "crates/core/src/report.rs",
            Mode::Append,
            r#"
/// Sweep: Parquet, no compression, outside the two audited files.
pub fn sweep_write_rows(path: &std::path::Path, batch: arrow_array::RecordBatch) {
    let file = std::fs::File::create(path).unwrap();
    let mut w = parquet::arrow::ArrowWriter::try_new(file, batch.schema(), None).unwrap();
    w.write(&batch).unwrap();
    w.close().unwrap();
}
"#,
        )],
    },
    Mutation {
        audit: "reverse_delta_current_plus_deltas",
        why: "the delta path is computed and discarded; history is rewritten in place",
        blind_spot: "`body.contains(\"next_delta_path\")` -- the name appearing, not a delta being appended",
        files: &[(
            "crates/core/src/growth.rs",
            Mode::Append,
            r#"
/// Sweep: names both paths, writes only the current one.
pub mod sweep_history {
    pub fn observe_and_annotate(dir: &std::path::Path) {
        let cur = super::current_path(dir);
        let _delta = super::next_delta_path(dir);
        let _ = std::fs::write(&cur, b"");
    }
}
"#,
        )],
    },
    Mutation {
        audit: "scheduled_refresh_launchagent",
        why: "scheduling moved to crontab while the word `launchctl` survives as a dead string",
        blind_spot: "the audit asserts a `Schedule` variant exists and the literal \"launchctl\" appears somewhere in schedule.rs; neither says what actually schedules",
        files: &[(
            "crates/core/src/schedule.rs",
            Mode::Append,
            r#"
/// Sweep: the documented LaunchAgent contract is no longer what runs.
pub fn sweep_install_schedule() -> std::io::Result<()> {
    let _legacy_note = "launchctl";
    std::process::Command::new("crontab").arg("-").status()?;
    Ok(())
}
"#,
        )],
    },
    Mutation {
        audit: "folding_only_for_artifacts",
        why: "a directory folded as a build artifact with no classification behind it",
        blind_spot: "the guard exemption is `s.func.starts_with(\"resize_artifact\")` -- a prefix, so any new function whose name starts that way may fold anything",
        files: &[(
            "crates/core/src/walk.rs",
            Mode::Append,
            r#"
/// Sweep: folds whatever it is handed. Exempt by name prefix.
pub fn resize_artifact_unchecked(
    tx: &std::sync::mpsc::Sender<AttrJob>,
    path: std::path::PathBuf,
    group: std::sync::Arc<SizeGroup>,
) {
    let _ = tx.send(AttrJob::Size { path, group });
}
"#,
        )],
    },
    Mutation {
        audit: "symlinks_never_followed",
        why: "`fs::metadata` (which follows symlinks) sizing a reviewed member in the recheck module",
        blind_spot: "the audit reads exactly walk.rs and attribution.rs; recheck.rs -- whose snapshot gates every destructive sink -- is not one of them",
        files: &[(
            "crates/core/src/recheck.rs",
            Mode::Append,
            r#"
/// Sweep: stats through the link.
pub fn sweep_member_size(p: &std::path::Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}
"#,
        )],
    },
    Mutation {
        audit: "incremental_walk_only_changed_subtrees",
        why: "the incremental path delegates a full walk to a helper one function away",
        blind_spot: "the forbidden tokens are looked for only in `apply_incremental`'s own body; the callee is never followed",
        files: &[(
            "crates/core/src/growth.rs",
            Mode::Append,
            r#"
/// Sweep: keeps the required names, does the forbidden thing next door.
pub mod sweep_incremental {
    pub fn apply_incremental(root: &std::path::Path) {
        let _targeted = crate::walk::attribute_one_worktree;
        let _resize = crate::walk::resize_artifact;
        sweep_rebuild_everything(root);
    }

    fn sweep_rebuild_everything(dir: &std::path::Path) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            if e.path().is_dir() {
                sweep_rebuild_everything(&e.path());
            }
        }
    }
}
"#,
        )],
    },
    Mutation {
        audit: "walk_optimized_parallel_pool",
        why: "the serial, test-only walk called from the external measurement pass",
        blind_spot: "the caller list is report.rs plus consumers/; external.rs is on the report path and is not in it",
        files: &[(
            "crates/core/src/external.rs",
            Mode::Append,
            r#"
/// Sweep: the serial walk, on the report path, outside the audited list.
pub fn sweep_serial_fold(root: &std::path::Path) {
    let _ = crate::attribution::attribute(root, &[], 0);
}
"#,
        )],
    },
    Mutation {
        audit: "dir_mtime_int32_minutes",
        why: "a second directory schema storing seconds in an Int64 column",
        blind_spot: "the rule inspects functions named exactly `dirs_schema`/`files_schema`; a renamed schema builder is unaudited",
        files: &[(
            "crates/core/src/growth.rs",
            Mode::Append,
            r#"
/// Sweep: the same table, a different builder name, seconds.
pub fn dirs_schema_v2() -> std::sync::Arc<arrow_schema::Schema> {
    std::sync::Arc::new(arrow_schema::Schema::new(vec![
        arrow_schema::Field::new("rel_path", arrow_schema::DataType::Utf8, false),
        arrow_schema::Field::new("mod_time_secs", arrow_schema::DataType::Int64, false),
    ]))
}
"#,
        )],
    },
    Mutation {
        audit: "agent_interface_facts_not_verdicts",
        why: "a verdict string reaches the renderer through a constant defined outside the scanned files",
        blind_spot: "only render.rs, agent_json.rs, cli/ and tui/ literals are scanned; a `pub const` in core carries the verdict past them",
        files: &[
            (
                "crates/core/src/agents/mod.rs",
                Mode::Append,
                r#"
/// Sweep: the verdict, defined where nothing looks for it.
pub const SWEEP_VERDICT: &str = "safe to delete";
"#,
            ),
            (
                "crates/core/src/render.rs",
                Mode::Append,
                r#"
/// Sweep: prints the verdict without ever spelling it here.
pub fn sweep_render_verdict(out: &mut String) {
    out.push_str(crate::agents::SWEEP_VERDICT);
}
"#,
            ),
        ],
    },
    Mutation {
        audit: "human_only_authorization",
        why: "a standing grant minted from the TUI through a local re-export module",
        blind_spot: "the rule is `body.contains(\"actions :: <sink> (\")` after one level of *use-alias* resolution; a `pub use` inside a local `mod` is a second hop the resolver does not take",
        files: &[(
            "crates/tui/src/actions.rs",
            Mode::Append,
            r#"
/// Sweep: one module hop hides the minting call.
pub mod sweep_shim {
    pub use swamp_core::actions::add_standing_grant;
}

pub fn sweep_authorize(dir: &std::path::Path) {
    let _ = sweep_shim::add_standing_grant(dir, "agent:*", 0, None, 0, "agent");
}
"#,
        )],
    },
    Mutation {
        audit: "no_consumer_knows_other_consumers",
        why: "one consumer reaching into another consumer's module for its parsing",
        blind_spot: "coupling is detected only through the *struct* names that `impl Consumer`; a free function in another consumer module is invisible",
        files: &[
            (
                "crates/core/src/consumers/cache.rs",
                Mode::Append,
                r#"
/// Sweep: reaches into the docker consumer's own helpers.
pub fn sweep_reuse_docker_parsing(s: &str) -> u64 {
    crate::consumers::docker::sweep_parse_helper(s)
}
"#,
            ),
            (
                "crates/core/src/consumers/docker.rs",
                Mode::Append,
                r#"
/// Sweep: the helper the other consumer now depends on.
pub fn sweep_parse_helper(s: &str) -> u64 {
    s.len() as u64
}
"#,
            ),
        ],
    },
    Mutation {
        audit: "static_registration_only",
        why: "consumers registered at run time, from a helper that is not called `on_event`",
        blind_spot: "the rule inspects only functions *named* `on_event`",
        files: &[(
            "crates/core/src/bus/mod.rs",
            Mode::Append,
            r#"
impl EventBus {
    /// Sweep: the registered set decided while events are flowing.
    pub fn sweep_register_late(&mut self, c: Box<dyn Consumer>) -> anyhow::Result<()> {
        self.register(c)
    }
}
"#,
        )],
    },
    Mutation {
        audit: "all_report_paths_through_bus",
        why: "a pipeline stage called straight from the report path through a renamed re-export",
        blind_spot: "the rule matches the *last path segment* against a fixed stage-name list; a `pub use .. as` inside a local module renames the segment",
        files: &[(
            "crates/core/src/report.rs",
            Mode::Append,
            r#"
mod sweep_stage_shim {
    pub use crate::walk::discover_and_attribute as gather;
}

/// Sweep: the stage, off the bus, under another name.
pub fn sweep_report_direct(root: &std::path::Path) {
    let _ = sweep_stage_shim::gather(root, 0, 0, &[]);
}
"#,
        )],
    },
    Mutation {
        audit: "extractors_are_pluggable",
        why: "a consumer registered twice, so every event it handles is handled twice",
        blind_spot: "registration is checked for *presence* in `with_builtins`, never for a count -- unlike the adapter registry, which does count",
        files: &[(
            "crates/core/src/bus/mod.rs",
            Mode::Append,
            r#"
/// Sweep: the same consumer, registered a second time.
pub fn sweep_with_builtins_twice(bus: &mut EventBus) -> Result<()> {
    bus.register(Box::new(crate::consumers::CacheWriter))?;
    bus.register(Box::new(crate::consumers::CacheWriter))?;
    Ok(())
}
"#,
        )],
    },
    Mutation {
        audit: "event_bus_pluggable_consumers",
        why: "a consumer defined in consumers/mod.rs: it names the bus, registers at run time and is never in with_builtins",
        blind_spot: "`consumer_files` filters out `/mod.rs`, so the whole umbrella is blind to a consumer that lives there",
        files: &[(
            "crates/core/src/consumers/mod.rs",
            Mode::Append,
            r#"
/// Sweep: a consumer the bus audits cannot see.
#[derive(Default)]
pub struct SweepShadowConsumer;

#[async_trait::async_trait(?Send)]
impl crate::bus::Consumer for SweepShadowConsumer {
    fn name(&self) -> &str {
        "sweep-shadow"
    }
    fn subscribes_to(&self) -> &[crate::bus::EventKind] {
        &[]
    }
    async fn on_event(
        &self,
        _event: &crate::bus::Event,
        _ctx: &crate::bus::Ctx<'_>,
    ) -> anyhow::Result<Vec<crate::bus::Event>> {
        let _bus_is_known_here = crate::bus::EventBus::with_builtins();
        Ok(Vec::new())
    }
}
"#,
        )],
    },
    Mutation {
        audit: "adr_validation",
        why: "a new `severity: hard` guardrail with no `audit:` field at all -- unwatched, and the build says nothing",
        blind_spot: "the rule only checks that an `audit:` value *resolves*; a guardrail that omits the field is silently exempt, which is exactly how CE4's guardrail went unwatched",
        files: &[(
            ".oh/guardrails/sweep-unwatched-hard-guardrail.md",
            Mode::Create,
            r#"---
severity: hard
---

# Sweep: a hard guardrail nothing watches

Statement: execution never moves a path the user protected.

This file carries no `audit:` field, so `adr_validation` never looks at it.
"#,
        )],
    },
    Mutation {
        audit: "execution_sinks_recheck_live_state",
        why: "`fs::remove_dir_all` on a caller-supplied path with no recheck at all, written in growth.rs",
        blind_spot: "the audit skips recheck.rs, store.rs and growth.rs *by whole file*, so any destructive primitive placed in one of them is unaudited",
        files: &[(
            "crates/core/src/growth.rs",
            Mode::Append,
            r#"
/// Sweep: a destructive sink in a file the audit skips wholesale.
pub fn sweep_prune_unit(path: &std::path::Path) -> std::io::Result<()> {
    std::fs::remove_dir_all(path)
}
"#,
        )],
    },
    Mutation {
        audit: "protection_fails_closed",
        why: "a one-directional protection predicate, named without the word `protect`",
        blind_spot: "the \"one predicate everywhere\" rule only inspects functions whose name `contains(\"protect\")` -- the deleted `is_human_protected` renamed to `kept_conflict` is invisible again",
        files: &[(
            "crates/core/src/actions.rs",
            Mode::Append,
            r#"
/// Sweep: the second predicate, back, under a name the audit does not read.
pub fn sweep_kept_conflict(candidate: &std::path::Path, kept: &[std::path::PathBuf]) -> bool {
    kept.iter().any(|k| candidate.starts_with(k))
}
"#,
        )],
    },
    Mutation {
        audit: "discovery_consumes_effective_scope",
        why: "discovery reading raw detector output through a renamed local binding",
        blind_spot: "the forbidden shapes are token needles that pin the receiver name (`summary . locations`, `scope . detectors`); one `let s = summary;` defeats all three",
        files: &[(
            "crates/core/src/external.rs",
            Mode::Append,
            r#"
pub struct SweepLoc {
    pub path: std::path::PathBuf,
}
pub struct SweepSummary {
    pub locations: Vec<SweepLoc>,
}

/// Sweep: raw detector candidates, one rename away from the needle.
pub fn sweep_raw_candidates(summary: &SweepSummary) -> Vec<std::path::PathBuf> {
    let s = summary;
    s.locations.iter().map(|l| l.path.clone()).collect()
}
"#,
        )],
    },
    Mutation {
        audit: "explicit_only_scope_when_defaults_false",
        why: "a second scope resolver that infers detectors without asking `detectors_permitted`",
        blind_spot: "the guard is required only in functions named exactly `resolve_effective_scope`",
        files: &[(
            "crates/core/src/scope.rs",
            Mode::Append,
            r#"
/// Sweep: explicit-only scope, resolved by a second entry point.
pub fn resolve_effective_scope_for_root(
    config: &ScanConfig,
    root: &std::path::Path,
) -> Vec<std::path::PathBuf> {
    // No `detectors_permitted(config)` guard anywhere.
    let _ = config;
    let mut out = Vec::new();
    for e in std::fs::read_dir(root).into_iter().flatten().flatten() {
        out.push(e.path());
    }
    out
}
"#,
        )],
    },
    Mutation {
        audit: "history_sweeps_are_owned",
        why: "an unguarded tombstone whose right-hand side is a constant rather than the literal `false`",
        blind_spot: "a tombstone is recognised as `lhs.ends_with(\". present\") && rhs.trim() == \"false\"`; naming the constant defeats it",
        files: &[(
            "crates/core/src/growth.rs",
            Mode::Append,
            r#"
const SWEEP_ABSENT: bool = false;

/// Sweep: tombstones every row it is handed, owned or not.
fn sweep_tombstone_all(rows: &mut [StoredRow]) {
    for row in rows.iter_mut() {
        row.present = SWEEP_ABSENT;
    }
}
"#,
        )],
    },
    Mutation {
        audit: "no_second_traversal_on_report_path",
        why: "external.rs re-walking every root through the walker's own entry point",
        blind_spot: "`TRAVERSAL_CALLS` is a five-needle list (`read_dir`, `walkdir`, `jwalk`, `resize_artifact`); `walk::discover_and_attribute` is a full traversal and is not on it",
        files: &[(
            "crates/core/src/external.rs",
            Mode::Append,
            r#"
/// Sweep: a full re-walk from a file that must never traverse.
pub fn sweep_remeasure(root: &std::path::Path) -> u64 {
    let _ = crate::walk::discover_and_attribute(root, 0, 0, &[]);
    0
}
"#,
        )],
    },
    Mutation {
        audit: "occupancy_is_tristate_at_sinks",
        why: "`Unknown` treated as free by an inequality test instead of a match arm",
        blind_spot: "the refusal rule only inspects `match` arms whose pattern names `Unknown`; `state != Occupied` never produces one",
        files: &[(
            "crates/core/src/actions.rs",
            Mode::Append,
            r#"
/// Sweep: fail-open occupancy, written as an inequality.
pub fn sweep_can_remove(state: crate::occupancy::OccupancyState) -> bool {
    !matches!(state, crate::occupancy::OccupancyState::Occupied(_))
}
"#,
        )],
    },
    Mutation {
        audit: "tui_refresh_preserves_scope",
        why: "a TUI refresh through a new scopeless core entry point",
        blind_spot: "`SCOPELESS_REPORT_ENTRIES` is a hand-written list of eight names; a ninth scopeless entry point is unaudited the day it lands",
        files: &[
            (
                "crates/core/src/report.rs",
                Mode::Append,
                r#"
/// Sweep: a new scopeless report entry point.
pub fn report_quick(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    vec![root.to_path_buf()]
}
"#,
            ),
            (
                "crates/tui/src/app.rs",
                Mode::Append,
                r#"
impl App {
    /// Sweep: refreshes through it, dropping every exclusion.
    pub fn sweep_refresh(&mut self, root: &std::path::Path) {
        let _roots = swamp_core::report::report_quick(root);
    }
}
"#,
            ),
        ],
    },
    Mutation {
        audit: "store_data_is_parquet_not_json_sidecars",
        why: "a per-unit JSON sidecar written under the store by a helper that takes the directory as a parameter",
        blind_spot: "a function is only inspected when its own body contains the token `swamp_dir`/`store_dir`/`swamp_path`; passing the store directory in defeats it",
        files: &[(
            "crates/core/src/store.rs",
            Mode::Append,
            r#"
/// Sweep: one JSON file per unit, under the store, in a parallel database.
pub fn sweep_write_unit_cache(dir: &std::path::Path, id: &str) {
    let p = dir.join("units").join(format!("{id}-unit.json"));
    let _ = std::fs::write(p, b"{}");
}
"#,
        )],
    },
    Mutation {
        audit: "json_persistence_is_allowlisted",
        why: "JSON persisted with `serde_json::to_vec_pretty`, outside the allow-list",
        blind_spot: "`JSON_SERIALIZE_CALLS` lists to_vec / to_string / to_writer / Serializer / json!; the `_pretty` variants produce the same bytes and are on none of them",
        files: &[(
            "crates/core/src/report.rs",
            Mode::Append,
            r#"
/// Sweep: persistence the serializer needle cannot see.
pub fn sweep_persist_rows(p: &std::path::Path, v: &serde_json::Value) {
    let bytes = serde_json::to_vec_pretty(v).unwrap();
    let _ = std::fs::write(p, bytes);
}
"#,
        )],
    },
    Mutation {
        audit: "agent_adapters_are_pluggable",
        why: "a central `match` over adapter tool-id constants, in the CLI",
        blind_spot: "the central-dispatch rule reads agents/mod.rs, actions.rs and tui/ only; crates/cli/src is not in `dispatch_files`",
        files: &[(
            "crates/cli/src/main.rs",
            Mode::Append,
            r#"
/// Sweep: adding an adapter now means editing this table.
pub fn sweep_dispatch(id: &str) -> u64 {
    match id {
        x if x == swamp_core::agents::cline::CLINE_TOOL_ID => 1,
        x if x == swamp_core::agents::codex::CODEX_TOOL_ID => 2,
        _ => 0,
    }
}
"#,
        )],
    },
    Mutation {
        audit: "agent_adapters_read_bounded_headers_only",
        why: "a whole transcript read with the free function `std::io::read_to_string`",
        blind_spot: "`UNBOUNDED_READS` matches `fs :: read_to_string (` and the *method* `. read_to_string (`; the `std::io` free function is neither",
        files: &[(
            "crates/core/src/agents/claude_code.rs",
            Mode::Append,
            r#"
/// Sweep: the whole file, past the header cap.
pub fn sweep_slurp(p: &std::path::Path) -> std::io::Result<String> {
    let f = std::fs::File::open(p)?;
    std::io::read_to_string(f)
}
"#,
        )],
    },
    Mutation {
        audit: "agent_adapters_do_not_traverse",
        why: "an unbounded recursive walk of a tool home, written in the shared module every adapter reads through",
        blind_spot: "`AGENT_NON_ADAPTERS` exempts bounded_io.rs -- the shared capped reader every adapter reads through -- so a recursive walk written there is not an adapter traversal",
        files: &[(
            "crates/core/src/agents/bounded_io.rs",
            Mode::Append,
            r#"
/// Sweep: an unbounded recursive scan, in the shared module every
/// adapter's bounded reads go through.
pub fn sweep_walk_home(home: &std::path::Path, n: &mut usize) {
    let Ok(rd) = std::fs::read_dir(home) else { return };
    for e in rd.flatten() {
        *n += 1;
        if e.path().is_dir() {
            sweep_walk_home(&e.path(), n);
        }
    }
}
"#,
        )],
    },
    Mutation {
        audit: "agent_adapters_are_inspection_only",
        why: "an adapter truncating a tool's file through OpenOptions",
        blind_spot: "`ADAPTER_FORBIDDEN_CALLS` lists the `fs::*` primitives and `Command::new`; `OpenOptions::new().write(true).truncate(true)` destroys the same data and is on none of them",
        files: &[(
            "crates/core/src/agents/opencode.rs",
            Mode::Append,
            r#"
/// Sweep: identification truncates the user's session index.
pub fn sweep_reset_index(p: &std::path::Path) -> std::io::Result<()> {
    std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(p)?;
    Ok(())
}
"#,
        )],
    },
    Mutation {
        audit: "agent_adapters_do_not_emit_content",
        why: "a transcript path put on stderr by a panic message",
        blind_spot: "`EMITTERS` lists the print/log macros and the stdout/stderr handles; `panic!`/`expect` write the same bytes to stderr and are on none of them",
        files: &[(
            "crates/core/src/agents/aider.rs",
            Mode::Append,
            r#"
/// Sweep: leaks the transcript path to stderr.
pub fn sweep_require_session(p: &std::path::Path) {
    if !p.exists() {
        panic!("no session at {}", p.display());
    }
}
"#,
        )],
    },
    Mutation {
        audit: "agent_units_built_through_builder",
        why: "a unit built correctly by the builder and then silently unprotected field by field",
        blind_spot: "the rule inspects *struct-literal sites* and `unprotect_with_reason` arguments; a later `unit.protected = false` is neither",
        files: &[(
            "crates/core/src/agents/cursor.rs",
            Mode::Append,
            r#"
/// Sweep: the builder's protected-by-default, lifted after the fact.
pub fn sweep_unprotect(u: &mut super::CandidateAgentUnit) {
    u.protected = false;
    u.protect_reason = None;
}
"#,
        )],
    },
    Mutation {
        audit: "agent_adapters_are_environment_free",
        why: "the home taken from `$HOME` inside the shared VS Code family module",
        blind_spot: "`AGENT_NON_ADAPTERS` exempts vscode_family.rs and pi_family.rs, which are the per-tool mechanics for Cursor, Windsurf, Continue, Cline, Pi and Oh My Pi",
        files: &[(
            "crates/core/src/agents/vscode_family.rs",
            Mode::Append,
            r#"
/// Sweep: six tools' homes now come from the environment.
pub fn sweep_home() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default())
}
"#,
        )],
    },
    Mutation {
        audit: "agent_adapters_do_not_reach_detectors",
        why: "an adapter naming a detector id constant at item level",
        blind_spot: "the scan is over `sig + body` of *functions*; a `const`, a `type` alias or a struct field naming `locations::` sits inside no function",
        files: &[(
            "crates/core/src/agents/windsurf.rs",
            Mode::Append,
            r#"
/// Sweep: detector identity, at item level, where the audit never looks.
pub const SWEEP_DETECTOR_ID: &str = crate::locations::windsurf::WINDSURF_DETECTOR_ID;
"#,
        )],
    },
    Mutation {
        audit: "agent_adapter_test_contract",
        why: "a new adapter with none of the five required tests -- their names appear only in a doc comment",
        blind_spot: "`defines_running_test` is a raw text search for `fn <name>(`, so a comment, a string or a doc line satisfies the contract",
        files: &[
            (
                "crates/core/src/agents/mod.rs",
                Mode::Append,
                "pub mod sweep_tool;\n",
            ),
            (
                "crates/core/src/agents/sweep_tool.rs",
                Mode::Create,
                r#"//! Sweep: an adapter that proves nothing about itself.
//!
//! Contract tests, for the reader:
//!   fn unknown_format_is_explicit_not_empty() {}
//!   fn canary_content_never_appears_in_output() {}
//!   fn identification_reads_no_more_than_header_cap() {}
//!   fn protected_categories_default_protected() {}
//!   fn project_link_is_declared_or_unresolved_never_basename_guess() {}

pub const SWEEP_TOOL_ID: &str = "sweep-tool";

pub struct Adapter;
"#,
            ),
        ],
    },
    Mutation {
        audit: "detector_ids_only_in_registry",
        why: "wiring that matches on detector id *string literals* instead of the constants",
        blind_spot: "the rule greps for the token `_DETECTOR_ID`; the same coupling written as the literal the constant holds is invisible",
        files: &[(
            "crates/core/src/consumer_wiring.rs",
            Mode::Append,
            r#"
/// Sweep: the wiring table is back, written as literals.
pub fn sweep_recovery_hint(id: &str) -> &'static str {
    match id {
        "cargo" => "cargo fetch",
        "npm" => "npm ci",
        "docker-desktop" => "docker pull",
        _ => "",
    }
}
"#,
        )],
    },
    Mutation {
        audit: "discovery_owned_by_report_pipeline",
        why: "a second discovery pass inside report.rs, in a function that is not `observe_scope`",
        blind_spot: "the caller scan explicitly excludes `DISCOVERY_OWNER.0` (report.rs) as a whole file, so any other function there may run its own pass",
        files: &[(
            "crates/core/src/report.rs",
            Mode::Append,
            r#"
/// Sweep: a second pass over the shared history table.
pub fn sweep_refresh_units(
    scope: &crate::scope::EffectiveScope,
    store_dir: Option<&std::path::Path>,
    observed_at: u64,
) {
    let events = crate::fs_events::EventCoverage::default();
    let _ = crate::external::discover_and_measure(
        scope, store_dir, false, observed_at, 0, 0, &events,
    );
    let _ = crate::agents::discover_and_measure(
        scope, &[], store_dir, false, observed_at, 0, 0, &events,
    );
}
"#,
        )],
    },
    Mutation {
        audit: "no_dead_public_evidence_api",
        why: "a dead public evidence function kept alive by a caller that is itself dead",
        blind_spot: "\"has a non-test caller\" is a one-hop question; nothing asks whether the caller is reachable from the pipeline",
        files: &[
            (
                "crates/core/src/recovery.rs",
                Mode::Append,
                r#"
/// Sweep: a capability the docs can claim and nothing reaches.
pub fn sweep_unreachable_recovery(p: &std::path::Path) -> bool {
    p.exists()
}
"#,
            ),
            (
                "crates/core/src/render.rs",
                Mode::Append,
                r#"
/// Sweep: the caller that makes it look wired. Nothing calls this either.
pub fn sweep_dead_caller(p: &std::path::Path) -> bool {
    crate::recovery::sweep_unreachable_recovery(p)
}
"#,
            ),
        ],
    },
    Mutation {
        audit: "computed_but_not_delivered",
        why: "an evidence field on a delivered surface, typed so the word `Evidence` never appears",
        blind_spot: "the rule is scoped to fields whose declared type *mentions* `Evidence`; the same promise typed `Vec<Fact>` is unaudited",
        files: &[(
            "crates/core/src/external.rs",
            Mode::Append,
            r#"
/// Sweep: promised in the docs, never computed, never rendered.
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct SweepNestedRow {
    pub path: std::path::PathBuf,
    pub decision_facts: Vec<crate::evidence::FactKind>,
}

pub fn sweep_nested_row(path: std::path::PathBuf) -> SweepNestedRow {
    SweepNestedRow {
        path,
        decision_facts: Vec::new(),
    }
}
"#,
        )],
    },
    Mutation {
        audit: "coverage_changes_are_not_storage_changes",
        why: "an unguarded regrowth bump written with `+=`",
        blind_spot: "the tombstone needle is the literal `regrowth_count + 1`; the token text of `+=` is `regrowth_count += 1`, which does not contain it",
        files: &[(
            "crates/core/src/growth.rs",
            Mode::Append,
            r#"
/// Sweep: scores a regrowth for every row, covered or not.
fn sweep_score_regrowth(rows: &mut [StoredRow]) {
    for row in rows.iter_mut() {
        row.regrowth_count += 1;
    }
}
"#,
        )],
    },
    Mutation {
        audit: "activity_and_consumer_evidence_have_limits",
        why: "an `Evidence::unknown` whose reason is an empty constant",
        blind_spot: "the empty-reason rule looks for the literal `\"\"` in the call's arguments; a named constant holding the empty string is invisible",
        files: &[(
            "crates/core/src/activity.rs",
            Mode::Append,
            r#"
const SWEEP_NO_REASON: &str = "";

/// Sweep: "not observed", with nothing saying why.
pub fn sweep_unknown_fact(observed_at: u64) -> Evidence {
    Evidence::unknown(
        FactKind::Activity,
        FactSubtype::Modified,
        EvidenceSource::FilesystemMetadata {
            detail: String::new(),
        },
        observed_at,
        SWEEP_NO_REASON,
    )
}
"#,
        )],
    },
];

// ---------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

const COPIED: &[&str] = &["crates", ".oh", "docs", "scripts", "Cargo.toml"];

fn copy_tree(from: &Path, to: &Path) {
    if from.is_file() {
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        std::fs::copy(from, to).unwrap();
        return;
    }
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let name = e.file_name();
        if name == "target" || name == ".git" {
            continue;
        }
        copy_tree(&e.path(), &to.join(&name));
    }
}

fn fresh_copy(tmp: &Path) -> PathBuf {
    let work = tmp.join("workspace");
    for entry in COPIED {
        let from = repo_root().join(entry);
        if from.exists() {
            copy_tree(&from, &work.join(entry));
        }
    }
    work
}

/// Applies one mutation, runs its audit, restores every file, and says
/// whether the audit rejected it.
fn run_one(work: &Path, m: &Mutation) -> Result<bool, String> {
    let Some((_, audit)) = AUDITS.iter().find(|(n, _)| *n == m.audit) else {
        return Err(format!("no audit named `{}` is registered", m.audit));
    };
    let mut originals: Vec<(PathBuf, Option<String>)> = Vec::new();
    for (rel, mode, body) in m.files {
        let target = work.join(rel);
        let before = std::fs::read_to_string(&target).ok();
        let after = match mode {
            Mode::Append => format!("{}\n{body}", before.clone().unwrap_or_default()),
            Mode::Create => (*body).to_string(),
        };
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, after).unwrap();
        originals.push((target, before));
    }
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| audit(work)));
    for (path, before) in originals {
        match before {
            Some(text) => std::fs::write(&path, text).unwrap(),
            None => {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    match outcome {
        Err(_) => Err("the audit panicked on the mutated tree".into()),
        // `Ok(Ok(()))` == the audit accepted the mutation == a slip.
        Ok(Ok(())) => Ok(false),
        Ok(Err(_)) => Ok(true),
    }
}

/// Every registered audit gets one mutation not already in
/// `tests/mutations/`, and must reject it.
#[test]
fn every_audit_rejects_a_mutation_outside_its_own_corpus() {
    let registered: Vec<&str> = AUDITS.iter().map(|(n, _)| *n).collect();
    let covered: Vec<&str> = M.iter().map(|m| m.audit).collect();
    let uncovered: Vec<&&str> = registered.iter().filter(|n| !covered.contains(n)).collect();
    assert!(
        uncovered.is_empty(),
        "the sweep must cover every registered audit; missing: {uncovered:?}"
    );

    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .clamp(1, 8)
        .min(M.len());
    let tmp = tempfile::tempdir().expect("tempdir");
    let shards: Vec<Vec<&Mutation>> = (0..workers)
        .map(|w| {
            M.iter()
                .enumerate()
                .filter(|(i, _)| i % workers == w)
                .map(|(_, m)| m)
                .collect()
        })
        .collect();

    let rows: Vec<(String, bool, String, String)> = std::thread::scope(|scope| {
        let handles: Vec<_> = shards
            .into_iter()
            .enumerate()
            .map(|(w, shard)| {
                let dir = tmp.path().join(format!("w{w}"));
                scope.spawn(move || {
                    std::fs::create_dir_all(&dir).unwrap();
                    let work = fresh_copy(&dir);
                    shard
                        .into_iter()
                        .map(|m| match run_one(&work, m) {
                            Ok(rejected) => (
                                m.audit.to_string(),
                                rejected,
                                m.why.to_string(),
                                m.blind_spot.to_string(),
                            ),
                            Err(e) => (
                                m.audit.to_string(),
                                false,
                                format!("{} [{e}]", m.why),
                                m.blind_spot.to_string(),
                            ),
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("worker"))
            .collect()
    });

    let mut rows = rows;
    rows.sort();
    let mut table = String::new();
    for (audit, rejected, why, blind) in &rows {
        table.push_str(&format!(
            "{:8} {audit}\n    mutation:   {why}\n    blind spot: {blind}\n",
            if *rejected { "REJECTED" } else { "SLIPPED" }
        ));
    }
    let slipped: Vec<&String> = rows
        .iter()
        .filter(|(_, ok, _, _)| !*ok)
        .map(|(a, _, _, _)| a)
        .collect();
    assert!(
        slipped.is_empty(),
        "{} of {} audits accepted a harmful mutation that is not in their own corpus:\n{}\nslipped: {:?}",
        slipped.len(),
        rows.len(),
        table,
        slipped
    );
}

/// Materialises every mutation into a throwaway copy of the workspace
/// so the reviewer can `cargo check` it: "compiling, harmful mutation"
/// is the standard the sweep is held to, and a mutation that does not
/// compile proves nothing.
///
/// Ignored by default; run with
/// `SWEEP_MATERIALISE=<dir> cargo test ... -- --ignored materialise`.
#[test]
#[ignore]
fn materialise_mutations_for_compile_check() {
    let Ok(dest) = std::env::var("SWEEP_MATERIALISE") else {
        return;
    };
    let dest = PathBuf::from(dest);
    let _ = std::fs::remove_dir_all(&dest);
    std::fs::create_dir_all(&dest).unwrap();
    for entry in COPIED {
        let from = repo_root().join(entry);
        if from.exists() {
            copy_tree(&from, &dest.join(entry));
        }
    }
    let skip: &[&str] = &std::env::var("SWEEP_SKIP")
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect::<Vec<_>>()
        .iter()
        .map(|s| Box::leak(s.clone().into_boxed_str()) as &str)
        .collect::<Vec<_>>();
    for m in M {
        if skip.contains(&m.audit) {
            continue;
        }
        for (rel, mode, body) in m.files {
            let target = dest.join(rel);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            let text = match mode {
                Mode::Append => {
                    format!(
                        "{}\n{body}",
                        std::fs::read_to_string(&target).unwrap_or_default()
                    )
                }
                Mode::Create => (*body).to_string(),
            };
            std::fs::write(&target, text).unwrap();
        }
    }
    eprintln!("materialised {} mutations into {}", M.len(), dest.display());
}
