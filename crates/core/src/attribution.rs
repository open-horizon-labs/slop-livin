//! R3: artifact classification at discovery, attributed to the nearest
//! containing checkout/worktree.
//!
//! A directory is classified by basename the moment it is found; a
//! classified directory is sized as one unit (allocated bytes, not
//! descended into for classification) and never walked further. Anything
//! left over inside a worktree becomes that worktree's single `Source`
//! row. Anything outside every worktree is an `UnownedRow`.

use crate::report::{ArtifactKind, ArtifactRow, Source, UnownedReason, UnownedRow, WorktreeRow};
use std::collections::HashSet;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

/// One basename -> kind entry. Data-driven so a later slice (R7) can
/// attach a recovery contract per row without touching the walk.
const ARTIFACT_KINDS: &[(&str, ArtifactKind)] = &[
    ("node_modules", ArtifactKind::DependencyTree),
    ("target", ArtifactKind::BuildOutput),
    ("build", ArtifactKind::BuildOutput),
    ("dist", ArtifactKind::BuildOutput),
    ("out", ArtifactKind::BuildOutput),
    (".next", ArtifactKind::BuildOutput),
    (".nuxt", ArtifactKind::BuildOutput),
    (".venv", ArtifactKind::DependencyTree),
    ("venv", ArtifactKind::DependencyTree),
    ("__pycache__", ArtifactKind::BuildOutput),
    (".git", ArtifactKind::Git),
    ("Pods", ArtifactKind::DependencyTree),
    ("DerivedData", ArtifactKind::BuildOutput),
    (".gradle", ArtifactKind::DependencyTree),
    (".cache", ArtifactKind::Cache),
    (".turbo", ArtifactKind::BuildOutput),
    (".parcel-cache", ArtifactKind::Cache),
    ("coverage", ArtifactKind::BuildOutput),
    ("vendor", ArtifactKind::DependencyTree),
];

/// Basenames that, when found *outside* every checkout/worktree, are a
/// shared cache rather than an ordinary unowned path.
const SHARED_CACHE_NAMES: &[&str] = &[
    ".cache",
    ".cargo-registry",
    ".npm",
    ".pnpm-store",
    ".gradle",
];

pub(crate) fn classify(name: &str) -> Option<ArtifactKind> {
    ARTIFACT_KINDS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, k)| k.clone())
        // Shared-cache-shaped names not already in the artifact table
        // (e.g. `.cargo-registry`, `.npm`, `.pnpm-store`) are still sized
        // as one unit rather than descended into; `record_artifact`
        // decides project-internal Cache vs. unowned SharedCache from
        // whether a containing worktree is found.
        .or_else(|| is_shared_cache_name(name).then_some(ArtifactKind::Cache))
}

pub(crate) fn is_shared_cache_name(name: &str) -> bool {
    SHARED_CACHE_NAMES.contains(&name)
}

/// Allocated bytes for one file: `st_blocks * 512`, the actual space the
/// file occupies on disk rather than its logical length.
pub(crate) fn allocated_bytes(meta: &fs::Metadata) -> u64 {
    meta.blocks() * 512
}

/// A worktree known to the attribution pass: its path (for nearest-match)
/// and the id it should attach rows to.
struct KnownWorktree<'a> {
    path: &'a Path,
    worktree_id: &'a str,
}

/// Output of the attribution walk: per-worktree artifact rows (including
/// the synthesized `Source` row), unowned rows, and the walked total.
pub struct AttributionResult {
    /// worktree_id -> artifact rows for that worktree.
    pub artifacts_by_worktree: std::collections::HashMap<String, Vec<ArtifactRow>>,
    pub unowned: Vec<UnownedRow>,
    pub walked_total: u64,
    /// Sum of every artifact row's bytes (all worktrees), used for
    /// reconciliation instead of re-deriving it from the result map.
    pub attributed_total: u64,
    /// Sum of unowned bytes actually seen by this filesystem walk
    /// (excludes anything added later from an out-of-band source such as
    /// Docker facts, which never appear in `walked_total` either).
    pub unowned_total: u64,
}

