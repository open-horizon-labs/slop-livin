#!/usr/bin/env bash
set -euo pipefail
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -q -p slop-livin-source-audit
# These checks intentionally fail obvious safety regressions in source review.
if rg -n 'rm[[:space:]]+-rf|std::fs::remove_dir_all|"safe"|"unused"|"stale"' crates/core/src crates/cli/src crates/mcp/src; then
  echo 'source audit: forbidden destructive/verdict shortcut found' >&2
  exit 1
fi
test -x scripts/check.sh
