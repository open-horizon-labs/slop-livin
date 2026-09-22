//! One named audit per guardrail in `.oh/guardrails/`. Every rule lives
//! in `rules/` and is written only against the whole-program model
//! (`program.rs`): no rule names a file to scan or lists this
//! workspace's own function names as its coverage. `adr_validation`
//! closes the loop: every hard guardrail names a registered audit (or a
//! dated `none` with runtime tests), every audit's guardrail names a
//! mutation-corpus fixture, and every ADR `validate:` entry resolves.

use crate::rules::{adapters, bus, evidence, execution, meta, scope, store, tui, walk};

/// Rules that run as tests rather than as registered audits (see each
/// rule's guardrail for why).
pub const TEST_ONLY_RULES: &[(&str, Audit)] = &[("every_spawn_is_counted", execution::every_spawn_is_counted)];
use std::path::Path;

pub type Audit = fn(&Path) -> Result<(), String>;

pub const AUDITS: &[(&str, Audit)] = &[
    ("tui_actions_off_event_thread", tui::tui_actions_off_event_thread),
    ("one_byte_formatter", walk::one_byte_formatter),
    ("legacy_invariants", walk::legacy_invariants),
    ("fsevents_before_full_walk", walk::fsevents_before_full_walk),
    ("column_store_parquet_zstd", walk::column_store_parquet_zstd),
    ("reverse_delta_current_plus_deltas", walk::reverse_delta_current_plus_deltas),
    ("scheduled_refresh_launchagent", walk::scheduled_refresh_launchagent),
    ("folding_only_for_artifacts", walk::folding_only_for_artifacts),
    ("symlinks_never_followed", walk::symlinks_never_followed),
    ("incremental_walk_only_changed_subtrees", walk::incremental_walk_only_changed_subtrees),
    ("walk_optimized_parallel_pool", walk::walk_optimized_parallel_pool),
    ("dir_mtime_int32_minutes", walk::dir_mtime_int32_minutes),
    ("agent_interface_facts_not_verdicts", evidence::agent_interface_facts_not_verdicts),
    ("human_only_authorization", execution::human_only_authorization),
    ("no_consumer_knows_other_consumers", bus::no_consumer_knows_other_consumers),
    ("static_registration_only", bus::static_registration_only),
    ("all_report_paths_through_bus", bus::all_report_paths_through_bus),
    ("extractors_are_pluggable", bus::extractors_are_pluggable),
    ("event_bus_pluggable_consumers", bus::event_bus_pluggable_consumers),
    ("adr_validation", meta::adr_validation),
    ("execution_sinks_recheck_live_state", execution::execution_sinks_recheck_live_state),
    ("protection_fails_closed", execution::protection_fails_closed),
    ("discovery_consumes_effective_scope", scope::discovery_consumes_effective_scope),
    ("explicit_only_scope_when_defaults_false", scope::explicit_only_scope_when_defaults_false),
    ("history_sweeps_are_owned", store::history_sweeps_are_owned),
    ("no_second_traversal_on_report_path", walk::no_second_traversal_on_report_path),
    ("occupancy_is_tristate_at_sinks", execution::occupancy_is_tristate_at_sinks),
    ("tui_refresh_preserves_scope", scope::tui_refresh_preserves_scope),
    ("store_data_is_parquet_not_json_sidecars", store::store_data_is_parquet_not_json_sidecars),
    ("json_persistence_is_allowlisted", store::json_persistence_is_allowlisted),
    ("agent_adapters_are_pluggable", adapters::agent_adapters_are_pluggable),
    ("agent_adapters_read_bounded_headers_only", adapters::agent_adapters_read_bounded_headers_only),
    ("agent_adapters_do_not_traverse", adapters::agent_adapters_do_not_traverse),
    ("agent_adapters_are_inspection_only", adapters::agent_adapters_are_inspection_only),
    ("agent_adapters_do_not_emit_content", adapters::agent_adapters_do_not_emit_content),
    ("agent_units_built_through_builder", adapters::agent_units_built_through_builder),
    ("agent_adapters_are_environment_free", adapters::agent_adapters_are_environment_free),
    ("agent_adapters_do_not_reach_detectors", adapters::agent_adapters_do_not_reach_detectors),
    ("agent_adapter_test_contract", adapters::agent_adapter_test_contract),
    ("detector_ids_only_in_registry", scope::detector_ids_only_in_registry),
    ("discovery_owned_by_report_pipeline", scope::discovery_owned_by_report_pipeline),
    ("no_dead_public_evidence_api", evidence::no_dead_public_evidence_api),
    ("computed_but_not_delivered", evidence::computed_but_not_delivered),
    ("coverage_changes_are_not_storage_changes", store::coverage_changes_are_not_storage_changes),
    ("activity_and_consumer_evidence_have_limits", evidence::activity_and_consumer_evidence_have_limits),
];