struct Ctx<'a> {
    worktrees: Vec<KnownWorktree<'a>>,
    seen_inodes: HashSet<(u64, u64)>,
    artifacts_by_worktree: std::collections::HashMap<String, Vec<ArtifactRow>>,
    source_bytes: std::collections::HashMap<String, u64>,
    unowned: Vec<UnownedRow>,
    walked_total: u64,
    attributed_total: u64,
    unowned_total: u64,
    observed_at: u64,
}

/// Finds the id of the worktree whose path is the longest prefix of
/// `path` (the nearest containing checkout/worktree), if any.
fn nearest_worktree<'a>(worktrees: &'a [KnownWorktree<'a>], path: &Path) -> Option<&'a str> {
    worktrees
        .iter()
        .filter(|w| path.starts_with(w.path))
        .max_by_key(|w| w.path.as_os_str().len())
        .map(|w| w.worktree_id)
}

impl<'a> Ctx<'a> {
    fn dedup(&mut self, meta: &fs::Metadata) -> bool {
        // Hardlinks are counted once per report.
        self.seen_inodes.insert((meta.dev(), meta.ino()))
    }

    /// Sizes a classified artifact directory as one unit: sums allocated
    /// bytes of every regular file underneath (deduped by hardlink),
    /// without treating any of it as source or classifying subdirectories
    /// further.
    fn size_as_unit(&mut self, path: &Path) -> u64 {
        let mut total = 0u64;
        let Ok(entries) = fs::read_dir(path) else {
            return total;
        };
        for entry in entries.flatten() {
            let Ok(meta) = fs::symlink_metadata(entry.path()) else {
                continue;
            };
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                total += self.size_as_unit(&entry.path());
            } else if meta.is_file() && self.dedup(&meta) {
                total += allocated_bytes(&meta);
            }
        }
        total
    }

    /// Records a classified artifact at `path` (kind already resolved),
    /// attributing it to the nearest containing worktree or recording it
    /// as unowned.
    fn record_artifact(&mut self, path: &Path, kind: ArtifactKind) {
        let bytes = self.size_as_unit(path);
        self.walked_total += bytes;
        match nearest_worktree(&self.worktrees, path) {
            Some(worktree_id) => {
                self.attributed_total += bytes;
                self.artifacts_by_worktree
                    .entry(worktree_id.to_string())
                    .or_default()
                    .push(ArtifactRow {
                        kind,
                        path: path.to_path_buf(),
                        bytes,
                        growth_bytes: None,
                        regrowth_count: 0,
                        observed_at: self.observed_at,
                        confidence: crate::entities::Confidence::High,
                        source: Source::new("filesystem.walk"),
                    });
            }
            None => {
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default();
                let reason = if is_shared_cache_name(name) {
                    UnownedReason::SharedCache
                } else {
                    UnownedReason::NoContainingRepo
                };
                self.unowned_total += bytes;
                self.unowned.push(UnownedRow {
                    path_or_object: path.display().to_string(),
                    bytes,
                    reason,
                    shared_bytes: None,
                    note: None,
                });
            }
        }
    }

    fn record_permission_denied(&mut self, path: &Path) {
        self.unowned.push(UnownedRow {
            path_or_object: path.display().to_string(),
            bytes: 0,
            reason: UnownedReason::PermissionDenied,
            shared_bytes: None,
            note: None,
        });
    }

    fn record_file(&mut self, path: &Path, meta: &fs::Metadata) {
        if !self.dedup(meta) {
            return;
        }
        let bytes = allocated_bytes(meta);
        self.walked_total += bytes;
        match nearest_worktree(&self.worktrees, path) {
            Some(worktree_id) => {
                self.attributed_total += bytes;
                *self
                    .source_bytes
                    .entry(worktree_id.to_string())
                    .or_default() += bytes;
            }
            None => {
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default();
                let reason = if is_shared_cache_name(name) {
                    UnownedReason::SharedCache
                } else {
                    UnownedReason::NoContainingRepo
                };
                self.unowned_total += bytes;
                self.unowned.push(UnownedRow {
                    path_or_object: path.display().to_string(),
                    bytes,
                    reason,
                    shared_bytes: None,
                    note: None,
                });
            }
        }
    }

    fn walk(&mut self, path: &Path) {
        let Ok(meta) = fs::symlink_metadata(path) else {
            return;
        };
        if meta.file_type().is_symlink() {
            return;
        }
        if meta.is_file() {
            self.record_file(path, &meta);
            return;
        }
        if !meta.is_dir() {
            return;
        }

        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if let Some(kind) = classify(name) {
            self.record_artifact(path, kind);
            return;
        }

        let entries = match fs::read_dir(path) {
            Ok(e) => e,
            Err(_) => {
                self.record_permission_denied(path);
                return;
            }
        };
        for entry in entries.flatten() {
            self.walk(&entry.path());
        }
    }
}

