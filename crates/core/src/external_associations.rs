//! Associates external build output and shared dependency caches with
//! known projects (#57). Every association here is a *declared*
//! reference (a lockfile entry, an Xcode workspace path recorded at
//! build time) -- never proof that a particular cache entry was
//! actually used at runtime. Multiple consumers, orphaned references
//! (a project no longer present), and unattributed entries are all
//! first-class outcomes; nothing here forces a single owner or
//! duplicates the underlying storage.
//!
//! Every parser operates on already-read text (never touches the
//! filesystem itself), so tests exercise the identity/matching logic
//! against fixture text without real project checkouts or package
//! caches.

use crate::evidence::{Evidence, EvidenceSource, FactKind, FactSubtype, FactValue};
use std::path::Path;

fn now() -> u64 {
    crate::entities::now()
}

// ---------------------------------------------------------------------
// Xcode DerivedData <Name>-<hash>/info.plist -> WorkspacePath
// ---------------------------------------------------------------------

/// Extracts `WorkspacePath`'s string value from an XML-plist rendering
/// of a DerivedData `info.plist` (`<key>WorkspacePath</key><string>
/// ...</string>`). Deliberately not a binary-plist parser: real
/// binary-format `info.plist` files are converted to XML first via the
/// bounded, read-only, output-only `plutil -convert xml1 -o - <path>`
/// (see [`read_workspace_path`]) -- this function only ever sees text.
pub fn parse_workspace_path_from_plist_xml(xml: &str) -> Option<String> {
    let key_pos = xml.find("<key>WorkspacePath</key>")?;
    let after_key = &xml[key_pos..];
    let string_start = after_key.find("<string>")? + "<string>".len();
    let string_end = after_key[string_start..].find("</string>")?;
    let value = &after_key[string_start..string_start + string_end];
    Some(html_unescape(value))
}

fn html_unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

