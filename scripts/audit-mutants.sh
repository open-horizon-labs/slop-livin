#!/bin/zsh
# Mutation tests for the walker audits: apply a wrong-but-plausible change
# to walk.rs / attribution.rs, run the two audits, revert. Every mutant
# must print a FAIL from the audit that owns the rule; a mutant that
# prints only `ok` means the audit has stopped proving anything. Run from
# a clean tree (it uses `git checkout` to revert).
set -e
cd "$(dirname "$0")/.."
if ! git diff --quiet crates/core/src/walk.rs crates/core/src/attribution.rs; then
  echo "walk.rs/attribution.rs have uncommitted changes; commit or stash first" >&2; exit 2
fi
run() { cargo run -q -p slop-livin-source-audit -- folding_only_for_artifacts symlinks_never_followed 2>&1 | grep -E '^(ok|FAIL)' | sed 's/^/    /'; git checkout -q crates/core/src/walk.rs crates/core/src/attribution.rs; }
echo "M1 process_size: symlink_metadata -> metadata (follows the link)"
sed -i '' '744s/fs::symlink_metadata(entry.path())/fs::metadata(entry.path())/' crates/core/src/walk.rs; run
echo "M2 process_size: delete the is_symlink guard from the loop"
sed -i '' '734,736d' crates/core/src/walk.rs; run
echo "M3 process_walk: decide is_dir on entry.path() (stats through the link)"
sed -i '' 's/} else if ft.is_dir() {/} else if entry.path().is_dir() {/' crates/core/src/walk.rs; run
echo "M4 process_walk: fold under a made-up kind instead of classify_at"
sed -i '' 's/if let Some(kind) = classify_at(&path, &name) {/if let Some(kind) = Some(ArtifactKind::Cache) {/' crates/core/src/walk.rs; run
echo "M5 classify_at: fold every unknown name as Cache"
python3 - <<'PY'
p='crates/core/src/attribution.rs'; s=open(p).read()
s=s.replace('''        .find(|(n, _, markers)| *n == name && has_marker(parent, markers))
        .map(|(_, k, _)| k.clone())
}''','''        .find(|(n, _, markers)| *n == name && has_marker(parent, markers))
        .map(|(_, k, _)| k.clone())
        .or(Some(ArtifactKind::Cache))
}''',1); open(p,'w').write(s)
PY
run
echo "M6 serial walker: record an artifact without classifying"
python3 - <<'PY'
p='crates/core/src/attribution.rs'; s=open(p).read()
old='        if let Some(kind) = path.parent().and_then(|parent| classify_at(parent, name)) {'
assert old in s
s=s.replace(old,'        if let Some(kind) = Some(ArtifactKind::BuildOutput) {',1); open(p,'w').write(s)
PY
run
echo "M7 size_dir_recursive: recurse on is_dir before is_symlink"
python3 - <<'PY'
p='crates/core/src/walk.rs'; s=open(p).read()
s=s.replace('''        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            total += size_dir_recursive(&entry.path(), seen, mtime_max);''','''        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            total += size_dir_recursive(&entry.path(), seen, mtime_max);
        }
        if ft.is_symlink() {
            continue;
        }
        if false {''',1); open(p,'w').write(s)
PY
run
git status --short crates/core/src | head -3