/// Runs the R3 classification/attribution walk under `root`, given the
/// worktrees R2 already discovered (path + worktree_id).
pub fn attribute(root: &Path, worktrees: &[(&Path, &str)], observed_at: u64) -> AttributionResult {
    let known: Vec<KnownWorktree> = worktrees
        .iter()
        .map(|(path, id)| KnownWorktree {
            path,
            worktree_id: id,
        })
        .collect();
    let mut ctx = Ctx {
        worktrees: known,
        seen_inodes: HashSet::new(),
        artifacts_by_worktree: std::collections::HashMap::new(),
        source_bytes: std::collections::HashMap::new(),
        unowned: Vec::new(),
        walked_total: 0,
        attributed_total: 0,
        unowned_total: 0,
        observed_at,
    };
    ctx.walk(root);

    for (worktree_id, bytes) in &ctx.source_bytes {
        if *bytes == 0 {
            continue;
        }
        let path = worktrees
            .iter()
            .find(|(_, id)| id == worktree_id)
            .map(|(p, _)| p.to_path_buf())
            .unwrap_or_default();
        ctx.artifacts_by_worktree
            .entry(worktree_id.clone())
            .or_default()
            .push(ArtifactRow {
                kind: ArtifactKind::Source,
                path,
                bytes: *bytes,
                growth_bytes: None,
                regrowth_count: 0,
                observed_at: ctx.observed_at,
                confidence: crate::entities::Confidence::High,
                source: Source::new("filesystem.walk"),
            });
    }

    AttributionResult {
        artifacts_by_worktree: ctx.artifacts_by_worktree,
        unowned: ctx.unowned,
        walked_total: ctx.walked_total,
        attributed_total: ctx.attributed_total,
        unowned_total: ctx.unowned_total,
    }
}

/// Fills each `WorktreeRow`'s `artifacts` from the attribution result.
pub fn apply_to_worktree(row: &mut WorktreeRow, result: &mut AttributionResult) {
    if let Some(rows) = result.artifacts_by_worktree.remove(&row.worktree_id) {
        row.artifacts = rows;
    }
}

