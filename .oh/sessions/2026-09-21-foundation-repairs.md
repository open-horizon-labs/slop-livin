# Foundation repairs after the 2026-09-21 independent reviews

Two independent adversarial reviews blocked the stack:
`REVIEW.md` (PRs #116-#122) with seven falsifying tests, and
`REVIEW-123.md` (PR #123) with ten more. Both are right, and both found
the same *shape* of problem rather than seventeen unrelated bugs:

- an approval was spent against whatever is at a path now, not against
  what a human reviewed;
- a discovery pass re-decided what was in scope instead of being told;
- a history sweep assumed every row it did not see had disappeared;
- a capability was documented as delivered while nothing called it.

Adapter-local patches would have fixed each counterexample and left the
shape. This work repairs the shared layers instead.

## Guardrails first, then fix

At the user's instruction the enforcement landed **before** the repairs.
Commit 1 adds 22 AST audits (sections 1-16 of the guardrail spec), each
with a `.oh/guardrails/<id>.md` and mutation tests proving both
directions -- the repaired shape passes, and a synthetic violation is
rejected -- and leaves `cargo run -p swamp-source-audit` failing.

The baseline below is the audits' own enumeration of violations in the
reviewed code (`stack/08-decision-evidence`, f0181bb), produced with the
new `--root` flag against a pristine `git archive` export in the
scratchpad so a working tree already being repaired could not flatter
the numbers:

```sh
cargo run -q -p swamp-source-audit -- --root <pristine-export>
```

**18 of the 22 new audits fail. 4 already pass**
(`agent_adapters_are_inspection_only`,
`agent_adapters_do_not_emit_content`,
`agent_adapters_are_environment_free`,
`agent_adapters_do_not_reach_detectors`) -- worth recording, because it
means those four rules describe how the adapters were already written
and now cannot drift.

## Baseline: the 18 failing audits

```
FAIL  execution_sinks_recheck_live_state: crates/core/src/actions.rs::execute_agent_cache_trash performs `:: rename (` without recheck::reviewed_snapshot + recheck::live_protection + recheck::member_occupancy first -- every sink that moves user data rechecks identity, protection and occupancy before its first destructive call (see .oh/guardrails/execution-sinks-recheck-live-state.md)
FAIL  protection_fails_closed: agents/mod.rs::is_human_protected tests protection in only one direction (candidate-under-protected: true, protected-under-candidate: false). Protecting `debug/log.txt` must also stop removing `debug/` -- see the review's protected_descendant_must_prevent_parent_cache_proposal counterexample
FAIL  discovery_consumes_effective_scope: crates/core/src/external.rs::discover_and_measure reads `LocationStatus :: Resolved`: discovery must consume `EffectiveScope::authorized_roots()`, not raw detector candidates (the review's excluded_agent_home_must_not_be_scanned counterexample)
FAIL  explicit_only_scope_when_defaults_false: scope.rs: `ScanConfig` has no `enabled_detectors` allow-list field
FAIL  history_sweeps_are_owned: growth.rs does not define `ObservationOwnership`
FAIL  no_second_traversal_on_report_path: crates/core/src/external.rs::discover_and_measure traverses with `:: read_dir (`: the ordinary report path traverses only in the folded walk; use folded rows, cached identification, or the bounded `locations::shallow_list`
FAIL  occupancy_is_tristate_at_sinks: occupancy.rs: `OccupancyState` has no `Free` variant; a sink cannot distinguish "nothing open" from "could not look"
FAIL  tui_refresh_preserves_scope: crates/tui/src/app.rs::observe_live refreshes through `report_full_mode_with_source (`, which takes no EffectiveScope: excluded subtrees and pruned external locations reappear on refresh. Use `report_scope_with_parts`/`report_scope_with_source` (or a TUI wrapper that forwards the scope)
FAIL  store_data_is_parquet_not_json_sidecars: crates/core/src/consumer_wiring.rs::declaration_cache_path joins the store file "toolchain_declarations_cache.json", which is not one of the small control files. Per-unit/per-row data belongs in the Parquet current + reverse-delta store, not a JSON sidecar (handoff: no parallel database or JSON artifact cache)
FAIL  json_persistence_is_allowlisted: crates/core/src/consumer_wiring.rs::save_json serializes JSON into a file write without a JSON_WRITE_ALLOWLIST entry. JSON is an output format and a format for a fixed set of small control files; data belongs in the Parquet store
FAIL  agent_adapters_are_pluggable: crates/core/src/agents/mod.rs::identify_for_tool matches on `AIDER_TOOL_ID`: adapter dispatch goes through `agents::Registry`, never a central tool-id match
FAIL  agent_adapters_read_bounded_headers_only: crates/core/src/agents/copilot_cli.rs::declared_path_in_dir reads file contents with `fs :: read_to_string (`: an adapter's only content access is `agents::bounded_io::read_header(path, max_bytes)`, capped, never a whole file (privacy is a hard contract, not a convention)
FAIL  agent_adapters_do_not_traverse: crates/core/src/agents/aider.rs::identify traverses with `:: read_dir (`: directory structure reaches an adapter through the folded walk rows in its context, or through the capped `locations::shallow_list`
ok    agent_adapters_are_inspection_only
ok    agent_adapters_do_not_emit_content
FAIL  agent_units_built_through_builder: crates/core/src/agents/aider.rs::identify builds a `CandidateAgentUnit { .. }` struct literal: units are built with `AgentUnitBuilder::new(tool, category, path)`, whose constructor applies protected-by-default categories that a literal can silently omit
ok    agent_adapters_are_environment_free
ok    agent_adapters_do_not_reach_detectors
FAIL  agent_adapter_test_contract: every adapter proves the same five things about itself; these do not:
  crates/core/src/agents/aider.rs: missing unknown_format_is_explicit_not_empty, canary_content_never_appears_in_output, identification_reads_no_more_than_header_cap, protected_categories_default_protected, project_link_is_declared_or_unresolved_never_basename_guess
  crates/core/src/agents/claude_code.rs: missing unknown_format_is_explicit_not_empty, canary_content_never_appears_in_output, identification_reads_no_more_than_header_cap, protected_categories_default_protected, project_link_is_declared_or_unresolved_never_basename_guess
  crates/core/src/agents/cline.rs: missing unknown_format_is_explicit_not_empty, canary_content_never_appears_in_output, identification_reads_no_more_than_header_cap, protected_categories_default_protected, project_link_is_declared_or_unresolved_never_basename_guess
  crates/core/src/agents/codex.rs: missing unknown_format_is_explicit_not_empty, canary_content_never_appears_in_output, identification_reads_no_more_than_header_cap, protected_categories_default_protected, project_link_is_declared_or_unresolved_never_basename_guess
  crates/core/src/agents/codex_desktop.rs: missing unknown_format_is_explicit_not_empty, canary_content_never_appears_in_output, identification_reads_no_more_than_header_cap, protected_categories_default_protected, project_link_is_declared_or_unresolved_never_basename_guess
  crates/core/src/agents/continue_dev.rs: missing unknown_format_is_explicit_not_empty, canary_content_never_appears_in_output, identification_reads_no_more_than_header_cap, protected_categories_default_protected, project_link_is_declared_or_unresolved_never_basename_guess
  crates/core/src/agents/copilot_cli.rs: missing unknown_format_is_explicit_not_empty, canary_content_never_appears_in_output, identification_reads_no_more_than_header_cap, protected_categories_default_protected, project_link_is_declared_or_unresolved_never_basename_guess
  crates/core/src/agents/cursor.rs: missing unknown_format_is_explicit_not_empty, canary_content_never_appears_in_output, identification_reads_no_more_than_header_cap, protected_categories_default_protected, project_link_is_declared_or_unresolved_never_basename_guess
  crates/core/src/agents/gemini_cli.rs: missing unknown_format_is_explicit_not_empty, canary_content_never_appears_in_output, identification_reads_no_more_than_header_cap, protected_categories_default_protected, project_link_is_declared_or_unresolved_never_basename_guess
  crates/core/src/agents/oh_my_pi.rs: missing unknown_format_is_explicit_not_empty, canary_content_never_appears_in_output, identification_reads_no_more_than_header_cap, protected_categories_default_protected, project_link_is_declared_or_unresolved_never_basename_guess
  crates/core/src/agents/opencode.rs: missing unknown_format_is_explicit_not_empty, canary_content_never_appears_in_output, identification_reads_no_more_than_header_cap, protected_categories_default_protected, project_link_is_declared_or_unresolved_never_basename_guess
  crates/core/src/agents/pi.rs: missing unknown_format_is_explicit_not_empty, canary_content_never_appears_in_output, identification_reads_no_more_than_header_cap, protected_categories_default_protected, project_link_is_declared_or_unresolved_never_basename_guess
  crates/core/src/agents/roo_code.rs: missing unknown_format_is_explicit_not_empty, canary_content_never_appears_in_output, identification_reads_no_more_than_header_cap, protected_categories_default_protected, project_link_is_declared_or_unresolved_never_basename_guess
  crates/core/src/agents/windsurf.rs: missing unknown_format_is_explicit_not_empty, canary_content_never_appears_in_output, identification_reads_no_more_than_header_cap, protected_categories_default_protected, project_link_is_declared_or_unresolved_never_basename_guess
FAIL  detector_ids_only_in_registry: crates/core/src/consumer_wiring.rs::installed_versions_for names a `*_DETECTOR_ID` constant: consumers match on detector-declared capabilities (`Detector::manager_conventions()`, `Detector::recovery_hint()`), never on ids, so adding a detector never means editing a wiring table
FAIL  discovery_owned_by_report_pipeline: crates/core/src/external.rs::discover_and_measure is not `pub(crate)`: one observation owns discovery, and CLI/TUI take units from the report rather than running their own pass
FAIL  no_dead_public_evidence_api: these public evidence-API functions have no non-test caller; wire them into the live pipeline where their issue requires, or delete them together with the docs and CHANGELOG claims that say they are delivered:
  crates/core/src/evidence.rs::expires_after_with_coverage
  crates/core/src/evidence.rs::of_kind
  crates/core/src/evidence.rs::stale
  crates/core/src/activity.rs::access_time_evidence
  crates/core/src/activity.rs::docker_last_used_evidence
  crates/core/src/occupancy.rs::docker_running_container_evidence
  crates/core/src/occupancy.rs::manager_lock_evidence
  crates/core/src/occupancy.rs::simulator_booted_evidence
  crates/core/src/recovery.rs::maven_artifact_recovery
  crates/core/src/recovery.rs::toolchain_installation_recovery
  crates/core/src/reclaimability.rs::apfs_clone_or_snapshot_bound
  crates/core/src/reclaimability.rs::sparse_file_accounting
  crates/core/src/reclaimability.rs::estimate_selection
  crates/core/src/reclaimability.rs::observed_free_space_change
  crates/core/src/external_associations.rs::join_cache_entry
  crates/core/src/toolchain_declarations.rs::conflicting_tools
18 audit(s) failed
```

## Precision, not weakness

Four audits were tightened while writing them, in ways that make them
*more* specific rather than more permissive; each is recorded because a
reader should be able to tell a precision fix from a loophole:

- `execution_sinks_recheck_live_state` skips three named
  store-bookkeeping functions (`save_plan`, `write_restore_manifest`,
  `write_atomic`), whose `rename` publishes a control file by
  temp-then-rename and never touches a path the user asked about. The
  spec's own allow-list named this category.
- `store_data_is_parquet_not_json_sidecars` ignores literals containing
  spaces (prose ending in a filename), bare extensions (`".json"` as a
  suffix test) and files under `agents/`/`locations/`, which describe
  *other tools'* layouts rather than swamp's store.
- `json_persistence_is_allowlisted` follows the serialized value through
  `;`-separated segments and `let` bindings instead of asking whether a
  function contains both a serializer and a writer somewhere. A CLI
  `main` that prints JSON in one arm and writes a TOML config in another
  is not persistence.
- `history_sweeps_are_owned` checks the *signature* for an
  `ObservationOwnership` parameter rather than a mention in the body, so
  deleting the guard cannot also delete the evidence that the parameter
  was ever required.

## One audit is expected to keep failing, and why

`discovery_owned_by_report_pipeline` requires
`external::discover_and_measure` and `agents::discover_and_measure` to be
`pub(crate)`. The reviewers' own mandatory, unchanged
`crates/core/tests/reviewer_counterexamples.rs` calls both from an
integration test, i.e. from outside the crate, so `pub(crate)` would stop
the required test file compiling. The two requirements are directly
incompatible as written.

Per the repair instruction the audit is **not** weakened to pass. Its
call-site half (no CLI/TUI caller) is enforceable today and is being
repaired; the visibility half stays failing until either the reviewers'
tests move in-crate or the rule is restated as "no CLI/TUI caller". See
`.oh/guardrails/discovery-owned-by-report-pipeline.md`.
