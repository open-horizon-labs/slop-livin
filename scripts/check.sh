#!/usr/bin/env bash
set -euo pipefail
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -q -p swamp-source-audit

# Named explicitly so a rename cannot silently drop them: these are the
# runtime halves of the 2026-09-21 review guardrails, and an AST audit
# alone does not prove a refusal actually refuses.
cargo test -p swamp-core \
  --test reviewer_counterexamples \
  --test reviewer_counterexamples_123 \
  --test reviewer_counterexamples_stack2 \
  --test coverage_changes_are_not_storage_changes \
  --test explicit_root_scope_exclusions \
  --test nested_artifact_evidence_is_delivered \
  --test upstream_citations_are_checked \
  --test execution_rechecks \
  --test shared_history_ownership \
  --test store_contents_are_allowlisted \
  --test incremental_external_and_agent_measurement \
  --test agent_matrix_matches_docs \
  --test agent_storage_validation

# The runtime halves of the section 18 build-adapter guardrails. Named
# for the same reason as the list above: the AST audits check the shape
# of the code, and only these prove that an unchanged container really
# reads nothing, that a shared store is charged once, that the published
# capability table matches the registry, and that no identified unit
# renders a verdict.
cargo test -p swamp-core \
  --test build_adapter_contract \
  --test build_adapter_cost

# `--test-threads=1` here and nowhere else. This test measures through
# the *process-global* work counters (`work_counters::reset` +
# `snapshot`, which is what a whole-observation cost report needs) and
# installs PATH shims to count subprocess spawns, so a sibling test
# running beside it would be measured as its work. Every other test that
# counts work uses `work_counters::measured`, whose sink is scoped to the
# calling thread and the pools it starts, and therefore needs nothing
# here -- including `reviewer_counterexamples_stack2`, which used to be
# in this list because its spawn count came from a process-wide PATH
# shim and a shared log file.
cargo test -p swamp-core \
  --test reviewer_cost_measurement_stack2 \
  -- --test-threads=1
cargo test -p swamp-tui --test scope_preserving_refresh \
  --test reviewer_counterexamples_stack2_tui

# The audit machinery's own mutation corpus: every slip the 2026-09-22
# background sweep found, applied to a copy of the real workspace, with
# the audit that let it through required to reject it
# (GUARDRAILS_SPEC.md section 17). Named explicitly because an audit
# nobody has shown rejects anything is the state all 42 were in.
cargo test -p swamp-source-audit --test mutation_corpus

# These checks intentionally fail obvious safety regressions in source
# review: a raw recursive delete, and verdict vocabulary the tool never
# applies to a path ("safe", "unused", "stale" are for the human to
# conclude, never for the tool to assert).
#
# `grep`, not `rg`: this has to run wherever CI or a contributor runs it,
# and ripgrep is not a dependency of this repo.
#
# Comment lines are excluded because the rule is written down in the
# source that enforces it: render.rs and filter.rs each explain the
# verdict-vocabulary ban using the very words it bans, so matching
# comments made this check impossible to pass. A trailing comment on a
# line of code is still reported -- only a line that is nothing but a
# comment is skipped.
violations=$(
  grep -rnE 'rm[[:space:]]+-rf|std::fs::remove_dir_all|"safe"|"unused"|"stale"' \
    crates/core/src crates/cli/src |
    awk '{ code = $0; sub(/^[^:]*:[0-9]+:/, "", code); if (code !~ /^[[:space:]]*\/\//) print }'
) || true
if [ -n "$violations" ]; then
  echo "$violations" >&2
  echo 'source audit: forbidden destructive/verdict shortcut found' >&2
  exit 1
fi

# Protection state is a Result and no caller may discard it: the AST
# audit checks the call sites it can see, this catches the shape
# anywhere it appears (.oh/guardrails/protection-fails-closed.md).
protect_violations=$(
  grep -rn 'unwrap_or_default()' crates/core/src crates/cli/src crates/tui/src |
    grep 'protect' |
    awk '{ code = $0; sub(/^[^:]*:[0-9]+:/, "", code); if (code !~ /^[[:space:]]*\/\//) print }'
) || true
if [ -n "$protect_violations" ]; then
  echo "$protect_violations" >&2
  echo 'source audit: protection state must fail closed, never unwrap_or_default()' >&2
  exit 1
fi

# JSON is an output format and a format for small control files, never a
# data store (.oh/guardrails/json-persistence-is-allowlisted.md). Belt
# and braces over the AST audit: a serializer call in a file that is not
# an allow-listed writer, and is not printing, fails here.
json_writer_files='actions.rs|grants.rs|ledger.rs|schedule.rs|scope.rs|agents/mod.rs|cargo_cleanup.rs|growth.rs|report.rs|assoc_store.rs|agent_json.rs|app.rs|main.rs|render.rs|docker.rs|github.rs|store.rs|recovery.rs|evidence.rs|entities.rs'
json_violations=$(
  grep -rnE 'serde_json::to_(vec|string|writer)' crates/core/src crates/cli/src crates/tui/src |
    grep -vE "/($json_writer_files):" |
    awk '{ code = $0; sub(/^[^:]*:[0-9]+:/, "", code); if (code !~ /^[[:space:]]*\/\//) print }'
) || true
if [ -n "$json_violations" ]; then
  echo "$json_violations" >&2
  echo 'source audit: JSON serialization outside the allow-listed writers' >&2
  exit 1
fi

# The spawn counter cannot be allowed to rot: a `Command::new` added
# without its `record_spawn()` would make
# `a_disabled_detector_must_not_probe_its_tool` pass while the spawn it
# guards against happens. The AST audit's resolver does not follow
# `Command` builders, so this is the check.
missing_spawn_counts=$(
  grep -rn 'Command::new(' crates/core/src |
    awk '{ code = $0; sub(/^[^:]*:[0-9]+:/, "", code); if (code !~ /^[[:space:]]*\/\//) print }' |
    while IFS= read -r hit; do
      file=${hit%%:*}
      rest=${hit#*:}
      line=${rest%%:*}
      prev=$((line - 1))
      if ! sed -n "${prev}p" "$file" | grep -q 'record_spawn()'; then
        echo "$hit"
      fi
    done
) || true
if [ -n "$missing_spawn_counts" ]; then
  echo "$missing_spawn_counts" >&2
  echo 'source audit: every Command::new in swamp-core must be preceded by work_counters::record_spawn()' >&2
  exit 1
fi

test -x scripts/check.sh
