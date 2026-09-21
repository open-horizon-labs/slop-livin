//! First-class external/shared storage units (#43): Cargo registry,
//! rustup toolchains, Homebrew, and every future detector-resolved
//! location that is not a project/worktree walk boundary. An external
//! unit's identity is independent of any project or worktree -- it
//! exists whether or not any project happens to reference it -- and it
//! keeps size/growth/presence/regrowth history under the same
//! current+reverse-delta retention semantics as everything else in the
//! growth store (see `growth::observe_and_annotate_external`, a new key
//! family in the existing store, never a second store).
//!
//! Consumer associations (project/tool <-> external unit) are evidence
//! a *separate* system supplies (#57's job for real ecosystem-sourced
//! evidence: manifests, lockfiles, Docker joins). This module's own
//! association sidecar (`associate_consumer`/`dissociate_consumer`) is a
//! deliberately minimal, explicit, non-evidence-based mechanism that
//! exists to prove and test the required shape (zero/one/many consumers,
//! counted once, association changes never duplicate the unit or reset
//! its history) without inventing a competing discovery pipeline.
//!
//! Action boundary (#43's acceptance criterion 5): `actions::unit_from_external`
//! and `actions::propose_external` let a plan *name* an external unit;
//! `actions::execute*` refuses every one of them unconditionally on
//! `PlanUnit::external_category`. Nothing in this module or in the
//! detector registry ever authorizes removing external storage.

use crate::locations::{LocationStatus, Provenance, StorageCategory};
use crate::report::ArtifactKind;
use crate::scope::EffectiveScope;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// A declared consumer of an external unit: which project/tool refers to
/// it, and why. Evidence-free in this chunk (see module docs); `label`
/// is free text (a project id, a tool name, or a human note) rather than
/// a typed reference, since #57 owns the real evidence-sourced version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalConsumer {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// One measured external storage unit: identity independent of any
/// project/worktree, size/growth/regrowth from the growth store, zero or
/// more declared consumers, never duplicated or reset by an association
/// change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalUnit {
    pub detector_id: String,
    pub detector_name: String,
    pub category: StorageCategory,
    pub provenance: Provenance,
    /// Canonical, absolute path this unit measures as one whole-directory
    /// unit (never walked for project/worktree structure -- an external
    /// unit is opaque from this chunk's point of view).
    pub path: PathBuf,
    pub bytes: u64,
    /// Whether the unit contains hardlinked files (conservative default
    /// `true` mirrors artifact rows -- see `ArtifactRow::hardlinked`).
    pub hardlinked: bool,
    pub growth_bytes: Option<i64>,
    pub regrowth_count: u32,
    pub observed_at: u64,
    pub consumers: Vec<ExternalConsumer>,
    /// Set when this unit's coverage this pass was incomplete (access
    /// lost, not measured) rather than absent -- distinct from simply
    /// not appearing in the list at all (a genuinely gone unit).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Decision evidence (#53): activity/consumer/current-use facts for
    /// this unit -- see `crate::evidence`. Populated from already-
    /// collected facts (this unit's own `consumers`, its `growth_bytes`
    /// history); never a new per-unit scan on every report.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<crate::evidence::Evidence>,
}

/// Turns this unit's existing declared-consumer sidecar
/// (`associate_consumer`/`dissociate_consumer`) into the shared
/// decision-evidence contract (#53/#57), so a manually-declared
/// consumer and a lockfile/manifest-sourced one (#57's real ecosystem
/// evidence, once wired) render identically. Reuses data already
/// loaded for `ExternalUnit::consumers`; never a new scan.
fn consumers_evidence(consumers: &[ExternalConsumer]) -> Vec<crate::evidence::Evidence> {
    consumers
        .iter()
        .map(|c| {
            let ev = crate::evidence::Evidence::known(
                crate::evidence::FactKind::Consumer,
                crate::evidence::FactSubtype::DeclaredConsumer,
                crate::evidence::FactValue::Text(c.label.clone()),
                crate::evidence::EvidenceSource::ConfigDeclaration {
                    path: "swamp consumer association".to_string(),
                },
                crate::entities::now(),
            );
            match &c.note {
                Some(n) => ev.with_note(n.clone()),
                None => ev,
            }
        })
        .collect()
}

/// The stable identity string used both for the growth-store row key and
/// the consumer-association sidecar key: `(detector_id, category,
/// device, canonical path)`, exactly the growth store's own key family.
pub fn unit_key(detector_id: &str, category: StorageCategory, device: u64, path: &Path) -> String {
    crate::growth::external_row_key(
        detector_id,
        category_str(category),
        device,
        &path.display().to_string(),
    )
}

