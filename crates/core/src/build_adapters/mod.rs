//! Build-artifact adapters: what is *inside* a build container, and
//! what removing a piece of it would cost.
//!
//! An artifact row (`target/`, `node_modules/`, `build/`) is an
//! accounting boundary the folded walk already measured. This module
//! identifies its interior into [`crate::artifact::NestedArtifact`]s the
//! same way [`crate::agents`] identifies the interior of an agent tool's
//! home -- same shape, different domain, and deliberately the same
//! seams:
//!
//! * a trait plus a static registry ([`registry::Registry`]), never a
//!   central `match ecosystem`;
//! * identification that reads directory *names* from the folded walk's
//!   own rows, plus bounded reads of named manifests
//!   ([`bounded_io::read_manifest`]) -- never a second traversal, never
//!   a whole arbitrary file, never a subprocess;
//! * units built through [`NestedUnitBuilder`], whose defaults are the
//!   honest ones (unsupported coverage, unknown accounting basis,
//!   unknown time source, inspection only);
//! * container-level reuse gated on
//!   [`crate::fs_events::EventCoverage`], exactly as stack/13 gated the
//!   agent containers -- never on a directory's own modification stamp,
//!   which does not move when a file inside a subdirectory changes.
//!
//! # What an adapter is allowed to claim
//!
//! Identification is not a verdict. An adapter says what a directory
//! *is* (a build output, a test report, an installed dependency tree, an
//! entry in a shared store), when it was last *modified*, how many bytes
//! it holds on a stated basis, and what it would cost to get it back.
//! It never says a unit is unused, stale, obsolete or removable, and it
//! never infers last execution from a timestamp. Where the evidence runs
//! out the unit carries an explicit unknown -- an unsupported layout is
//! a named limitation, not an empty result.
//!
//! # Cost
//!
//! Ordinary refresh of an unchanged container does zero listings and
//! zero manifest reads: the container's previously identified units are
//! replayed under event coverage. A changed container pays one shallow
//! listing per directory the adapter actually needs to look inside, plus
//! one bounded manifest read per named metadata file. Both are counted
//! (`crate::work_counters`), which is what makes the cost tests in
//! `crates/core/tests/build_adapter_cost.rs` assertions rather than
//! claims.

pub mod bounded_io;
pub mod cargo;
pub mod gradle;
pub mod jvm_common;
pub mod matrix;
pub mod maven;
pub mod node;
pub mod registry;

use crate::artifact::{
    AccountingBasis, ArtifactCoverage, ArtifactEvidence, ArtifactRole, ArtifactVariant, Membership,
    NestedActionCapability, NestedArtifact, RoleFamily, TimeSource, relative_path,
};
use crate::entities::Confidence;
use crate::fs_events::EventCoverage;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------
// Containers
// ---------------------------------------------------------------------

/// One build container an adapter has been asked to explain: a
/// `target/`, a `node_modules/`, a Gradle `build/`, a `~/.m2/repository`.
///
/// The path arrives from the report pipeline, resolved from the artifact
/// rows the folded walk produced and (for shared stores) from the
/// location detectors. An adapter never resolves a home, reads an
/// environment variable or guesses a root, for the same reason the agent
/// adapters do not: a fixture must be able to inject one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildContainer {
    pub adapter_id: &'static str,
    pub path: PathBuf,
    /// The checkout this container belongs to, when one is established.
    /// `None` for a shared store that belongs to no single project --
    /// which is a fact about ownership, not a missing value to fill in.
    pub project_root: Option<PathBuf>,
    /// Whether this container is shared across projects. A shared
    /// container's entries are charged once, wherever else they are
    /// linked from (#68: "Account for linked/shared store entries
    /// without duplicated measurement").
    pub shared: bool,
}

impl BuildContainer {
    pub fn project(adapter_id: &'static str, path: PathBuf, project_root: PathBuf) -> Self {
        Self {
            adapter_id,
            path,
            project_root: Some(project_root),
            shared: false,
        }
    }

    pub fn shared_store(adapter_id: &'static str, path: PathBuf) -> Self {
        Self {
            adapter_id,
            path,
            project_root: None,
            shared: true,
        }
    }

    /// The container's own storage id, the prefix every unit inside it
    /// hangs off. Identical to what `cargo_artifacts` used before the
    /// port, so a stored report's units keep their ids.
    pub fn scope(&self) -> String {
        NestedArtifact::storage_id(&self.path, "")
    }
}

