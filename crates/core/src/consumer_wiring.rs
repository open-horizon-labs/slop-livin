//! Live wiring of `toolchain_declarations.rs` (#56) and
//! `external_associations.rs` (#57) into the production report /
//! external-unit pipeline.
//!
//! Both modules were already complete, real and unit-tested (see their
//! own module docs); what was missing (named in
//! `.oh/sessions/2026-09-21-decision-evidence.md`'s follow-ups) was a
//! caller that actually runs them against real worktrees on every
//! report, with a real per-worktree cache so unchanged worktrees never
//! re-parse declaration/lockfile text on every call. This module is
//! that caller.
//!
//! Cost shape (#56/#57's own bounded-cost requirement): one bounded
//! `fs::metadata` per known declaration/lockfile filename per worktree
//! to build a cheap fingerprint; a full re-parse only when that
//! fingerprint changes (a declaration/lockfile file was added, removed,
//! or modified). Joining a parsed identity against a shared store is a
//! single `Path::exists()` hash lookup (see
//! `external_associations::cargo_registry_entry_exists` and its
//! siblings), never an enumeration of the store's contents. Nothing
//! here is written to the byte-history Parquet store; the cache is a
//! small JSON sidecar under the current-state `${SWAMP_DIR}`, exactly
//! like `external.rs`'s existing `external_consumers.json`.
//!
//! Entry point: [`attach_associations`], called once by every caller
//! that has *both* a computed [`Report`] and a computed
//! `Vec<ExternalUnit>` in hand at the same time (the CLI's `report
//! --view external`/`propose` routes and the TUI's startup) --
//! mirroring how `agents::discover_and_measure` already takes
//! `project_worktrees` from that same already-computed report rather
//! than re-walking anything.

use crate::external::ExternalUnit;
use crate::external_associations;
use crate::locations::StorageCategory;
use crate::report::{ArtifactKind, Report};
use crate::toolchain_declarations::{self, ProjectDeclarationSources, ToolVersionDeclaration};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

fn now() -> u64 {
    crate::entities::now()
}

// ---------------------------------------------------------------------
// Per-worktree mtime fingerprint (shared shape for both caches).
// ---------------------------------------------------------------------

const DECLARATION_FILENAMES: &[&str] = &[
    ".tool-versions",
    ".mise.toml",
    "mise.toml",
    ".python-version",
    ".ruby-version",
    ".nvmrc",
    ".node-version",
    "rust-toolchain",
    "rust-toolchain.toml",
];

const LOCKFILE_FILENAMES: &[&str] = &[
    "Cargo.lock",
    "package-lock.json",
    "pnpm-lock.yaml",
    "go.sum",
    "gradle.lockfile",
    "pom.xml",
];