fn category_str(c: StorageCategory) -> &'static str {
    match c {
        StorageCategory::Installation => "installation",
        StorageCategory::Downloads => "downloads",
        StorageCategory::Cache => "cache",
        StorageCategory::LocalState => "local-state",
        StorageCategory::Environments => "environments",
        StorageCategory::BuildOutput => "build-output",
        StorageCategory::Models => "models",
        StorageCategory::Unclassified => "unclassified",
    }
}

fn device_of(path: &Path) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        fs::metadata(path).map(|m| m.dev()).unwrap_or(0)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        0
    }
}

/// Discovers and measures every external unit in `scope`: one whole-
/// directory measurement per `Resolved`, currently-present, non-scan-root
/// detector location. The `builtin-defaults` detector is skipped -- it
/// proposes ordinary scan roots (`~/src`, ...), not external storage --
/// so nothing here duplicates `report_scope`'s walked/unowned totals.
///
/// When `swamp_dir` is `Some`, this observes (persists to the growth
/// store) when `observe` is true, or reads existing history read-only
/// otherwise. `swamp_dir: None` returns units with `growth_bytes: None`
/// and `regrowth_count: 0` (no store to consult), matching the
/// artifact-row read-only contract elsewhere.
/// One candidate detector location, before it is known whether it can be
/// measured this pass.
struct Candidate {
    detector_id: String,
    detector_name: String,
    category: StorageCategory,
    provenance: Provenance,
    path: PathBuf,
}

/// A candidate that was actually measured this pass, keyed by its
/// growth-store row key, holding what `discover_and_measure` needs to
/// build the final `ExternalUnit` after growth annotation.
struct MeasuredUnit {
    detector_id: String,
    detector_name: String,
    category: StorageCategory,
    provenance: Provenance,
    path: PathBuf,
    bytes: u64,
    hardlinked: bool,
}