// ---------------------------------------------------------------------
// Folded rows: the structure an adapter is allowed to see
// ---------------------------------------------------------------------

/// One measured directory, as the folded walk left it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoldedDir {
    pub path: PathBuf,
    /// This directory's own files plus every descendant's, in allocated
    /// bytes.
    pub allocated_total: u64,
    /// Newest modification among its measured entries, in seconds.
    pub mtime_max: u64,
    /// Whether the walk read all of it.
    pub complete: bool,
}

/// The folded walk's directory rows, indexed by absolute path.
///
/// This is the *only* structural input an adapter gets for free. It is
/// what makes "no second traversal" achievable rather than aspirational:
/// the walk already listed every directory under a container, so an
/// adapter asking "what is under `target/`" is answering from memory.
#[derive(Debug, Default, Clone)]
pub struct FoldedIndex {
    by_path: HashMap<PathBuf, FoldedDir>,
    children: HashMap<PathBuf, Vec<PathBuf>>,
}

impl FoldedIndex {
    pub fn from_dirs(dirs: impl IntoIterator<Item = FoldedDir>) -> Self {
        let mut idx = Self::default();
        for d in dirs {
            if let Some(parent) = d.path.parent() {
                idx.children
                    .entry(parent.to_path_buf())
                    .or_default()
                    .push(d.path.clone());
            }
            idx.by_path.insert(d.path.clone(), d);
        }
        for kids in idx.children.values_mut() {
            kids.sort();
            kids.dedup();
        }
        idx
    }

    pub fn get(&self, path: &Path) -> Option<&FoldedDir> {
        self.by_path.get(path)
    }

    /// Directories directly inside `path`, in name order.
    pub fn children(&self, path: &Path) -> Vec<&FoldedDir> {
        self.children
            .get(path)
            .map(|kids| kids.iter().filter_map(|k| self.by_path.get(k)).collect())
            .unwrap_or_default()
    }

    /// Every measured directory at or under `path`.
    pub fn under(&self, path: &Path) -> Vec<&FoldedDir> {
        let mut out: Vec<&FoldedDir> = self
            .by_path
            .values()
            .filter(|d| d.path.starts_with(path))
            .collect();
        out.sort_by(|a, b| a.path.cmp(&b.path));
        out
    }

    pub fn is_empty(&self) -> bool {
        self.by_path.is_empty()
    }
}

// ---------------------------------------------------------------------
// Container-level reuse, gated on event coverage
// ---------------------------------------------------------------------

/// Previously identified units, keyed by container scope, with the
/// observation time they were written at.
#[derive(Debug, Default)]
pub struct ContainerCache {
    entries: RefCell<HashMap<String, (u64, Vec<NestedArtifact>)>>,
    enabled: bool,
}

impl ContainerCache {
    /// A cache nothing can be replayed from. What a store-less caller, a
    /// forced full walk and every execution-time recheck get.
    pub fn disabled() -> Self {
        Self {
            entries: RefCell::new(HashMap::new()),
            enabled: false,
        }
    }

    /// Seeds the cache from a previous report's nested units, grouped by
    /// the container each one belongs to.
    pub fn from_previous(units: Vec<NestedArtifact>, stored_at: u64) -> Self {
        let mut grouped: HashMap<String, Vec<NestedArtifact>> = HashMap::new();
        for u in units {
            let key = u.container_id.clone().unwrap_or_else(|| u.id.clone());
            grouped.entry(key).or_default().push(u);
        }
        Self {
            entries: RefCell::new(
                grouped
                    .into_iter()
                    .map(|(k, v)| (k, (stored_at, v)))
                    .collect(),
            ),
            enabled: true,
        }
    }
}

/// What an adapter identifies through.
pub struct BuildCtx<'a> {
    pub observed_at: u64,
    folded: &'a FoldedIndex,
    coverage: &'a EventCoverage,
    cache: &'a ContainerCache,
}

impl<'a> BuildCtx<'a> {
    pub fn new(
        observed_at: u64,
        folded: &'a FoldedIndex,
        coverage: &'a EventCoverage,
        cache: &'a ContainerCache,
    ) -> Self {
        Self {
            observed_at,
            folded,
            coverage,
            cache,
        }
    }