/// Real read: converts `info_plist_path` to XML via `plutil` (a fixed,
/// read-only, output-only system utility -- it never rewrites the
/// source file with `-o -`) and extracts `WorkspacePath`. Any failure
/// (missing file, plutil unavailable, unexpected format) is a named
/// `Err`, never a silent `None` indistinguishable from "no workspace
/// recorded".
pub fn read_workspace_path(info_plist_path: &Path) -> Result<Option<String>, String> {
    let output = std::process::Command::new("plutil")
        .args(["-convert", "xml1", "-o", "-"])
        .arg(info_plist_path)
        .output()
        .map_err(|e| format!("plutil could not be started: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "plutil failed to convert {}: {}",
            info_plist_path.display(),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let xml = String::from_utf8_lossy(&output.stdout);
    Ok(parse_workspace_path_from_plist_xml(&xml))
}

/// Builds the Xcode DerivedData -> project consumer evidence: `Known`
/// when `workspace_path` matches (canonicalized) one of `project_roots`,
/// `Unknown` when it names a path that matches none of them (moved or
/// missing project -- never guessed from the DerivedData folder's own
/// name, which is a basename+hash, not a path).
pub fn xcode_derived_data_association(
    workspace_path: Option<&str>,
    project_roots: &[std::path::PathBuf],
) -> Evidence {
    let source = EvidenceSource::BuildMetadata {
        path: "info.plist#WorkspacePath".into(),
    };
    let Some(workspace_path) = workspace_path else {
        return Evidence::unknown(
            FactKind::Consumer,
            FactSubtype::DeclaredConsumer,
            source,
            now(),
            "info.plist has no WorkspacePath recorded",
        );
    };
    let ws = Path::new(workspace_path);
    let matches: Vec<String> = project_roots
        .iter()
        .filter(|root| ws.starts_with(root.as_path()) || ws == root.as_path())
        .map(|r| r.display().to_string())
        .collect();
    match matches.len() {
        0 => Evidence::unknown(
            FactKind::Consumer,
            FactSubtype::DeclaredConsumer,
            source,
            now(),
            format!(
                "WorkspacePath {workspace_path} matches no currently known project (moved or missing)"
            ),
        ),
        1 => Evidence::known(
            FactKind::Consumer,
            FactSubtype::DeclaredConsumer,
            FactValue::Text(matches[0].clone()),
            source,
            now(),
        ),
        _ => Evidence::conflicting(
            FactKind::Consumer,
            FactSubtype::DeclaredConsumer,
            matches.into_iter().map(FactValue::Text).collect(),
            source,
            now(),
            "WorkspacePath matches more than one known project root (nested checkouts)",
        ),
    }
}

// ---------------------------------------------------------------------
// Dependency lockfile identity extraction
// ---------------------------------------------------------------------

/// One dependency identity a project's lockfile declares: never proof
/// that the corresponding cache entry was actually built/imported this
/// run, only that the project's own lockfile names it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DependencyIdentity {
    pub ecosystem: &'static str,
    pub name: String,
    pub version: String,
}

/// `Cargo.lock`: TOML `[[package]]` tables with `name`/`version`.
pub fn parse_cargo_lock(text: &str) -> Result<Vec<DependencyIdentity>, String> {
    #[derive(serde::Deserialize)]
    struct Doc {
        #[serde(default, rename = "package")]
        packages: Vec<Pkg>,
    }
    #[derive(serde::Deserialize)]
    struct Pkg {
        name: String,
        version: String,
    }
    let doc: Doc = toml::from_str(text).map_err(|e| format!("invalid Cargo.lock: {e}"))?;
    Ok(doc
        .packages
        .into_iter()
        .map(|p| DependencyIdentity {
            ecosystem: "cargo",
            name: p.name,
            version: p.version,
        })
        .collect())
}

/// `package-lock.json` (npm lockfile v2/v3 `packages` map, keyed by
/// install path with a `version` field; the root package's own empty-
/// string key is skipped). Falls back to the older `dependencies` map
/// when `packages` is absent.
pub fn parse_package_lock_json(text: &str) -> Result<Vec<DependencyIdentity>, String> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("invalid package-lock.json: {e}"))?;
    let mut out = Vec::new();
    if let Some(packages) = value.get("packages").and_then(|v| v.as_object()) {
        for (path, entry) in packages {
            if path.is_empty() {
                continue; // the root project itself, not a dependency
            }
            let name = path
                .rsplit("node_modules/")
                .next()
                .unwrap_or(path)
                .to_string();
            if let Some(version) = entry.get("version").and_then(|v| v.as_str()) {
                out.push(DependencyIdentity {
                    ecosystem: "npm",
                    name,
                    version: version.to_string(),
                });
            }
        }
        return Ok(out);
    }
    if let Some(deps) = value.get("dependencies").and_then(|v| v.as_object()) {
        for (name, entry) in deps {
            if let Some(version) = entry.get("version").and_then(|v| v.as_str()) {
                out.push(DependencyIdentity {
                    ecosystem: "npm",
                    name: name.clone(),
                    version: version.to_string(),
                });
            }
        }
    }
    Ok(out)
}

/// `pnpm-lock.yaml`'s `packages:` section (a narrow line-based scan of
/// its documented `/name@version:` / `/@scope/name@version:` key shape
/// -- not a general YAML parser, mirroring how `.condarc`'s plain
/// YAML-list lines are already read in `locations::conda`). A key that
/// does not match this shape is skipped, not treated as a parse error,
/// since pnpm-lock.yaml carries many other sections this module does
/// not need.
pub fn parse_pnpm_lock_yaml(text: &str) -> Vec<DependencyIdentity> {
    let mut out = Vec::new();
    let mut in_packages = false;
    for line in text.lines() {
        if line.starts_with("packages:") {
            in_packages = true;
            continue;
        }
        if in_packages {
            if !line.starts_with(' ') && !line.is_empty() {
                break; // left the packages: section
            }
            let trimmed = line.trim();
            let Some(key) = trimmed
                .strip_suffix(':')
                .or_else(|| trimmed.strip_suffix(":"))
            else {
                continue;
            };
            let key = key.trim_start_matches('/');
            if let Some(at) = key.rfind('@') {
                let (name, version) = key.split_at(at);
                let version = &version[1..];
                if !name.is_empty() && !version.is_empty() {
                    out.push(DependencyIdentity {
                        ecosystem: "pnpm",
                        name: name.to_string(),
                        version: version.to_string(),
                    });
                }
            }
        }
    }
    out
}