#[cfg(unix)]
fn mtime_secs(meta: &fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    meta.mtime().max(0) as u64
}
#[cfg(not(unix))]
fn mtime_secs(meta: &fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Every present file's own mtime, from the fixed candidate list --
/// never a directory walk. Two calls with the same result mean "nothing
/// this module reads has changed" (added/removed/modified files all
/// change this).
fn fingerprint(root: &Path, names: &[&str]) -> Vec<(String, u64)> {
    let mut out: Vec<(String, u64)> = names
        .iter()
        .filter_map(|name| {
            fs::metadata(root.join(name))
                .ok()
                .map(|m| (name.to_string(), mtime_secs(&m)))
        })
        .collect();
    out.sort();
    out
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct DeclarationCacheEntry {
    fingerprint: Vec<(String, u64)>,
    declarations: Vec<ToolVersionDeclaration>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CachedIdentity {
    ecosystem: String,
    name: String,
    version: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct DependencyCacheEntry {
    fingerprint: Vec<(String, u64)>,
    identities: Vec<CachedIdentity>,
    /// Named parse failures this worktree's lockfiles produced this
    /// pass (#57: "an unparseable lockfile is a named evidence gap,
    /// never a silent empty list").
    parse_errors: Vec<(String, String)>, // (ecosystem, message)
}

type DeclarationCacheMap = HashMap<String, DeclarationCacheEntry>;
type DependencyCacheMap = HashMap<String, DependencyCacheEntry>;

fn declaration_cache_path(swamp_dir: &Path) -> PathBuf {
    swamp_dir.join("toolchain_declarations_cache.json")
}
fn dependency_cache_path(swamp_dir: &Path) -> PathBuf {
    swamp_dir.join("dependency_identities_cache.json")
}

fn load_json<T: Default + serde::de::DeserializeOwned>(path: &Path) -> T {
    fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}
fn save_json<T: Serialize>(path: &Path, value: &T) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string_pretty(value) {
        let _ = fs::write(path, text);
    }
}

// ---------------------------------------------------------------------
// Toolchain declarations (#56): real file reads, cached per worktree.
// ---------------------------------------------------------------------

fn read_declarations(root: &Path) -> Vec<ToolVersionDeclaration> {
    let read = |name: &str| fs::read_to_string(root.join(name)).ok();
    let tool_versions = read(".tool-versions");
    let mise_toml = read(".mise.toml").or_else(|| read("mise.toml"));
    let python_version = read(".python-version");
    let ruby_version = read(".ruby-version");
    let nvmrc = read(".nvmrc");
    let node_version = read(".node-version");
    let rust_toolchain = read("rust-toolchain");
    let rust_toolchain_toml = read("rust-toolchain.toml");
    let sources = ProjectDeclarationSources {
        tool_versions: tool_versions.as_deref(),
        mise_toml: mise_toml.as_deref(),
        python_version: python_version.as_deref(),
        ruby_version: ruby_version.as_deref(),
        nvmrc: nvmrc.as_deref(),
        node_version: node_version.as_deref(),
        rust_toolchain: rust_toolchain.as_deref(),
        rust_toolchain_toml: rust_toolchain_toml.as_deref(),
        java_version: None,
    };
    toolchain_declarations::project_declarations(root, &sources)
}

/// Declarations for one worktree, reusing the cache when the worktree's
/// declaration files have not changed since the last call.
fn cached_declarations(
    worktree: &Path,
    cache: &mut DeclarationCacheMap,
) -> Vec<ToolVersionDeclaration> {
    let key = worktree.display().to_string();
    let fp = fingerprint(worktree, DECLARATION_FILENAMES);
    if let Some(entry) = cache.get(&key)
        && entry.fingerprint == fp
    {
        return entry.declarations.clone();
    }
    let declarations = read_declarations(worktree);
    cache.insert(
        key,
        DeclarationCacheEntry {
            fingerprint: fp,
            declarations: declarations.clone(),
        },
    );
    declarations
}

// ---------------------------------------------------------------------
// Dependency identities (#57): real lockfile reads, cached per worktree.
// ---------------------------------------------------------------------

fn read_identities(root: &Path) -> (Vec<CachedIdentity>, Vec<(String, String)>) {
    let mut out = Vec::new();
    let mut errors = Vec::new();
    if let Ok(text) = fs::read_to_string(root.join("Cargo.lock")) {
        match external_associations::parse_cargo_lock(&text) {
            Ok(ids) => out.extend(ids.into_iter().map(|i| CachedIdentity {
                ecosystem: i.ecosystem.to_string(),
                name: i.name,
                version: i.version,
            })),
            Err(e) => errors.push(("cargo".to_string(), e)),
        }
    }
    if let Ok(text) = fs::read_to_string(root.join("package-lock.json")) {
        match external_associations::parse_package_lock_json(&text) {
            Ok(ids) => out.extend(ids.into_iter().map(|i| CachedIdentity {
                ecosystem: i.ecosystem.to_string(),
                name: i.name,
                version: i.version,
            })),
            Err(e) => errors.push(("npm".to_string(), e)),
        }
    }
    if let Ok(text) = fs::read_to_string(root.join("pnpm-lock.yaml")) {
        out.extend(
            external_associations::parse_pnpm_lock_yaml(&text)
                .into_iter()
                .map(|i| CachedIdentity {
                    ecosystem: i.ecosystem.to_string(),
                    name: i.name,
                    version: i.version,
                }),
        );
    }
    if let Ok(text) = fs::read_to_string(root.join("go.sum")) {
        out.extend(
            external_associations::parse_go_sum(&text)
                .into_iter()
                .map(|i| CachedIdentity {
                    ecosystem: i.ecosystem.to_string(),
                    name: i.name,
                    version: i.version,
                }),
        );
    }
    if let Ok(text) = fs::read_to_string(root.join("gradle.lockfile")) {
        out.extend(
            external_associations::parse_gradle_lockfile(&text)
                .into_iter()
                .map(|i| CachedIdentity {
                    ecosystem: i.ecosystem.to_string(),
                    name: i.name,
                    version: i.version,
                }),
        );
    }
    if let Ok(text) = fs::read_to_string(root.join("pom.xml")) {
        match external_associations::parse_pom_xml(&text) {
            Ok(ids) => out.extend(ids.into_iter().map(|i| CachedIdentity {
                ecosystem: i.ecosystem.to_string(),
                name: i.name,
                version: i.version,
            })),
            Err(e) => errors.push(("maven".to_string(), e)),
        }
    }
    (out, errors)
}

fn cached_identities(
    worktree: &Path,
    cache: &mut DependencyCacheMap,
) -> (Vec<CachedIdentity>, Vec<(String, String)>) {
    let key = worktree.display().to_string();
    let fp = fingerprint(worktree, LOCKFILE_FILENAMES);
    if let Some(entry) = cache.get(&key)
        && entry.fingerprint == fp
    {
        return (entry.identities.clone(), entry.parse_errors.clone());
    }
    let (identities, errors) = read_identities(worktree);
    cache.insert(
        key,
        DependencyCacheEntry {
            fingerprint: fp,
            identities: identities.clone(),
            parse_errors: errors.clone(),
        },
    );
    (identities, errors)
}

// ---------------------------------------------------------------------
// Matching declarations against measured installations (#56).
// ---------------------------------------------------------------------

fn readdir_names(dir: &Path) -> Vec<String> {
    fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().is_dir())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// Installed version identifiers for one declaration's `(manager,
/// tool)`, paired with the index into `units` each one came from -- so
/// a real match can be attributed back to the specific external unit(s)
/// that measured it, without guessing which of possibly several
/// (asdf+mise, rbenv+rvm+ruby-install) actually holds it.
fn add_flat(out: &mut Vec<(String, usize)>, units: &[ExternalUnit], detector_id: &str) {
    for (i, u) in units.iter().enumerate() {
        if u.detector_id == detector_id && u.category == StorageCategory::Installation {
            out.extend(readdir_names(&u.path).into_iter().map(|n| (n, i)));
        }
    }
}

fn add_per_tool(
    out: &mut Vec<(String, usize)>,
    units: &[ExternalUnit],
    detector_id: &str,
    tool: &str,
) {
    for (i, u) in units.iter().enumerate() {
        if u.detector_id == detector_id && u.category == StorageCategory::Installation {
            let tool_dir = u.path.join(tool);
            if tool_dir.is_dir() {
                out.extend(readdir_names(&tool_dir).into_iter().map(|n| (n, i)));
            }
        }
    }
}

fn installed_versions_for(
    manager: &str,
    tool: &str,
    units: &[ExternalUnit],
) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    match manager {
        "pyenv" => add_flat(&mut out, units, crate::locations::pyenv::PYENV_DETECTOR_ID),
        "rbenv-or-rvm" => {
            add_flat(&mut out, units, crate::locations::rbenv::RBENV_DETECTOR_ID);
            add_flat(&mut out, units, crate::locations::rvm::RVM_DETECTOR_ID);
            add_flat(
                &mut out,
                units,
                crate::locations::ruby_install::RUBY_INSTALL_DETECTOR_ID,
            );
        }
        "nvm" => add_flat(&mut out, units, crate::locations::nvm::NVM_DETECTOR_ID),
        "rustup" => add_flat(
            &mut out,
            units,
            crate::locations::rustup::RUSTUP_DETECTOR_ID,
        ),
        "asdf-or-mise" => {
            add_per_tool(
                &mut out,
                units,
                crate::locations::asdf::ASDF_DETECTOR_ID,
                tool,
            );
            add_per_tool(
                &mut out,
                units,
                crate::locations::mise::MISE_DETECTOR_ID,
                tool,
            );
        }
        "mise" => add_per_tool(
            &mut out,
            units,
            crate::locations::mise::MISE_DETECTOR_ID,
            tool,
        ),
        // "jenv-or-sdkman" and anything else: no detector measures it in
        // this catalog. An empty installed list is exactly the honest
        // answer `match_version`'s own docs already give it --
        // `NoMatchingInstallation`/`UnresolvedRange`, "the installation
        // may be outside scanned scope" -- never a fabricated source.
        _ => {}
    }
    out
}

/// Resolves one declaration, applying rustup's channel-to-host-triple
/// widening ([`toolchain_declarations::resolve_rustup_channel_to_dir`])
/// only for the `rustup` manager, where the installed directory naming
/// convention (`<channel>-<host-triple>`) is not the dotted-version
/// convention `match_version`'s own alias/range logic models.
fn resolve_declaration(
    decl: ToolVersionDeclaration,
    named: &[(String, usize)],
) -> (toolchain_declarations::ToolVersionAssociation, Vec<usize>) {
    let names: Vec<String> = named.iter().map(|(n, _)| n.clone()).collect();
    let assoc = if decl.manager == "rustup" && !names.contains(&decl.version_spec) {
        match toolchain_declarations::resolve_rustup_channel_to_dir(&decl.version_spec, &names) {
            Some(dir) => toolchain_declarations::resolve_explicit(
                decl,
                toolchain_declarations::VersionMatch::Exact { installed: dir },
            ),
            None => toolchain_declarations::resolve(decl, &names),
        }
    } else {
        toolchain_declarations::resolve(decl, &names)
    };
    let contributing_units: Vec<usize> = match &assoc.match_result {
        toolchain_declarations::VersionMatch::Exact { installed } => named
            .iter()
            .filter(|(n, _)| n == installed)
            .map(|(_, i)| *i)
            .collect(),
        _ => Vec::new(),
    };
    (assoc, contributing_units)
}

// ---------------------------------------------------------------------
// Shared-store identity joins (#57): hash-lookup existence checks, not
// enumeration -- see `external_associations`'s own module doc for why.
// ---------------------------------------------------------------------

fn path_ends_with(path: &Path, suffix: &[&str]) -> bool {
    let comps: Vec<String> = path
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    if suffix.len() > comps.len() {
        return false;
    }
    comps[comps.len() - suffix.len()..] == suffix.iter().map(|s| s.to_string()).collect::<Vec<_>>()
}

fn unit_index_by_suffix(
    units: &[ExternalUnit],
    detector_id: &str,
    category: StorageCategory,
    suffix: &[&str],
) -> Option<usize> {
    units.iter().position(|u| {
        u.detector_id == detector_id && u.category == category && path_ends_with(&u.path, suffix)
    })
}

/// The Go module cache unit's own path has no fixed relative suffix
/// (`GOMODCACHE` is a free-form override) -- but its sibling
/// `cache/download` unit's path is always `<GOMODCACHE>/cache/download`
/// (`go.rs` derives it programmatically), so the module cache is
/// identified as that sibling's grandparent rather than guessed from a
/// path shape.
fn go_modcache_unit_index(units: &[ExternalUnit]) -> Option<usize> {
    let download_path = units
        .iter()
        .find(|u| {
            u.detector_id == crate::locations::go::GO_DETECTOR_ID
                && u.category == StorageCategory::Downloads
        })
        .map(|u| u.path.clone())?;
    let modcache_path = download_path.parent()?.parent()?;
    units.iter().position(|u| {
        u.detector_id == crate::locations::go::GO_DETECTOR_ID
            && u.category == StorageCategory::Cache
            && u.path == modcache_path
    })
}

/// Which shared-store unit indices actually hold an entry for
/// `identity`, via one deterministic `Path::exists()` lookup per
/// candidate unit -- never a store enumeration. npm cacache (content-
/// addressed) and pnpm's store (handled collectively, see the caller)
/// are not resolved here: neither can be mapped to a specific entry.
fn match_identity(identity: &CachedIdentity, units: &[ExternalUnit]) -> Vec<usize> {
    let mut out = Vec::new();
    match identity.ecosystem.as_str() {
        "cargo" => {
            if let Some(i) = unit_index_by_suffix(
                units,
                crate::locations::cargo_home::CARGO_HOME_DETECTOR_ID,
                StorageCategory::Cache,
                &["registry", "src"],
            ) && external_associations::cargo_registry_entry_exists(
                &units[i].path,
                &identity.name,
                &identity.version,
            ) {
                out.push(i);
            }
        }
        "go" => {
            if let Some(i) = go_modcache_unit_index(units)
                && external_associations::go_module_cache_entry_exists(
                    &units[i].path,
                    &identity.name,
                    &identity.version,
                )
            {
                out.push(i);
            }
        }
        "gradle" => {
            if let Some(i) = unit_index_by_suffix(
                units,
                crate::locations::gradle::GRADLE_DETECTOR_ID,
                StorageCategory::Cache,
                &["caches"],
            ) && external_associations::gradle_cache_entry_exists(
                &units[i].path,
                &identity.name,
                &identity.version,
            ) {
                out.push(i);
            }
        }
        "maven" => {
            if let Some(i) = units
                .iter()
                .position(|u| u.detector_id == crate::locations::maven::MAVEN_DETECTOR_ID)
                && external_associations::maven_repo_entry_exists(
                    &units[i].path,
                    &identity.name,
                    &identity.version,
                )
            {
                out.push(i);
            }
        }
        _ => {}
    }
    out
}

// ---------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------

/// Attaches consumer evidence both ways -- installation/shared-store
/// unit <- projects declaring/depending on it; project's own `Source`
/// row -> installations/dependencies it declares -- from real per-
/// worktree declaration and lockfile reads, cached by file mtime.
/// Called once per report by every caller that has both `report` and
/// `external_units` computed (CLI `report --view external`/`propose`,
/// TUI startup); never invoked from inside the bus (external units are
/// not part of `Report` -- see `external.rs`'s own module doc).
pub fn attach_associations(
    report: &mut Report,
    external_units: &mut [ExternalUnit],
    swamp_dir: Option<&Path>,
) {
    let mut decl_cache: DeclarationCacheMap = swamp_dir
        .map(|d| load_json(&declaration_cache_path(d)))
        .unwrap_or_default();
    let mut dep_cache: DependencyCacheMap = swamp_dir
        .map(|d| load_json(&dependency_cache_path(d)))
        .unwrap_or_default();

    let project_roots: Vec<PathBuf> = report
        .projects
        .iter()
        .flat_map(|p| p.worktrees.iter())
        .map(|wt| wt.path.clone())
        .collect();

    // unit index -> evidence to append, collected during the read-only
    // matching pass below and applied once at the end (avoids mixing
    // mutable/immutable borrows of `external_units` mid-pass).
    let mut unit_evidence: HashMap<usize, Vec<crate::evidence::Evidence>> = HashMap::new();
    // unit index -> confirmed consuming project labels, deduplicated
    // and combined into one Evidence per unit at the end.
    let mut unit_consumers: HashMap<usize, Vec<String>> = HashMap::new();

    for project in &mut report.projects {
        let project_label = project.name.clone();
        for wt in &mut project.worktrees {
            let wt_path = wt.path.clone();

            // --- #56: toolchain declarations -> installations ---
            let declarations = cached_declarations(&wt_path, &mut decl_cache);
            let mut project_side_evidence = Vec::new();
            for decl in declarations {
                let named = installed_versions_for(&decl.manager, &decl.tool, external_units);
                let (assoc, contributing_units) = resolve_declaration(decl, &named);
                project_side_evidence.push(assoc.evidence.clone());
                if assoc.evidence.is_known() {
                    for u in contributing_units {
                        unit_consumers
                            .entry(u)
                            .or_default()
                            .push(project_label.clone());
                    }
                }
            }

            // --- #57: dependency lockfiles -> shared store entries ---
            let (identities, parse_errors) = cached_identities(&wt_path, &mut dep_cache);
            for (ecosystem, message) in &parse_errors {
                project_side_evidence.push(external_associations::invalid_lockfile_evidence(
                    ecosystem, &wt_path, message,
                ));
            }
            let mut has_pnpm_identity = false;
            for identity in &identities {
                if identity.ecosystem == "pnpm" {
                    has_pnpm_identity = true;
                    continue; // handled collectively below
                }
                for unit_index in match_identity(identity, external_units) {
                    unit_consumers
                        .entry(unit_index)
                        .or_default()
                        .push(project_label.clone());
                }
            }
            if has_pnpm_identity
                && let Some(i) = external_units
                    .iter()
                    .position(|u| u.detector_id == crate::locations::pnpm::PNPM_DETECTOR_ID)
            {
                unit_consumers
                    .entry(i)
                    .or_default()
                    .push(project_label.clone());
            }

            // Attach every project-side fact to this worktree's own
            // `Source` row -- "at most one Source row per worktree" is
            // this pass's single canonical attachment point.
            if let Some(source_row) = wt
                .artifacts
                .iter_mut()
                .find(|a| a.kind == ArtifactKind::Source)
            {
                source_row.evidence.extend(project_side_evidence);
            }
        }
    }

    // npm cacache: opaque content-addressed cache, once per unit
    // (never per project -- there is no per-entry attribution to make).
    for (i, u) in external_units.iter().enumerate() {
        if u.detector_id == crate::locations::npm::NPM_DETECTOR_ID {
            unit_evidence.entry(i).or_default().push(
                crate::evidence::Evidence::unknown(
                    crate::evidence::FactKind::Consumer,
                    crate::evidence::FactSubtype::DeclaredConsumer,
                    crate::evidence::EvidenceSource::Inferred {
                        basis: "npm cacache content-addressed store".into(),
                    },
                    now(),
                    "npm's cache is content-addressed (sha-keyed); a declared package name+version cannot be mapped to a specific cache entry",
                ),
            );
        }
    }

    // rustup global default (#56's "global defaults as their own role"):
    // read from the rustup LocalState unit's own `settings.toml` (real
    // ExternalUnit already resolved by this same call, not a fresh
    // Environment/env-var lookup).
    attach_rustup_global_default(external_units);

    // Xcode DerivedData -> project (#57's own listed shared-store join):
    // one `plutil` read per DerivedData subfolder, bounded by how many
    // Xcode projects have ever built locally.
    attach_xcode_associations(external_units, &project_roots, &mut unit_evidence);

    for (i, labels) in unit_consumers {
        let mut labels = labels;
        labels.sort();
        labels.dedup();
        let value = if labels.len() == 1 {
            crate::evidence::FactValue::Text(labels[0].clone())
        } else {
            crate::evidence::FactValue::List(labels.clone())
        };
        let mut ev = crate::evidence::Evidence::known(
            crate::evidence::FactKind::Consumer,
            crate::evidence::FactSubtype::DeclaredConsumer,
            value,
            crate::evidence::EvidenceSource::Inferred {
                basis: "toolchain declaration or dependency lockfile match".into(),
            },
            now(),
        );
        if labels.len() > 1 {
            ev =
                ev.with_note("declared by more than one project; a shared reference, not an error");
        }
        unit_evidence.entry(i).or_default().push(ev);
    }
    for (i, evs) in unit_evidence {
        if let Some(u) = external_units.get_mut(i) {
            u.evidence.extend(evs);
        }
    }

    if let Some(dir) = swamp_dir {
        save_json(&declaration_cache_path(dir), &decl_cache);
        save_json(&dependency_cache_path(dir), &dep_cache);
    }
}

fn attach_rustup_global_default(units: &mut [ExternalUnit]) {
    let Some(local_state_idx) = units.iter().position(|u| {
        u.detector_id == crate::locations::rustup::RUSTUP_DETECTOR_ID
            && u.category == StorageCategory::LocalState
    }) else {
        return;
    };
    let Some(toolchains_idx) = units.iter().position(|u| {
        u.detector_id == crate::locations::rustup::RUSTUP_DETECTOR_ID
            && u.category == StorageCategory::Installation
    }) else {
        return;
    };
    let settings_path = units[local_state_idx].path.join("settings.toml");
    let Some(default_toolchain) = fs::read_to_string(&settings_path)
        .ok()
        .and_then(|t| toolchain_declarations::parse_rustup_default_toolchain(&t))
    else {
        return;
    };
    let installed = readdir_names(&units[toolchains_idx].path);
    let matched = if installed.contains(&default_toolchain) {
        Some(default_toolchain.clone())
    } else {
        toolchain_declarations::resolve_rustup_channel_to_dir(&default_toolchain, &installed)
    };
    let ev = match matched {
        Some(installed_name) => crate::evidence::Evidence::known(
            crate::evidence::FactKind::Consumer,
            crate::evidence::FactSubtype::DeclaredConsumer,
            crate::evidence::FactValue::Text(installed_name),
            crate::evidence::EvidenceSource::ConfigDeclaration {
                path: settings_path.display().to_string(),
            },
            now(),
        ),
        None => crate::evidence::Evidence::unknown(
            crate::evidence::FactKind::Consumer,
            crate::evidence::FactSubtype::DeclaredConsumer,
            crate::evidence::EvidenceSource::ConfigDeclaration {
                path: settings_path.display().to_string(),
            },
            now(),
            format!("rustup default_toolchain '{default_toolchain}' matches no installed toolchain directory"),
        ),
    }
    .with_note("global default (rustup settings.toml default_toolchain), not a per-project declaration");
    units[toolchains_idx].evidence.push(ev);
}

fn attach_xcode_associations(
    units: &mut [ExternalUnit],
    project_roots: &[PathBuf],
    unit_evidence: &mut HashMap<usize, Vec<crate::evidence::Evidence>>,
) {
    let Some(idx) = unit_index_by_suffix(
        units,
        crate::locations::xcode::XCODE_DETECTOR_ID,
        StorageCategory::BuildOutput,
        &["DerivedData"],
    ) else {
        return;
    };
    let derived_data_path = units[idx].path.clone();
    for subfolder in external_associations::list_derived_data_subfolders(&derived_data_path) {
        let info_plist = subfolder.join("info.plist");
        if !info_plist.exists() {
            continue;
        }
        let name = subfolder
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        match external_associations::read_workspace_path(&info_plist) {
            Ok(workspace_path) => {
                let ev = external_associations::xcode_derived_data_association(
                    workspace_path.as_deref(),
                    project_roots,
                )
                .with_note(format!("DerivedData/{name}"));
                unit_evidence.entry(idx).or_default().push(ev);
            }
            Err(e) => {
                unit_evidence.entry(idx).or_default().push(
                    crate::evidence::Evidence::unavailable(
                        crate::evidence::FactKind::Consumer,
                        crate::evidence::FactSubtype::DeclaredConsumer,
                        crate::evidence::EvidenceSource::BuildMetadata {
                            path: info_plist.display().to_string(),
                        },
                        now(),
                        e,
                    )
                    .with_note(format!("DerivedData/{name}")),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locations::Provenance;
    use crate::report::{ArtifactRow, ProjectRow, Source, WorktreeKind, WorktreeRow};

    fn empty_artifact_row(kind: ArtifactKind, path: &Path) -> ArtifactRow {
        ArtifactRow {
            kind,
            path: path.to_path_buf(),
            bytes: 0,
            mtime_max: 0,
            ecosystem: None,
            hardlinked: false,
            dedup_stale: false,
            allocated_bytes: None,
            allocated_growth_bytes: None,
            local_bytes: 0,
            track: None,
            growth_bytes: None,
            regrowth_count: 0,
            observed_at: 0,
            confidence: crate::entities::Confidence::High,
            source: Source::new("test"),
            note: None,
            created_at: None,
            containers: Vec::new(),
            shared_with: Vec::new(),
            dangling: false,
            evidence: Vec::new(),
        }
    }

    fn external_unit(detector_id: &str, category: StorageCategory, path: &Path) -> ExternalUnit {
        ExternalUnit {
            detector_id: detector_id.to_string(),
            detector_name: detector_id.to_string(),
            category,
            provenance: Provenance::BuiltinConvention,
            path: path.to_path_buf(),
            bytes: 0,
            hardlinked: false,
            growth_bytes: None,
            regrowth_count: 0,
            observed_at: 0,
            consumers: Vec::new(),
            note: None,
            evidence: Vec::new(),
        }
    }

    fn empty_report(root: &Path, projects: Vec<ProjectRow>) -> Report {
        Report {
            observed_at: now(),
            root: root.to_path_buf(),
            projects,
            unowned: Vec::new(),
            reconciliation: crate::report::Reconciliation {
                attributed: 0,
                unowned: 0,
                walked_total: 0,
                du_total: None,
                docker_attributed: 0,
                docker_unowned: 0,
            },
            series_by_key: Default::default(),
            total_series: Vec::new(),
            series_window_secs: 0,
            notes: Vec::new(),
            dirs_by_worktree: None,
            files_by_worktree: None,
            schedule_line: None,
            summary: Default::default(),
            github_enrichment: None,
            nested_artifacts: Vec::new(),
        }
    }

    fn one_worktree_project(name: &str, root: &Path, artifacts: Vec<ArtifactRow>) -> ProjectRow {
        ProjectRow {
            project_id: name.into(),
            name: name.into(),
            worktrees: vec![WorktreeRow {
                worktree_id: name.into(),
                path: root.to_path_buf(),
                kind: WorktreeKind::Main,
                artifacts,
                signals: Vec::new(),
                branch: None,
                github: None,
                merge_complete: None,
                idle_secs: None,
            }],
            ecosystems: Vec::new(),
            remote: None,
        }
    }

    #[test]
    fn pinned_rust_toolchain_declaration_attaches_evidence_both_ways() {
        let tmp = tempfile::tempdir().unwrap();
        let project_root = tmp.path().join("proj");
        std::fs::create_dir_all(&project_root).unwrap();
        std::fs::write(project_root.join("rust-toolchain"), "1.82.0\n").unwrap();

        let rustup_home = tmp.path().join("rustup");
        std::fs::create_dir_all(rustup_home.join("toolchains/1.82.0-x86_64-apple-darwin")).unwrap();

        let mut report = empty_report(
            tmp.path(),
            vec![one_worktree_project(
                "proj",
                &project_root,
                vec![empty_artifact_row(ArtifactKind::Source, &project_root)],
            )],
        );
        let mut units = vec![external_unit(
            crate::locations::rustup::RUSTUP_DETECTOR_ID,
            StorageCategory::Installation,
            &rustup_home.join("toolchains"),
        )];

        attach_associations(&mut report, &mut units, None);

        let source_row = &report.projects[0].worktrees[0].artifacts[0];
        assert!(
            source_row
                .evidence
                .iter()
                .any(|e| e.is_known() && e.kind == crate::evidence::FactKind::Consumer),
            "project's Source row must carry a resolved installation fact: {:?}",
            source_row.evidence
        );
        assert!(
            units[0].evidence.iter().any(|e| e.is_known()),
            "installation unit must carry a resolved consuming-project fact: {:?}",
            units[0].evidence
        );
    }

    #[test]
    fn cached_declarations_reuses_result_when_files_unchanged() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(".ruby-version"), "3.2.0\n").unwrap();
        let mut cache = DeclarationCacheMap::new();
        let first = cached_declarations(tmp.path(), &mut cache);
        assert_eq!(first.len(), 1);
        // Mutate the cache entry's stored declarations directly to a
        // sentinel the real file would never produce, proving the
        // second call reused the cache instead of re-reading the file.
        cache
            .get_mut(&tmp.path().display().to_string())
            .unwrap()
            .declarations[0]
            .version_spec = "sentinel".into();
        let second = cached_declarations(tmp.path(), &mut cache);
        assert_eq!(second[0].version_spec, "sentinel");
    }

    #[test]
    fn cached_declarations_reparses_after_mtime_change() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(".ruby-version"), "3.2.0\n").unwrap();
        let mut cache = DeclarationCacheMap::new();
        let _ = cached_declarations(tmp.path(), &mut cache);
        // Simulate a real edit: change content and force a different
        // mtime (some filesystems have 1s mtime resolution).
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(tmp.path().join(".ruby-version"), "3.3.0\n").unwrap();
        let second = cached_declarations(tmp.path(), &mut cache);
        assert_eq!(second[0].version_spec, "3.3.0");
    }

    #[test]
    fn cargo_dependency_shared_by_two_projects_is_a_known_list_on_the_unit() {
        let tmp = tempfile::tempdir().unwrap();
        let registry_src = tmp.path().join("cargo/registry/src");
        std::fs::create_dir_all(registry_src.join("index.crates.io-abc/serde-1.0.203")).unwrap();

        let mk_project = |name: &str| -> ProjectRow {
            let root = tmp.path().join(name);
            std::fs::create_dir_all(&root).unwrap();
            std::fs::write(
                root.join("Cargo.lock"),
                "[[package]]\nname = \"serde\"\nversion = \"1.0.203\"\n",
            )
            .unwrap();
            one_worktree_project(
                name,
                &root,
                vec![empty_artifact_row(ArtifactKind::Source, &root)],
            )
        };
        let p1 = mk_project("project-a");
        let p2 = mk_project("project-b");

        let mut report = empty_report(tmp.path(), vec![p1, p2]);
        let mut units = vec![external_unit(
            crate::locations::cargo_home::CARGO_HOME_DETECTOR_ID,
            StorageCategory::Cache,
            &registry_src,
        )];

        attach_associations(&mut report, &mut units, None);

        let consumer_fact = units[0]
            .evidence
            .iter()
            .find(|e| e.is_known() && e.kind == crate::evidence::FactKind::Consumer)
            .expect("expected a known consumer fact on the registry/src unit");
        match &consumer_fact.status {
            crate::evidence::FactStatus::Known(crate::evidence::FactValue::List(labels)) => {
                assert_eq!(labels.len(), 2, "both projects must be named: {labels:?}");
            }
            other => panic!("expected a known list of two consumers, got {other:?}"),
        }
    }

    #[test]
    fn npm_cache_is_always_unknown_never_guessed_from_package_lock() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("proj");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("package-lock.json"),
            r#"{"packages":{"node_modules/lodash":{"version":"4.17.21"}}}"#,
        )
        .unwrap();
        let mut report = empty_report(
            tmp.path(),
            vec![one_worktree_project(
                "proj",
                &root,
                vec![empty_artifact_row(ArtifactKind::Source, &root)],
            )],
        );
        let npm_cache = tmp.path().join("npm");
        std::fs::create_dir_all(&npm_cache).unwrap();
        let mut units = vec![external_unit(
            crate::locations::npm::NPM_DETECTOR_ID,
            StorageCategory::Cache,
            &npm_cache,
        )];
        attach_associations(&mut report, &mut units, None);
        // The tempting shortcut this rejects: matching package-lock.json's
        // `lodash@4.17.21` entry back to a specific npm cacache file.
        assert!(units[0].evidence.iter().all(|e| !e.is_known()));
        assert!(units[0].evidence.iter().any(|e| matches!(
            &e.status,
            crate::evidence::FactStatus::Unknown { reason }
                if reason.contains("content-addressed")
        )));
    }
}