    pub fn folded(&self) -> &FoldedIndex {
        self.folded
    }

    /// Identifies one container, replaying the stored units when this
    /// pass's event coverage can show that nothing under the container
    /// changed since they were written.
    ///
    /// The gate is [`EventCoverage::unchanged_since`] and nothing else.
    /// A directory-stamp comparison is deliberately absent: a container's
    /// own stamp does not move when a file inside one of its
    /// subdirectories changes, so stamp-only reuse replays a stale
    /// identification forever -- and a `target/` or `node_modules` is
    /// exactly the tree where that happens.
    pub fn container(
        &self,
        container: &BuildContainer,
        identify: &dyn Fn() -> Vec<NestedArtifact>,
    ) -> Vec<NestedArtifact> {
        if let Some(units) = self.replay(container) {
            crate::work_counters::record_container_reused();
            return units;
        }
        crate::work_counters::record_container_identified();
        identify()
    }

    /// The reuse decision on its own, so a test can ask for it without
    /// running an identification.
    pub fn can_reuse(&self, container: &BuildContainer) -> bool {
        self.replay_key(container).is_some()
    }

    fn replay_key(&self, container: &BuildContainer) -> Option<(String, u64)> {
        if !self.cache.enabled {
            return None;
        }
        let key = container.scope();
        let stored_at = self.cache.entries.borrow().get(&key).map(|(at, _)| *at)?;
        // The whole gate, in one condition: a trusted window that
        // covers this container, opened no later than the rows were
        // written, reporting no event under it.
        self.coverage
            .unchanged_since(&container.path, stored_at)
            .then_some((key, stored_at))
    }

    fn replay(&self, container: &BuildContainer) -> Option<Vec<NestedArtifact>> {
        let (key, _) = self.replay_key(container)?;
        let mut entries = self.cache.entries.borrow_mut();
        let (observed_at, units) = entries.get_mut(&key)?;
        // Replayed *and re-verified*: the window vouched for these rows
        // just now, so the next pass's window -- which starts where this
        // one ended -- can vouch for them again. Without this, reuse
        // would only ever work on alternate passes.
        *observed_at = self.observed_at;
        Some(units.clone())
    }

    /// A single-level listing of `dir`, through the shared capped
    /// helper. Counted, symlink-refusing, and the only listing an
    /// adapter may do.
    pub fn list(&self, dir: &Path) -> Vec<crate::locations::ShallowEntry> {
        crate::locations::shallow_list(dir)
    }

    /// `symlink_metadata` for one named path. Not a traversal: the
    /// adapter already knew this path's name from the folded rows or
    /// from a capped listing.
    pub fn stat(&self, path: &Path) -> Option<std::fs::Metadata> {
        crate::work_counters::record_files_statted(1);
        std::fs::symlink_metadata(path).ok()
    }

    /// A bounded read of one named manifest.
    pub fn manifest(&self, path: &Path) -> Option<bounded_io::Manifest> {
        bounded_io::read_manifest(path, bounded_io::MAX_MANIFEST_BYTES)
    }
}

// ---------------------------------------------------------------------
// The trait
// ---------------------------------------------------------------------

/// What an adapter says it can do, checked against the published matrix.
///
/// Every field defaults to `false`, and that is the point: an adapter
/// that says nothing claims nothing. `actions_available` in particular
/// must stay false for every adapter until #73 gives one an executor,
/// which `registry::no_adapter_claims_an_action_is_available` asserts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BuildCapabilities {
    /// Whether this adapter identifies containers shared across
    /// projects (a package store, a local repository) as well as
    /// project-local ones.
    pub identifies_shared_stores: bool,
    /// Whether this adapter can attribute a unit to a package identity
    /// (a name and version) from read-only metadata.
    pub attributes_package_identity: bool,
    /// Whether any action is available on this adapter's units. `false`
    /// for every adapter in this chunk; #73 implements adapter actions.
    pub actions_available: bool,
}

/// One ecosystem family's read-only build-artifact identification.
///
/// An adapter identifies, and only identifies: it reads folded rows,
/// capped listings and bounded manifests, returns
/// [`NestedArtifact`]s, and never acts, never emits and never spawns.
/// Each of those is a separate audited guardrail
/// (`.oh/guardrails/build-adapters-*.md`).
pub trait BuildAdapter: Send + Sync {
    /// Stable id, equal to this adapter's module name with `_` replaced
    /// by `-`, to its [`matrix`] row and to its docs table row.
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;
    fn capabilities(&self) -> BuildCapabilities {
        BuildCapabilities::default()
    }