/// `go.sum`: `<module> <version[/go.mod]> <hash>` lines; deduplicates
/// the plain-module and `/go.mod` line pair for the same module@version.
pub fn parse_go_sum(text: &str) -> Vec<DependencyIdentity> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let Some(module) = parts.next() else { continue };
        let Some(version) = parts.next() else {
            continue;
        };
        let version = version.trim_end_matches("/go.mod");
        if seen.insert((module.to_string(), version.to_string())) {
            out.push(DependencyIdentity {
                ecosystem: "go",
                name: module.to_string(),
                version: version.to_string(),
            });
        }
    }
    out
}

/// `gradle.lockfile`: `group:artifact:version=configurations` lines
/// (the `empty=...` marker line and comments are skipped).
pub fn parse_gradle_lockfile(text: &str) -> Vec<DependencyIdentity> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("empty=") {
            continue;
        }
        let Some((coord, _configs)) = line.split_once('=') else {
            continue;
        };
        let parts: Vec<&str> = coord.split(':').collect();
        if parts.len() == 3 {
            out.push(DependencyIdentity {
                ecosystem: "gradle",
                name: format!("{}:{}", parts[0], parts[1]),
                version: parts[2].to_string(),
            });
        }
    }
    out
}

// ---------------------------------------------------------------------
// Joining a cache entry's identity to the projects whose lockfiles
// declare it.
// ---------------------------------------------------------------------

/// One project's already-parsed lockfile identities, kept with the
/// project's own label so a join result can name exactly which
/// projects declared a given entry.
pub struct ProjectDependencies<'a> {
    pub project_label: &'a str,
    pub identities: &'a [DependencyIdentity],
}