pub fn discover_and_measure(
    scope: &EffectiveScope,
    swamp_dir: Option<&Path>,
    observe: bool,
    observed_at: u64,
    retention_days: u64,
    since_secs: u64,
) -> Result<Vec<ExternalUnit>> {
    let mut candidates: Vec<Candidate> = Vec::new();
    for summary in &scope.detectors {
        if summary.detector_id == crate::locations::builtin::BUILTIN_DEFAULTS_DETECTOR_ID {
            continue;
        }
        for loc in &summary.locations {
            if loc.status != LocationStatus::Resolved {
                continue;
            }
            let Some(path) = &loc.path else { continue };
            candidates.push(Candidate {
                detector_id: summary.detector_id.clone(),
                detector_name: summary.name.clone(),
                category: loc.category,
                provenance: loc.provenance.clone(),
                path: path.clone(),
            });
        }
    }

    // Canonicalize once up front and de-duplicate exact (category,
    // canonical path) repeats -- e.g. a symlinked alias, or two
    // detectors independently conventionalizing the same real directory
    // -- before deciding what to measure, so a duplicate registration
    // never becomes two units for the same bytes (#45's "duplicate
    // registrations without double counting").
    let mut canon_candidates: Vec<(Candidate, PathBuf)> = Vec::new();
    {
        let mut seen: HashSet<(StorageCategory, PathBuf)> = HashSet::new();
        for c in candidates {
            let canonical = fs::canonicalize(&c.path).unwrap_or_else(|_| c.path.clone());
            if !seen.insert((c.category, canonical.clone())) {
                continue;
            }
            canon_candidates.push((c, canonical));
        }
    }

    let mut units: Vec<ExternalUnit> = Vec::new();
    let mut observed: Vec<crate::growth::ObservedExternal> = Vec::new();
    let mut protected_keys: HashSet<String> = HashSet::new();
    let mut meta_by_key: HashMap<String, MeasuredUnit> = HashMap::new();

    for (idx, (candidate, canonical)) in canon_candidates.iter().enumerate() {
        let Candidate {
            detector_id,
            detector_name,
            category,
            provenance,
            path: _,
        } = candidate;
        let category = *category;
        let detector_id = detector_id.clone();
        let detector_name = detector_name.clone();
        let provenance = provenance.clone();
        let canonical = canonical.clone();
        // Other resolved locations strictly nested inside this one are
        // measured as their own external units; excluding them here is
        // what makes this location's own measurement stop double as
        // "this location plus everything separately counted under it"
        // (the external-location double-measurement fix alongside
        // #45-#49: e.g. Cargo home's registry/git subtrees, mise's
        // installs/downloads/plugins/shims, or a model store's blobs).
        let nested_exclusions: Vec<PathBuf> = canon_candidates
            .iter()
            .enumerate()
            .filter(|(j, (_, other_canonical))| {
                *j != idx
                    && other_canonical != &canonical
                    && other_canonical.starts_with(&canonical)
            })
            .map(|(_, (_, other_canonical))| other_canonical.clone())
            .collect();
        let device = device_of(&canonical);
        let key = unit_key(&detector_id, category, device, &canonical);

        match fs::symlink_metadata(&canonical) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // Genuinely absent: no candidate this pass. If it was
                // measured before, the growth store's own not-seen sweep
                // tombstones it correctly (a real removal, e.g. the tool
                // was uninstalled and its whole home deleted).
                continue;
            }
            Err(_) => {
                // Exists but could not be statted (permission on a
                // parent directory): protect it from tombstoning, same
                // contract as #42's `compute_unconfirmed_worktrees`, and
                // skip measuring it this pass rather than guessing.
                protected_keys.insert(key);
                continue;
            }
            Ok(meta) if meta.is_dir() && fs::read_dir(&canonical).is_err() => {
                // `stat` succeeds (traversal to the path itself needs
                // only the *parent's* execute bit) but the directory's
                // own contents cannot be listed (e.g. `chmod 000`): the
                // path still exists, it just cannot be measured this
                // pass. Same protection as the case above.
                protected_keys.insert(key);
                continue;
            }
            Ok(_) => {}
        }

        let row = crate::walk::resize_artifact_excluding(
            &canonical,
            ArtifactKind::Unknown,
            observed_at,
            &nested_exclusions,
        );
        observed.push(crate::growth::ObservedExternal {
            key: key.clone(),
            detector_id: detector_id.clone(),
            category: category_str(category).to_string(),
            device,
            path: canonical.display().to_string(),
            bytes: row.bytes,
            hardlinked: row.hardlinked,
        });
        meta_by_key.insert(
            key,
            MeasuredUnit {
                detector_id,
                detector_name,
                category,
                provenance,
                path: canonical,
                bytes: row.bytes,
                hardlinked: row.hardlinked,
            },
        );
    }

    let annotations: HashMap<String, (Option<i64>, u32)> = match swamp_dir {
        Some(dir) if observe => crate::growth::observe_and_annotate_external(
            dir,
            &observed,
            &protected_keys,
            observed_at,
            retention_days,
            since_secs,
        )?,
        Some(dir) => {
            let keys: Vec<String> = observed.iter().map(|o| o.key.clone()).collect();
            crate::growth::annotate_readonly_external(
                dir,
                &keys,
                observed_at,
                retention_days,
                since_secs,
            )?
        }
        None => HashMap::new(),
    };

    let consumers_by_key = swamp_dir
        .map(load_all_consumers)
        .transpose()?
        .unwrap_or_default();

    for (
        key,
        MeasuredUnit {
            detector_id,
            detector_name,
            category,
            provenance,
            path,
            bytes,
            hardlinked,
        },
    ) in meta_by_key
    {
        let (growth_bytes, regrowth_count) = annotations.get(&key).copied().unwrap_or((None, 0));
        let consumers = consumers_by_key.get(&key).cloned().unwrap_or_default();
        let evidence = consumers_evidence(&consumers);
        units.push(ExternalUnit {
            detector_id,
            detector_name,
            category,
            provenance,
            path,
            bytes,
            hardlinked,
            growth_bytes,
            regrowth_count,
            observed_at,
            consumers,
            note: None,
            evidence,
        });
    }
    for key in &protected_keys {
        // A protected (inaccessible-this-pass) unit still needs to be
        // visible -- "unknown", never silently dropped -- but there is
        // no fresh measurement to report; fall back to its last known
        // value from the store, when there is one.
        if let Some((detector_id, category_s, _device, path)) = parse_external_key(key) {
            let path_buf = PathBuf::from(path);
            let last = swamp_dir.and_then(|dir| last_known_external(dir, key).ok().flatten());
            let consumers = swamp_dir
                .map(|dir| load_consumers(dir, key))
                .transpose()?
                .unwrap_or_default();
            let evidence = consumers_evidence(&consumers);
            units.push(ExternalUnit {
                detector_id: detector_id.clone(),
                detector_name: scope
                    .detectors
                    .iter()
                    .find(|d| d.detector_id == detector_id)
                    .map(|d| d.name.clone())
                    .unwrap_or_else(|| detector_id.clone()),
                category: category_from_str(&category_s).unwrap_or(StorageCategory::Unclassified),
                provenance: Provenance::BuiltinConvention,
                path: path_buf,
                bytes: last.as_ref().map(|r| r.0).unwrap_or(0),
                hardlinked: true,
                growth_bytes: None,
                regrowth_count: last.as_ref().map(|r| r.1).unwrap_or(0),
                observed_at,
                consumers,
                note: Some("coverage incomplete this pass: could not be read".to_string()),
                evidence,
            });
        }
    }

    Ok(units)
}