    /// Which containers under `project_root` this adapter claims, given
    /// the artifact directories the walk found there.
    ///
    /// `candidates` are absolute paths of artifact rows beneath the
    /// checkout; the adapter decides which ones are its own, by name and
    /// by the marker files beside them. It never scans for more.
    fn containers(&self, project_root: &Path, candidates: &[PathBuf]) -> Vec<BuildContainer>;

    /// Identify the interior of one container.
    fn identify(&self, container: &BuildContainer, ctx: &BuildCtx) -> Vec<NestedArtifact>;
}

// ---------------------------------------------------------------------
// NestedUnitBuilder: the honest defaults are a constructor
// ---------------------------------------------------------------------

/// The only way an adapter builds a unit
/// (`.oh/guardrails/build-units-built-through-builder.md`).
///
/// A `NestedArtifact { .. }` literal has to spell out coverage,
/// membership, the accounting basis, the timestamp source, the physical
/// charge and the action capability -- and every one of those has a
/// wrong value that *overstates* what swamp knows: supported coverage
/// for an untested layout, `Exclusive` membership on a hardlinked store
/// entry, a physical charge on an aggregate, an action capability no
/// executor backs. The constructor picks the understating value for each
/// one; lifting it is a named method, visible in the diff.
pub struct NestedUnitBuilder {
    unit: NestedArtifact,
}

impl NestedUnitBuilder {
    /// A new unit inside `container`, at `path`, in `role`.
    ///
    /// Defaults: coverage unsupported and incomplete, membership
    /// unknown, accounting basis unknown, time source unknown, zero
    /// physical charge, [`NestedActionCapability::InspectionOnly`].
    pub fn new(container: &BuildContainer, role: ArtifactRole, path: PathBuf) -> Self {
        let scope = container.scope();
        let rel = relative_path(&container.path, &path);
        let id = NestedArtifact::within(&scope, &rel);
        let parent_id = (path != container.path)
            .then(|| {
                path.parent()
                    .map(|p| NestedArtifact::within(&scope, &relative_path(&container.path, p)))
            })
            .flatten();
        Self {
            unit: NestedArtifact {
                id,
                path,
                relative_path: rel,
                parent_id,
                container_id: Some(scope),
                role,
                membership: Membership::Unknown,
                is_dir: false,
                device: 0,
                inode: 0,
                logical_bytes: 0,
                bytes: 0,
                physical_bytes: 0,
                physical_total: 0,
                mtime_max: 0,
                variant: ArtifactVariant::default(),
                producer_evidence: Vec::new(),
                consumer_evidence: Vec::new(),
                coverage: ArtifactCoverage {
                    supported: false,
                    complete: false,
                    limits: Vec::new(),
                },
                action_group: None,
                present: true,
                growth_bytes: None,
                regrowth_count: 0,
                decision_evidence: Vec::new(),
                adapter: Some(container.adapter_id.to_string()),
                basis: AccountingBasis::Unknown,
                time_source: TimeSource::Unknown,
                action: NestedActionCapability::InspectionOnly,
                consequence: None,
            },
        }
    }

    /// Declares this unit's layout tested and understood, with the
    /// reason that makes the claim reviewable. An empty reason is
    /// rejected by `build_units_built_through_builder`.
    pub fn supported_with_reason(mut self, reason: impl Into<String>) -> Self {
        let reason = reason.into();
        debug_assert!(!reason.trim().is_empty(), "a support claim needs a reason");
        self.unit.coverage.supported = true;
        self.unit.producer_evidence.push(ArtifactEvidence {
            source: format!("{}-layout", self.unit.adapter.clone().unwrap_or_default()),
            detail: reason,
            confidence: Confidence::Medium,
        });
        self
    }

    /// Marks the layout as one this adapter does not understand. The
    /// unit is still emitted -- an unsupported layout is a named
    /// limitation, never an empty result.
    pub fn unsupported_layout(mut self, limit: impl Into<String>) -> Self {
        self.unit.coverage.supported = false;
        self.unit.coverage.limits.push(limit.into());
        self.unit.action = NestedActionCapability::Unsupported {
            reason: "the layout is not one this adapter identifies".into(),
        };
        self
    }

