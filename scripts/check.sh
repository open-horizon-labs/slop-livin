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
  --test execution_rechecks \
  --test shared_history_ownership \
  --test store_contents_are_allowlisted \
  --test incremental_external_and_agent_measurement \
  -- --test-threads=1
cargo test -p swamp-tui --test scope_preserving_refresh

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

test -x scripts/check.sh
