//! The build-adapter capability matrix: a checked code artifact, not a
//! prose claim (#64: "a checked coverage matrix"; #68: "a checked
//! per-tool capability matrix").
//!
//! Every ecosystem family swamp's own catalog can produce build
//! artifacts for gets a row here, including the ones no adapter
//! implements yet. That is deliberate. A matrix listing only what is
//! implemented answers "what does swamp support?" with a list that looks
//! complete; a matrix listing every family with a status answers it with
//! the truth, and the gap is visible to a user deciding whether swamp
//! can explain their disk.
//!
//! The registry, this table and `docs/build-artifacts.md` are checked
//! against each other in both directions
//! (`.oh/guardrails/build-adapter-matrix-matches-docs.md`).

use crate::artifact::RoleFamily;

/// Whether a family has an adapter behind it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// An adapter is registered and tested.
    Implemented,
    /// The family is recognised by the ecosystem catalog and its
    /// containers are measured as whole artifact rows, but nothing
    /// identifies their interior yet.
    Planned,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::Implemented => "implemented",
            Self::Planned => "planned",
        }
    }
}

/// One ecosystem family's published capability claim.
#[derive(Debug, Clone)]
pub struct MatrixEntry {
    /// Equal to the adapter id when `status` is
    /// [`Status::Implemented`]; a family name otherwise.
    pub id: &'static str,
    pub name: &'static str,
    pub status: Status,
    /// Role families this adapter identifies. Empty for a planned row.
    pub families: &'static [RoleFamily],
    /// The layouts and versions the fixtures actually cover. Not "all
    /// versions": a claim here is a claim a test backs.
    pub known_layouts: &'static [&'static str],
    /// What this adapter cannot attribute, in its own words. #64 asks
    /// for attribution limits as a first-class column because "supported"
    /// with no limits is the claim nobody can keep.
    pub attribution_limits: &'static [&'static str],
    /// The granularity at which an operation could ever be offered --
    /// separate from whether one is offered, which is `actions` below.
    pub operation_granularity: &'static str,
    /// Always `"inspection only"` in this chunk. #73 implements adapter
    /// actions; until then any other value would be a promise with no
    /// executor.
    pub actions: &'static str,
}

const INSPECTION_ONLY: &str = "inspection only";