    pub fn complete(mut self, complete: bool) -> Self {
        self.unit.coverage.complete = complete;
        self
    }

    pub fn limit(mut self, limit: impl Into<String>) -> Self {
        self.unit.coverage.limits.push(limit.into());
        self
    }

    pub fn is_dir(mut self, is_dir: bool) -> Self {
        self.unit.is_dir = is_dir;
        self
    }

    /// Allocated bytes from a folded directory measurement, with the
    /// basis and the timestamp source recorded together -- the two facts
    /// that make the number interpretable.
    pub fn folded(mut self, dir: &FoldedDir) -> Self {
        self.unit.bytes = dir.allocated_total;
        self.unit.basis = AccountingBasis::Allocated;
        self.unit.mtime_max = dir.mtime_max;
        self.unit.time_source = TimeSource::FoldedDirectoryModification;
        self.unit.is_dir = true;
        self.unit.coverage.complete = dir.complete;
        if !dir.complete {
            self.unit
                .coverage
                .limits
                .push("the walk could not read all of this directory".into());
        }
        self
    }

    /// One file's own metadata.
    pub fn from_file_metadata(mut self, meta: &std::fs::Metadata) -> Self {
        use std::os::unix::fs::MetadataExt;
        self.unit.bytes = meta.blocks() * 512;
        self.unit.basis = AccountingBasis::Allocated;
        self.unit.logical_bytes = meta.len();
        self.unit.mtime_max = meta.mtime().max(0) as u64;
        self.unit.time_source = TimeSource::FileModification;
        self.unit.device = meta.dev();
        self.unit.inode = meta.ino();
        self.unit.is_dir = meta.is_dir();
        self.unit.membership = if meta.nlink() > 1 && meta.is_file() {
            Membership::SharedHardlink
        } else {
            Membership::Exclusive
        };
        self.unit.coverage.complete = true;
        self
    }

    pub fn bytes_on_basis(mut self, bytes: u64, basis: AccountingBasis) -> Self {
        self.unit.bytes = bytes;
        self.unit.basis = basis;
        self
    }

    pub fn modified(mut self, mtime: u64, source: TimeSource) -> Self {
        self.unit.mtime_max = mtime;
        self.unit.time_source = source;
        self
    }

    pub fn membership(mut self, m: Membership) -> Self {
        self.unit.membership = m;
        self
    }

    pub fn variant(mut self, v: ArtifactVariant) -> Self {
        self.unit.variant = v;
        self
    }

    pub fn evidence(mut self, source: &str, detail: impl Into<String>, c: Confidence) -> Self {
        self.unit.producer_evidence.push(ArtifactEvidence {
            source: source.into(),
            detail: detail.into(),
            confidence: c,
        });
        self
    }

    /// What it would cost to get these bytes back, in the ecosystem's
    /// own words. A consequence, never a verdict.
    pub fn consequence(mut self, text: impl Into<String>) -> Self {
        self.unit.consequence = Some(text.into());
        self
    }

    /// Declares that no action is available for this unit and why --
    /// for a shared store entry other projects link to, or a unit whose
    /// measurement is incomplete.
    pub fn no_action_because(mut self, reason: impl Into<String>) -> Self {
        self.unit.action = NestedActionCapability::Unsupported {
            reason: reason.into(),
        };
        self
    }

    /// The exact filesystem members that would have to be considered
    /// together. Naming the group does not make it actionable; the
    /// action layer still authorizes and re-checks it (#64: units are
    /// distinct from action groups).
    pub fn action_group(mut self, group: impl Into<String>) -> Self {
        self.unit.action_group = Some(NestedArtifact::action_group(&group.into()));
        self
    }

    pub fn build(self) -> NestedArtifact {
        self.unit
    }
}

// ---------------------------------------------------------------------
// Aggregation (#64's collapsed category rows)
// ---------------------------------------------------------------------

/// A collapsed family row: what a container holds in one role family,
/// on one accounting basis, with one oldest *modification* time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FamilySummary {
    pub family: RoleFamily,
    pub count: usize,
    pub bytes: u64,
    pub basis: AccountingBasis,
    /// The oldest known modification among this family's units, or
    /// `None` when no unit in it has a known time. A category's oldest
    /// candidate is not its last use.
    pub oldest_modified: Option<u64>,
    /// Units whose modification time is unknown. Reported rather than
    /// folded into the oldest, so "unknown ages last" is a fact the
    /// caller has, not an ordering trick.
    pub unknown_age: usize,
    /// Whether every unit in this family had complete coverage.
    pub complete: bool,
}

