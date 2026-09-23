#!/usr/bin/env bash
# The full tier: everything `scripts/check.sh` runs, then the heavy
# harnesses, each exactly once -- the compile-fail cases (every retired
# rule's shortcut against the production API), the mutation sweep (every
# corpus, re-review and operator mutation applied to a copy of the
# workspace) and the cost test (process-global work counters and a PATH
# spawn shim, so single-threaded and alone). What CI runs
# (`.github/workflows/check-full.yml`).
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
target=()
if [ -n "${SWAMP_TARGET_DIR:-}" ]; then
  target=(--target-dir "$SWAMP_TARGET_DIR")
fi
step() { printf '\n== %s (%s)\n' "$1" "$(date +%H:%M:%S)"; }

"$here/check.sh"

step compile-fail
cargo test -p swamp-source-audit --locked "${target[@]}" --test compile_fail -- --ignored

step mutation-sweep
cargo test -p swamp-source-audit --locked "${target[@]}" --test mutation_sweep -- --ignored

step cost
cargo test -p swamp-core --locked "${target[@]}" \
  --test reviewer_cost_measurement_stack3 \
  -- --test-threads=1
step done
