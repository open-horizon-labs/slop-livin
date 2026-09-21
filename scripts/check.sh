#!/usr/bin/env bash
set -euo pipefail
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -q -p swamp-source-audit

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

test -x scripts/check.sh