pub const MATRIX: &[MatrixEntry] = &[
    MatrixEntry {
        id: "cargo",
        name: "Rust / Cargo",
        status: Status::Implemented,
        families: &[
            RoleFamily::Container,
            RoleFamily::Outputs,
            RoleFamily::Tests,
            RoleFamily::Intermediates,
            RoleFamily::Dependencies,
            RoleFamily::Metadata,
            RoleFamily::Residual,
        ],
        known_layouts: &[
            "target/<profile>/",
            "target/<target-triple>/<profile>/",
            "deps/, examples/, incremental/, build/, .fingerprint/",
            "custom target-dir/build-dir from .cargo/config[.toml] or CARGO_TARGET_DIR",
        ],
        attribution_limits: &[
            "Cargo's intermediate layout is an implementation detail and version-dependent",
            "per-crate dependency sizing is not attempted (#107)",
            "a command-line --target-dir override is invisible to an observer",
        ],
        operation_granularity: "profile directory, or one target's executable plus its fingerprint",
        actions: INSPECTION_ONLY,
    },
    MatrixEntry {
        id: "node",
        name: "Node.js",
        status: Status::Implemented,
        families: &[
            RoleFamily::Container,
            RoleFamily::Outputs,
            RoleFamily::Tests,
            RoleFamily::Intermediates,
            RoleFamily::Dependencies,
            RoleFamily::SharedStore,
            RoleFamily::Metadata,
            RoleFamily::Residual,
        ],
        known_layouts: &[
            "node_modules/ top-level packages, scoped @scope/name",
            "node_modules/.pnpm/ virtual store",
            "dist, build, out, .next, .nuxt, .svelte-kit, .output, .vercel/output, storybook-static",
            "coverage, .nyc_output, playwright-report, test-results",
            ".next/cache, .turbo, .parcel-cache, .cache, .vite, node_modules/.cache/<tool>",
            ".eslintcache, tsconfig.tsbuildinfo",
            "npm _cacache, pnpm content-addressable store",
        ],
        attribution_limits: &[
            "a package's identity comes from its own package.json name/version; a package.json \
             that cannot be read leaves the identity unknown",
            "workspace hoisting means a top-level package may be a dependency of any workspace \
             member, and which one is not recorded on disk",
            "pnpm store entries are content-addressed: which project links a given object is not \
             derivable from the object",
            "no build generation is inferred -- npm records none",
        ],
        operation_granularity: "one output directory, one cache directory, or one installed tree",
        actions: INSPECTION_ONLY,
    },
    MatrixEntry {
        id: "gradle",
        name: "Gradle",
        status: Status::Implemented,
        families: &[
            RoleFamily::Container,
            RoleFamily::Outputs,
            RoleFamily::Tests,
            RoleFamily::Intermediates,
            RoleFamily::SharedStore,
            RoleFamily::Metadata,
            RoleFamily::Residual,
        ],
        known_layouts: &[
            "build/classes, build/libs, build/tmp, build/reports, build/test-results, \
             build/generated, build/intermediates",
            ".gradle/ project cache",
            "<gradle-user-home>/caches/{modules-2,jars-*,transforms-*,build-cache-*}",
            "<gradle-user-home>/wrapper/dists/<dist>-<hash>",
            "<gradle-user-home>/daemon/<version>",
        ],
        attribution_limits: &[
            "build scripts and plugins are never evaluated, so a custom buildDir or a plugin's \
             own output directory is an unidentified residual",
            "a transforms-* or build-cache-* entry is keyed by a hash with no recorded inputs",
            "which project last wrote a shared cache entry is not recorded on disk",
        ],
        operation_granularity: "one project build directory, or one cache category directory",
        actions: INSPECTION_ONLY,
    },
    MatrixEntry {
        id: "maven",
        name: "Maven",
        status: Status::Implemented,
        families: &[
            RoleFamily::Container,
            RoleFamily::Outputs,
            RoleFamily::Tests,
            RoleFamily::Intermediates,
            RoleFamily::SharedStore,
            RoleFamily::Metadata,
            RoleFamily::Residual,
        ],
        known_layouts: &[
            "target/{classes,test-classes,generated-sources,surefire-reports,failsafe-reports,\
             site}",
            "target/*.jar, *.war",
            "<local-repository>/<group>/<artifact>/<version>/",
            "_remote.repositories, *.lastUpdated, maven-metadata-local.xml origin evidence",
        ],
        attribution_limits: &[
            "a repository artifact with no _remote.repositories and no maven-metadata-local.xml \
             has unknown origin: swamp never promises it can be downloaded again",
            "POM property and parent-inherited versions are not resolved; an unresolved \
             ${property} is an explicit identity gap",
            "plugins are never evaluated, so a plugin's own output directory under target/ is an \
             unidentified residual",
        ],
        operation_granularity: "one project target directory, or one repository artifact version",
        actions: INSPECTION_ONLY,
    },
    // Families the catalog knows and no adapter identifies the interior
    // of yet. Listed so the gap is visible, with the container-level
    // measurement that *is* available named in `known_layouts`.
    MatrixEntry {
        id: "python",
        name: "Python",
        status: Status::Planned,
        families: &[],
        known_layouts: &["container-level rows only: .venv, __pycache__, build, dist, *.egg-info"],
        attribution_limits: &["no interior identification"],
        operation_granularity: "whole artifact row",
        actions: INSPECTION_ONLY,
    },
    MatrixEntry {
        id: "go",
        name: "Go",
        status: Status::Planned,
        families: &[],
        known_layouts: &["container-level rows only: vendor, bin, GOCACHE, GOMODCACHE"],
        attribution_limits: &["no interior identification"],
        operation_granularity: "whole artifact row",
        actions: INSPECTION_ONLY,
    },
    MatrixEntry {
        id: "xcode-swift",
        name: "Xcode / Swift",
        status: Status::Planned,
        families: &[],
        known_layouts: &[
            "container-level rows only: DerivedData, .build, Archives, iOS DeviceSupport",
        ],
        attribution_limits: &["no interior identification"],
        operation_granularity: "whole artifact row",
        actions: INSPECTION_ONLY,
    },
    MatrixEntry {
        id: "android",
        name: "Android",
        status: Status::Planned,
        families: &[],
        known_layouts: &[
            "container-level rows only: .cxx, .externalNativeBuild, captures, \
                          ~/.android/avd",
        ],
        attribution_limits: &["no interior identification"],
        operation_granularity: "whole artifact row",
        actions: INSPECTION_ONLY,
    },
    MatrixEntry {
        id: "docker-buildkit",
        name: "Docker / BuildKit",
        status: Status::Planned,
        families: &[],
        known_layouts: &[
            "daemon-reported objects only (crate::docker); never filesystem provenance",
        ],
        attribution_limits: &[
            "Docker byte accounting comes from the daemon and is not a filesystem measurement",
        ],
        operation_granularity: "daemon object",
        actions: INSPECTION_ONLY,
    },
];

/// The ids of families with a registered adapter.
pub fn implemented_ids() -> Vec<&'static str> {
    MATRIX
        .iter()
        .filter(|e| e.status == Status::Implemented)
        .map(|e| e.id)
        .collect()
}

pub fn get(id: &str) -> Option<&'static MatrixEntry> {
    MATRIX.iter().find(|e| e.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_row_claims_an_action() {
        for e in MATRIX {
            assert_eq!(
                e.actions, INSPECTION_ONLY,
                "{} claims `{}`: no build adapter implements an action, so any other value is a \
                 promise with no executor (#73 is where actions land)",
                e.id, e.actions
            );
        }
    }

    #[test]
    fn every_implemented_row_states_its_attribution_limits() {
        for e in MATRIX.iter().filter(|e| e.status == Status::Implemented) {
            assert!(
                !e.attribution_limits.is_empty(),
                "{} claims support with no stated limits; \"supported\" with no limits is the \
                 claim nobody can keep",
                e.id
            );
            assert!(!e.families.is_empty(), "{} identifies no families", e.id);
            assert!(!e.known_layouts.is_empty(), "{} names no layouts", e.id);
        }
    }

    #[test]
    fn ids_are_unique_and_kebab_case() {
        let mut ids: Vec<&str> = MATRIX.iter().map(|e| e.id).collect();
        let before = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(before, ids.len(), "duplicate matrix id");
        for id in ids {
            assert!(
                id.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
                "matrix id {id} is not kebab-case; ids must match module names and docs rows"
            );
        }
    }
}
