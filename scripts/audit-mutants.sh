#!/bin/zsh
# Mutation tests for the walker audits: apply a wrong-but-plausible change
# to walk.rs / attribution.rs, run the two audits, revert. Every mutant
# must produce a FAIL from the audit that owns the rule; a mutant that
# produces only `ok` means that audit has stopped proving anything.
#
# Every mutation is anchored on exact text and asserts that it applied.
# An earlier version patched by line number and named a function that had
# since been deleted, so three mutants silently did nothing and the run
# looked green: a mutation test that cannot fail is worth less than none.
set -e
cd "$(dirname "$0")/.."
if ! git diff --quiet crates/core/src/walk.rs crates/core/src/attribution.rs; then
  echo "walk.rs/attribution.rs have uncommitted changes; commit or stash first" >&2
  exit 2
fi

survivors=0

mutate() { # name file old new expect_audit
  local name="$1" file="$2" old="$3" new="$4" expect="$5"
  python3 - "$file" "$old" "$new" <<'PY'
import sys
path, old, new = sys.argv[1], sys.argv[2], sys.argv[3]
s = open(path).read()
n = s.count(old)
assert n >= 1, f"mutation anchor not found in {path}:\n{old}"
open(path, "w").write(s.replace(old, new, 1))
PY
  echo "$name"
  local out
  out="$(cargo run -q -p swamp-source-audit -- folding_only_for_artifacts symlinks_never_followed 2>&1 | grep -E '^(ok|FAIL)')"
  echo "$out" | sed 's/^/    /'
  if ! echo "$out" | grep -q "FAIL  $expect"; then
    echo "    ^^ SURVIVED: expected $expect to fail" >&2
    survivors=$((survivors + 1))
  fi
  git checkout -q crates/core/src/walk.rs crates/core/src/attribution.rs
}

mutate "M1 size job: symlink_metadata -> metadata (follows the link)" \
  crates/core/src/walk.rs \
  'let Ok(meta) = fs::symlink_metadata(entry.path()) else {' \
  'let Ok(meta) = fs::metadata(entry.path()) else {' \
  symlinks_never_followed

mutate "M2 size job: delete the symlink discard from the loop" \
  crates/core/src/walk.rs \
  '        if ft.is_symlink() {
            symlink_count += 1;
            continue;
        }
' '' \
  symlinks_never_followed

mutate "M3 walk job: decide is_dir on entry.path() (stats through the link)" \
  crates/core/src/walk.rs \
  '} else if ft.is_dir() {' \
  '} else if entry.path().is_dir() {' \
  symlinks_never_followed

mutate "M4 discovery: drop the symlink half of the combined guard" \
  crates/core/src/walk.rs \
  'if file_type.is_symlink() || !file_type.is_dir() {' \
  'if !file_type.is_dir() {' \
  symlinks_never_followed

mutate "M5 walk job: fold under a made-up kind instead of classify_at" \
  crates/core/src/walk.rs \
  'if let Some(kind) = classify_at(&path, &name) {' \
  'if let Some(kind) = Some(ArtifactKind::Cache) {' \
  folding_only_for_artifacts

mutate "M6 classify_at: fold every unknown name as Cache" \
  crates/core/src/attribution.rs \
  '        .find(|(n, _, markers)| *n == name && has_marker(parent, markers))
        .map(|(_, k, _)| k.clone())
}' \
  '        .find(|(n, _, markers)| *n == name && has_marker(parent, markers))
        .map(|(_, k, _)| k.clone())
        .or(Some(ArtifactKind::Cache))
}' \
  folding_only_for_artifacts

mutate "M7 serial walker: record an artifact without classifying" \
  crates/core/src/attribution.rs \
  'if let Some(kind) = path.parent().and_then(|parent| classify_at(parent, name)) {' \
  'if let Some(kind) = Some(ArtifactKind::BuildOutput) {' \
  folding_only_for_artifacts

if [ "$survivors" -gt 0 ]; then
  echo "\n$survivors mutant(s) survived: the audits do not cover them." >&2
  exit 1
fi
echo "\nall mutants killed by the audit that owns the rule."