fn category_from_str(s: &str) -> Option<StorageCategory> {
    Some(match s {
        "installation" => StorageCategory::Installation,
        "downloads" => StorageCategory::Downloads,
        "cache" => StorageCategory::Cache,
        "local-state" => StorageCategory::LocalState,
        "environments" => StorageCategory::Environments,
        "build-output" => StorageCategory::BuildOutput,
        "models" => StorageCategory::Models,
        "unclassified" => StorageCategory::Unclassified,
        _ => return None,
    })
}

fn parse_external_key(key: &str) -> Option<(String, String, u64, String)> {
    let mut parts = key.split('\u{1}');
    let detector_id = parts.next()?.to_string();
    let category = parts.next()?.to_string();
    let device: u64 = parts.next()?.parse().ok()?;
    let path = parts.next()?.to_string();
    Some((detector_id, category, device, path))
}

fn last_known_external(swamp_dir: &Path, key: &str) -> Result<Option<(u64, u32)>> {
    crate::growth::peek_external_current(swamp_dir, key)
}

// ---------------------------------------------------------------------
// Consumer associations (#43 acceptance: zero/one/many, counted once,
// association changes never duplicate the unit or reset its history).
// A small JSON sidecar, deliberately separate from the byte-history
// Parquet store: touching it can never affect growth/regrowth.
// ---------------------------------------------------------------------

fn consumers_path(swamp_dir: &Path) -> PathBuf {
    swamp_dir.join("external_consumers.json")
}

fn load_all_consumers(swamp_dir: &Path) -> Result<HashMap<String, Vec<ExternalConsumer>>> {
    let path = consumers_path(swamp_dir);
    match fs::read_to_string(&path) {
        Ok(text) => Ok(serde_json::from_str(&text).unwrap_or_default()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(HashMap::new()),
        Err(e) => Err(e.into()),
    }
}

fn save_all_consumers(
    swamp_dir: &Path,
    map: &HashMap<String, Vec<ExternalConsumer>>,
) -> Result<()> {
    fs::create_dir_all(swamp_dir)?;
    let text = serde_json::to_string_pretty(map)?;
    fs::write(consumers_path(swamp_dir), text)?;
    Ok(())
}

/// This unit's declared consumers, `[]` when none are recorded.
pub fn load_consumers(swamp_dir: &Path, key: &str) -> Result<Vec<ExternalConsumer>> {
    Ok(load_all_consumers(swamp_dir)?
        .remove(key)
        .unwrap_or_default())
}

/// Associates `label` with the external unit identified by `key`.
/// Idempotent (associating the same label twice is a no-op, not a
/// duplicate entry). Never touches the growth store: byte history and
/// regrowth for this unit are completely unaffected by this call.
pub fn associate_consumer(
    swamp_dir: &Path,
    key: &str,
    label: &str,
    note: Option<&str>,
) -> Result<()> {
    let mut map = load_all_consumers(swamp_dir)?;
    let entry = map.entry(key.to_string()).or_default();
    if let Some(existing) = entry.iter_mut().find(|c| c.label == label) {
        existing.note = note.map(str::to_string);
    } else {
        entry.push(ExternalConsumer {
            label: label.to_string(),
            note: note.map(str::to_string),
        });
    }
    save_all_consumers(swamp_dir, &map)
}

/// Removes `label` from the external unit identified by `key`, if
/// present. Never touches the growth store.
pub fn dissociate_consumer(swamp_dir: &Path, key: &str, label: &str) -> Result<()> {
    let mut map = load_all_consumers(swamp_dir)?;
    if let Some(entry) = map.get_mut(key) {
        entry.retain(|c| c.label != label);
        if entry.is_empty() {
            map.remove(key);
        }
    }
    save_all_consumers(swamp_dir, &map)
}

/// Sum of every external unit's bytes, for `reconciliation` -- counted
/// once per unit regardless of how many consumers reference it (a
/// consumer association never duplicates the unit).
pub fn total_bytes(units: &[ExternalUnit]) -> u64 {
    units.iter().map(|u| u.bytes).sum()
}