/// Collapses a container's units into one row per role family.
///
/// Two rules, both from #65:
///
/// * **stop at the outermost included unit.** A unit whose ancestor is
///   also in the list is not counted again -- otherwise a `dist/` and
///   the `dist/assets/` inside it would both be charged and the family
///   total would exceed the container.
/// * **never mix bases.** Units are summed only with others on the same
///   accounting basis; a family holding both allocated and logical
///   numbers reports [`AccountingBasis::Unknown`] and no total, because
///   a mixed number is worse than no number.
pub fn summarize_families(units: &[NestedArtifact]) -> Vec<FamilySummary> {
    let paths: std::collections::HashSet<&Path> = units.iter().map(|u| u.path.as_path()).collect();
    let mut by_family: HashMap<RoleFamily, Vec<&NestedArtifact>> = HashMap::new();
    for u in units {
        if u.role == ArtifactRole::Container {
            continue;
        }
        // Descendant of another identified unit in the same list: its
        // bytes are already inside that one's total.
        let nested_under_sibling = u
            .path
            .ancestors()
            .skip(1)
            .any(|a| paths.contains(a) && units.iter().any(|o| o.path == a && o.is_dir));
        if nested_under_sibling {
            continue;
        }
        by_family.entry(u.role.family()).or_default().push(u);
    }
    let mut out = Vec::new();
    for family in RoleFamily::ALL {
        let Some(members) = by_family.get(family) else {
            continue;
        };
        if members.is_empty() {
            continue;
        }
        let bases: std::collections::BTreeSet<&str> =
            members.iter().map(|u| u.basis.label()).collect();
        let (bytes, basis) = if bases.len() == 1 {
            (members.iter().map(|u| u.bytes).sum(), members[0].basis)
        } else {
            (0, AccountingBasis::Unknown)
        };
        let known: Vec<u64> = members
            .iter()
            .filter(|u| u.time_source != TimeSource::Unknown && u.mtime_max > 0)
            .map(|u| u.mtime_max)
            .collect();
        out.push(FamilySummary {
            family: *family,
            count: members.len(),
            bytes,
            basis,
            oldest_modified: known.iter().copied().min(),
            unknown_age: members.len() - known.len(),
            complete: members.iter().all(|u| u.coverage.complete),
        });
    }
    out
}