/// Joins one shared cache entry's identity to every project that
/// declares it. Distinguishes a similar name at a different version
/// (not a match) from a genuine shared reference (an exact
/// name+version match in more than one project). Never forces a single
/// owner and never mutates/duplicates the underlying cache entry.
pub fn join_cache_entry(
    entry: &DependencyIdentity,
    projects: &[ProjectDependencies<'_>],
) -> Evidence {
    let consumers: Vec<String> = projects
        .iter()
        .filter(|p| p.identities.contains(entry))
        .map(|p| p.project_label.to_string())
        .collect();
    let source = EvidenceSource::Lockfile {
        ecosystem: entry.ecosystem.to_string(),
        path: format!("{}@{}", entry.name, entry.version),
    };
    match consumers.len() {
        0 => Evidence::unknown(
            FactKind::Consumer,
            FactSubtype::DeclaredConsumer,
            source,
            now(),
            "no scanned project's lockfile declares this exact name+version",
        ),
        1 => Evidence::known(
            FactKind::Consumer,
            FactSubtype::DeclaredConsumer,
            FactValue::Text(consumers[0].clone()),
            source,
            now(),
        ),
        _ => Evidence::known(
            FactKind::Consumer,
            FactSubtype::DeclaredConsumer,
            FactValue::List(consumers),
            source,
            now(),
        )
        .with_note("declared by more than one project; this is a shared reference, not an error"),
    }
}

/// An unparseable lockfile is a named evidence gap, never a silent
/// "no dependencies declared" (which would be indistinguishable from a
/// project that genuinely has none).
pub fn invalid_lockfile_evidence(ecosystem: &str, path: &Path, parse_error: &str) -> Evidence {
    Evidence::unavailable(
        FactKind::Consumer,
        FactSubtype::DeclaredConsumer,
        EvidenceSource::Lockfile {
            ecosystem: ecosystem.to_string(),
            path: path.display().to_string(),
        },
        now(),
        format!("could not parse lockfile: {parse_error}"),
    )
}

/// Normalizes an already-established Docker join (compose-project
/// label, `org.opencontainers.image.source` match, or a label naming a
/// path inside a discovered worktree -- see `attribution.rs`'s existing
/// join logic) into this module's shared `Evidence` contract, so a
/// Docker consumer relationship and a lockfile-declared one render the
/// same way. This does not re-derive the join; it only carries an
/// already-decided project label (or `None` for `DockerNoJoin`) into
/// the new shape.
pub fn docker_join_evidence(project_label: Option<&str>, join_basis: &str) -> Evidence {
    let source = EvidenceSource::Inferred {
        basis: join_basis.to_string(),
    };
    match project_label {
        Some(label) => Evidence::known(
            FactKind::Consumer,
            FactSubtype::InferredConsumer,
            FactValue::Text(label.to_string()),
            source,
            now(),
        ),
        None => Evidence::unknown(
            FactKind::Consumer,
            FactSubtype::InferredConsumer,
            source,
            now(),
            "no compose-project label, image-source label, or worktree-path label matched a discovered project",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const SAMPLE_INFO_PLIST_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>WorkspacePath</key>
    <string>/Users/dev/src/MyApp/MyApp.xcworkspace</string>
</dict>
</plist>
"#;

    #[test]
    fn extracts_workspace_path_from_plist_xml() {
        assert_eq!(
            parse_workspace_path_from_plist_xml(SAMPLE_INFO_PLIST_XML),
            Some("/Users/dev/src/MyApp/MyApp.xcworkspace".to_string())
        );
    }

    #[test]
    fn missing_workspace_path_key_is_none() {
        assert_eq!(
            parse_workspace_path_from_plist_xml("<plist><dict></dict></plist>"),
            None
        );
    }

    #[test]
    fn xcode_association_matches_known_project() {
        let roots = vec![PathBuf::from("/Users/dev/src/MyApp")];
        let ev =
            xcode_derived_data_association(Some("/Users/dev/src/MyApp/MyApp.xcworkspace"), &roots);
        assert!(ev.is_known());
    }

    #[test]
    fn xcode_association_moved_project_is_unknown_never_guessed_from_basename() {
        // The tempting shortcut this rejects: matching DerivedData's own
        // `MyApp-abc123` folder name against a project named "MyApp"
        // even though the recorded WorkspacePath points elsewhere.
        let roots = vec![PathBuf::from("/Users/dev/src/OtherApp")];
        let ev =
            xcode_derived_data_association(Some("/Users/dev/src/MyApp/MyApp.xcworkspace"), &roots);
        assert!(!ev.is_known());
    }

    #[test]
    fn cargo_lock_two_projects_share_one_entry() {
        let text = r#"
[[package]]
name = "serde"
version = "1.0.203"

[[package]]
name = "libc"
version = "0.2.155"
"#;
        let entries = parse_cargo_lock(text).unwrap();
        let serde = entries.iter().find(|e| e.name == "serde").unwrap();
        let project_a = ProjectDependencies {
            project_label: "project-a",
            identities: &entries,
        };
        let project_b = ProjectDependencies {
            project_label: "project-b",
            identities: &entries,
        };
        let ev = join_cache_entry(serde, &[project_a, project_b]);
        match ev.status {
            crate::evidence::FactStatus::Known(FactValue::List(labels)) => {
                assert_eq!(labels.len(), 2);
            }
            other => panic!("expected known list of two consumers, got {other:?}"),
        }
    }

    #[test]
    fn similar_name_different_version_is_not_a_match() {
        let a = vec![DependencyIdentity {
            ecosystem: "cargo",
            name: "serde".into(),
            version: "1.0.203".into(),
        }];
        let b = vec![DependencyIdentity {
            ecosystem: "cargo",
            name: "serde".into(),
            version: "1.0.150".into(),
        }];
        let entry = DependencyIdentity {
            ecosystem: "cargo",
            name: "serde".into(),
            version: "1.0.203".into(),
        };
        let projects = vec![
            ProjectDependencies {
                project_label: "uses-newer",
                identities: &a,
            },
            ProjectDependencies {
                project_label: "uses-older",
                identities: &b,
            },
        ];
        let ev = join_cache_entry(&entry, &projects);
        match ev.status {
            crate::evidence::FactStatus::Known(FactValue::Text(label)) => {
                assert_eq!(label, "uses-newer");
            }
            other => panic!("expected exactly one match, got {other:?}"),
        }
    }

    #[test]
    fn moved_or_missing_project_leaves_entry_orphaned_not_owned() {
        let entry = DependencyIdentity {
            ecosystem: "cargo",
            name: "orphan-crate".into(),
            version: "1.0.0".into(),
        };
        let ev = join_cache_entry(&entry, &[]);
        assert!(!ev.is_known());
    }

    #[test]
    fn invalid_cargo_lock_is_a_named_gap_not_a_silent_empty_list() {
        let err = parse_cargo_lock("not valid toml [[[").unwrap_err();
        let ev = invalid_lockfile_evidence("cargo", Path::new("/proj/Cargo.lock"), &err);
        assert!(matches!(
            ev.status,
            crate::evidence::FactStatus::Unavailable { .. }
        ));
    }

    #[test]
    fn package_lock_json_parses_packages_map() {
        let text = r#"{
            "packages": {
                "": {"name": "root"},
                "node_modules/lodash": {"version": "4.17.21"}
            }
        }"#;
        let entries = parse_package_lock_json(text).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "lodash");
        assert_eq!(entries[0].version, "4.17.21");
    }

    #[test]
    fn pnpm_lock_yaml_parses_scoped_and_unscoped_packages() {
        let text = "lockfileVersion: '6.0'\npackages:\n  /lodash@4.17.21:\n    resolution: {}\n  /@babel/core@7.20.0:\n    resolution: {}\nsettings:\n  autoInstallPeers: true\n";
        let entries = parse_pnpm_lock_yaml(text);
        assert!(
            entries
                .iter()
                .any(|e| e.name == "lodash" && e.version == "4.17.21")
        );
        assert!(
            entries
                .iter()
                .any(|e| e.name == "@babel/core" && e.version == "7.20.0")
        );
    }

    #[test]
    fn go_sum_parses_module_version_pairs_deduplicating_go_mod_lines() {
        let text = "golang.org/x/text v0.14.0 h1:abc=\ngolang.org/x/text v0.14.0/go.mod h1:def=\n";
        let entries = parse_go_sum(text);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "golang.org/x/text");
        assert_eq!(entries[0].version, "v0.14.0");
    }

    #[test]
    fn gradle_lockfile_parses_coordinate_lines() {
        let text =
            "org.jetbrains.kotlin:kotlin-stdlib:1.9.0=compileClasspath\nempty=testCompileOnly\n";
        let entries = parse_gradle_lockfile(text);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "org.jetbrains.kotlin:kotlin-stdlib");
        assert_eq!(entries[0].version, "1.9.0");
    }

    #[test]
    fn docker_join_normalizes_into_shared_contract() {
        let joined = docker_join_evidence(Some("hiphi-relay"), "compose-project label");
        assert!(joined.is_known());
        let unjoined = docker_join_evidence(None, "no matching label");
        assert!(!unjoined.is_known());
    }
}