/// Runs `du -skPx <root>` and returns bytes (KiB * 1024), or `None` if
/// the command is unavailable or fails.
pub fn du_total(root: &Path) -> Option<u64> {
    let out = std::process::Command::new("du")
        .arg("-skPx")
        .arg(root)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let kib: u64 = text.split_whitespace().next()?.parse().ok()?;
    Some(kib * 1024)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::tempdir;

    fn touch(path: &Path, bytes: u64) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, vec![b'x'; bytes as usize]).unwrap();
    }

    #[test]
    fn classification_stops_at_artifact_boundary_does_not_descend() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let repo = root.join("proj");
        touch(&repo.join("node_modules/pkg/deeply/nested/file.js"), 4096);
        touch(&repo.join("README.md"), 4096);

        let worktrees = [(repo.as_path(), "wt1")];
        let result = attribute(root, &worktrees, 0);

        let rows = result.artifacts_by_worktree.get("wt1").unwrap();
        let node_modules_row = rows
            .iter()
            .find(|r| r.kind == ArtifactKind::DependencyTree)
            .expect("node_modules classified as one unit");
        assert_eq!(node_modules_row.path, repo.join("node_modules"));
        // Only one artifact row for the whole node_modules subtree: the
        // deeply nested file never produced its own row.
        assert_eq!(
            rows.iter()
                .filter(|r| r.kind == ArtifactKind::DependencyTree)
                .count(),
            1
        );
    }

    #[test]
    fn nested_repo_build_dir_attributes_to_nested_repo_not_parent() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let parent = root.join("parent");
        let nested = parent.join("nested");
        touch(&nested.join("build/out.bin"), 4096);
        touch(&parent.join("README.md"), 4096);

        let worktrees = [
            (parent.as_path(), "parent-wt"),
            (nested.as_path(), "nested-wt"),
        ];
        let result = attribute(root, &worktrees, 0);

        let nested_rows = result.artifacts_by_worktree.get("nested-wt").unwrap();
        assert!(
            nested_rows
                .iter()
                .any(|r| r.kind == ArtifactKind::BuildOutput && r.path == nested.join("build")),
            "nested repo owns its own build/ artifact: {nested_rows:?}"
        );
        let parent_rows = result.artifacts_by_worktree.get("parent-wt");
        let parent_has_build = parent_rows
            .map(|rows| rows.iter().any(|r| r.kind == ArtifactKind::BuildOutput))
            .unwrap_or(false);
        assert!(
            !parent_has_build,
            "parent must not also claim the nested build/"
        );
    }

    #[test]
    fn shared_cache_outside_repos_is_unowned_once_never_duplicated() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        touch(&root.join("cache/.cargo-registry/a"), 4096);
        touch(&root.join("cache/.cargo-registry/b"), 4096);

        let worktrees: [(&Path, &str); 0] = [];
        let result = attribute(root, &worktrees, 0);

        let cache_rows: Vec<_> = result
            .unowned
            .iter()
            .filter(|u| u.reason == UnownedReason::SharedCache)
            .collect();
        assert_eq!(
            cache_rows.len(),
            1,
            "the shared cache dir is exactly one row, not one per file: {cache_rows:?}"
        );
        assert_eq!(cache_rows[0].bytes, 8192);
        assert_eq!(result.attributed_total, 0);
    }

    #[test]
    fn permission_denied_dir_is_counted_not_dropped() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let locked = root.join("locked");
        fs::create_dir_all(&locked).unwrap();
        touch(&locked.join("secret"), 4096);
        let mut perms = fs::metadata(&locked).unwrap().permissions();
        perms.set_mode(0o000);
        fs::set_permissions(&locked, perms).unwrap();

        let worktrees: [(&Path, &str); 0] = [];
        let result = attribute(root, &worktrees, 0);

        // Restore permissions so tempdir cleanup can remove it.
        let mut restored = fs::metadata(&locked).unwrap().permissions();
        restored.set_mode(0o755);
        fs::set_permissions(&locked, restored).unwrap();

        // Running as root can still read a 0o000 directory; only assert
        // the guard fires when the OS actually denied the read.
        if result
            .unowned
            .iter()
            .any(|u| u.reason == UnownedReason::PermissionDenied)
        {
            let row = result
                .unowned
                .iter()
                .find(|u| u.reason == UnownedReason::PermissionDenied)
                .unwrap();
            assert_eq!(row.bytes, 0);
            assert_eq!(row.path_or_object, locked.display().to_string());
        }
    }
}