/// The entry point the report pipeline uses: every registered adapter,
/// over every container it claims under the given projects.
///
/// One pass, one ownership, no per-adapter traversal. Ordering between
/// adapters is fixed by the registry, so two adapters that both claim a
/// `build/` directory (a Gradle project that is also a Node workspace)
/// resolve the same way every pass.
pub fn identify_all(
    registry: &registry::Registry,
    projects: &[(PathBuf, Vec<PathBuf>)],
    shared: &[BuildContainer],
    ctx: &BuildCtx,
) -> Vec<NestedArtifact> {
    let mut out = Vec::new();
    let mut claimed: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for adapter in registry.adapters() {
        for (root, candidates) in projects {
            for container in adapter.containers(root, candidates) {
                if !claimed.insert(container.path.clone()) {
                    continue;
                }
                out.extend(ctx.container(&container, &|| adapter.identify(&container, ctx)));
            }
        }
        for container in shared.iter().filter(|c| c.adapter_id == adapter.id()) {
            if !claimed.insert(container.path.clone()) {
                continue;
            }
            out.extend(ctx.container(container, &|| adapter.identify(container, ctx)));
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out.dedup_by(|a, b| a.path == b.path);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn container(path: &Path) -> BuildContainer {
        BuildContainer::project("cargo", path.to_path_buf(), path.to_path_buf())
    }

    #[test]
    fn builder_defaults_understate_rather_than_overstate() {
        let tmp = tempfile::tempdir().unwrap();
        let c = container(tmp.path());
        let u = NestedUnitBuilder::new(&c, ArtifactRole::Output, tmp.path().join("dist")).build();
        assert!(!u.coverage.supported, "support is claimed, never assumed");
        assert!(!u.coverage.complete);
        assert_eq!(u.basis, AccountingBasis::Unknown);
        assert_eq!(u.time_source, TimeSource::Unknown);
        assert_eq!(u.membership, Membership::Unknown);
        assert_eq!(u.physical_total, 0, "an aggregate carries no charge");
        assert_eq!(u.action, NestedActionCapability::InspectionOnly);
    }

    #[test]
    fn an_unchanged_container_is_replayed_only_under_trusted_coverage() {
        let tmp = tempfile::tempdir().unwrap();
        let c = container(tmp.path());
        let folded = FoldedIndex::default();
        let prior =
            vec![NestedUnitBuilder::new(&c, ArtifactRole::Output, tmp.path().join("dist")).build()];
        let cache = ContainerCache::from_previous(prior, 100);

        let untrusted = EventCoverage::untrusted();
        let ctx = BuildCtx::new(200, &folded, &untrusted, &cache);
        assert!(
            !ctx.can_reuse(&c),
            "no window, no reuse -- a stamp is not a substitute"
        );

        let trusted = EventCoverage::trusted(tmp.path().to_path_buf(), Vec::new(), 100);
        let ctx = BuildCtx::new(200, &folded, &trusted, &cache);
        assert!(ctx.can_reuse(&c));

        let noisy = EventCoverage::trusted(
            tmp.path().to_path_buf(),
            vec![tmp.path().join("dist/app.js")],
            100,
        );
        let ctx = BuildCtx::new(200, &folded, &noisy, &cache);
        assert!(
            !ctx.can_reuse(&c),
            "an event *inside* the container is a change to it"
        );
    }

    #[test]
    fn a_family_summary_stops_at_the_outermost_unit() {
        let tmp = tempfile::tempdir().unwrap();
        let c = container(tmp.path());
        let outer = NestedUnitBuilder::new(&c, ArtifactRole::Output, tmp.path().join("dist"))
            .is_dir(true)
            .bytes_on_basis(1000, AccountingBasis::Allocated)
            .build();
        let inner =
            NestedUnitBuilder::new(&c, ArtifactRole::Output, tmp.path().join("dist/assets"))
                .is_dir(true)
                .bytes_on_basis(400, AccountingBasis::Allocated)
                .build();
        let s = summarize_families(&[outer, inner]);
        let outputs = s.iter().find(|f| f.family == RoleFamily::Outputs).unwrap();
        assert_eq!(outputs.count, 1);
        assert_eq!(
            outputs.bytes, 1000,
            "a descendant's bytes are already inside its ancestor's total"
        );
    }

    #[test]
    fn a_family_mixing_accounting_bases_reports_no_total() {
        let tmp = tempfile::tempdir().unwrap();
        let c = container(tmp.path());
        let a = NestedUnitBuilder::new(&c, ArtifactRole::Output, tmp.path().join("a"))
            .bytes_on_basis(100, AccountingBasis::Allocated)
            .build();
        let b = NestedUnitBuilder::new(&c, ArtifactRole::Output, tmp.path().join("b"))
            .bytes_on_basis(100, AccountingBasis::Logical)
            .build();
        let s = summarize_families(&[a, b]);
        let outputs = s.iter().find(|f| f.family == RoleFamily::Outputs).unwrap();
        assert_eq!(outputs.basis, AccountingBasis::Unknown);
        assert_eq!(
            outputs.bytes, 0,
            "a mixed-basis number is worse than no number"
        );
    }

    #[test]
    fn unknown_ages_are_counted_not_folded_into_the_oldest() {
        let tmp = tempfile::tempdir().unwrap();
        let c = container(tmp.path());
        let dated = NestedUnitBuilder::new(&c, ArtifactRole::Output, tmp.path().join("a"))
            .bytes_on_basis(1, AccountingBasis::Allocated)
            .modified(5_000, TimeSource::FoldedDirectoryModification)
            .build();
        let undated = NestedUnitBuilder::new(&c, ArtifactRole::Output, tmp.path().join("b"))
            .bytes_on_basis(1, AccountingBasis::Allocated)
            .build();
        let s = summarize_families(&[dated, undated]);
        let outputs = s.iter().find(|f| f.family == RoleFamily::Outputs).unwrap();
        assert_eq!(outputs.oldest_modified, Some(5_000));
        assert_eq!(
            outputs.unknown_age, 1,
            "an unknown age is reported, never treated as epoch and ranked ancient"
        );
    }
}
