//! Oh My Pi identification (#94): sessions, a content-addressed shared
//! blob store, terminal breadcrumbs and protected configuration under
//! the home `crate::locations::oh_my_pi::OhMyPiDetector` resolves.
//!
//! Layout researched from primary source during implementation (never
//! from a real `~/.omp` on this machine -- PRIVACY IS A HARD RULE), both
//! from <https://github.com/can1357/oh-my-pi>, current `main` as of this
//! chunk:
//! - `docs/session.md`: sessions at
//!   `~/.omp/agent/sessions/<encoded-cwd>/<timestamp>_<sessionId>.jsonl`;
//!   files "physically begin with a fixed-width, 256-byte `type:
//!   "title"` slot, followed by the session header" (header fields
//!   include `cwd` and `additionalDirectories`, "normalized, deduplicated
//!   workspace roots beyond cwd"); a content-addressed blob store at
//!   `~/.omp/agent/blobs/<sha256>` ("Image data URLs in `image_url`
//!   fields are always content-addressed in the blob store and replaced
//!   with `blob:sha256:<hash>`"); terminal breadcrumbs at
//!   `~/.omp/agent/terminal-sessions/`.
//! - `docs/settings.md`: main config `~/.omp/agent/config.yml`
//!   (`config.yaml` also accepted); custom models `models.yml`;
//!   authentication in `agent.db`, under the agent directory.
//!
//! ## Unknown-format disambiguation (#94's explicit acceptance)
//!
//! `crate::locations::oh_my_pi` already narrows the default path to the
//! `agent/` subdirectory specifically (not the bare `~/.omp` wrapper),
//! which most unrelated `~/.omp` users (the issue names oh-my-posh as one
//! to check) would have no reason to create. This adapter adds a second,
//! independent check: before identifying anything, it looks for at least
//! one of this format's own content markers (`config.yml`, `config.yaml`,
//! `agent.db`, `sessions/`, `blobs/`). If none are present but the
//! directory exists and is non-empty, this adapter reports one
//! `Unclassified`, non-actionable "unknown format" unit for the whole
//! directory and identifies nothing further -- never a guess.
//!
//! ## Shared blobs (#94's named concern)
//!
//! A blob is content-addressed and may be referenced by more than one
//! session's `image_url` fields. Establishing *complete* reference
//! coverage would mean reading every session body in full, which this
//! adapter deliberately does not do (see `MAX_BODY_SCAN_BYTES`): each
//! session's body is scanned only up to a bound, extracting `blob:
//! sha256:<hash>` reference tokens -- never anything else in the body,
//! and never persisted or logged as text, only folded into a
//! `HashSet<String>` of hashes in memory. A session whose body exceeds
//! the bound is marked with incomplete coverage, and if *any* session in
//! this pass had incomplete coverage, every blob's reference count is
//! reported as unknown rather than a possibly-wrong number. No blob is
//! ever offered a selective action in this chunk (`AgentActionCapability
//! ::None`, unconditionally) -- reference-based GC is out of scope here,
//! not merely gated, so "block GC until complete" is trivially true
//! rather than a runtime check this adapter could get wrong.

use super::{
    AgentActionCapability, AgentCategory, AgentMember, AgentMemberKind, CandidateAgentUnit,
    ProjectLinkState, folded_bytes, mtime_secs, resolve_declared_path,
};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

pub const OH_MY_PI_TOOL_ID: &str = crate::locations::oh_my_pi::OH_MY_PI_DETECTOR_ID;

const MAX_FOLD_ENTRIES: usize = 200_000;
const MAX_WALK_DEPTH: usize = 4;
/// The fixed-width title slot every session file begins with, per
/// `docs/session.md`: "Current files physically begin with a
/// fixed-width, 256-byte `type: "title"` slot, followed by the session
/// header." The header line starts immediately after this many bytes.
const TITLE_SLOT_BYTES: usize = 256;
/// Bound on how many bytes of a session's header line (after the title
/// slot) this adapter reads looking for `cwd`/`additionalDirectories`.
const HEADER_READ_BYTES: usize = 8192;
/// Bound on how many bytes of a session's *body* (from the very start of
/// the file, title slot included) this adapter scans for `blob:sha256:`
/// reference tokens. Deliberately larger than `HEADER_READ_BYTES` --
/// blob references can appear anywhere in the conversation, not just the
/// header -- but still bounded: see the module doc comment on why a
/// session exceeding this is marked incomplete rather than fully read.
const MAX_BODY_SCAN_BYTES: usize = 65_536;

pub fn identify(home: &Path, _observed_at: u64) -> Vec<CandidateAgentUnit> {
    if !home.is_dir() {
        return Vec::new();
    }
    if !has_format_markers(home) {
        return unknown_format_residual(home);
    }
    let mut units = Vec::new();
    let (referenced, any_truncated) = scan_blob_references(home);
    identify_sessions(home, &mut units);
    identify_blobs(home, &referenced, any_truncated, &mut units);
    identify_static_categories(home, &mut units);
    units
}

fn has_format_markers(home: &Path) -> bool {
    ["config.yml", "config.yaml", "agent.db", "sessions", "blobs"]
        .iter()
        .any(|rel| home.join(rel).exists())
}

fn unknown_format_residual(home: &Path) -> Vec<CandidateAgentUnit> {
    // A genuinely empty, existing directory (nothing here yet) is not
    // "unknown format" -- there is simply nothing to classify. Checked by
    // directory entries, not by `folded_bytes`'s byte/mtime pair: an
    // empty directory's own mtime is non-zero, which would otherwise
    // read as "something present" and produce a bogus residual unit.
    let has_entries = fs::read_dir(home)
        .map(|mut rd| rd.next().is_some())
        .unwrap_or(false);
    if !has_entries {
        return Vec::new();
    }
    let (bytes, mtime, _truncated) = folded_bytes(home, MAX_FOLD_ENTRIES);
    vec![CandidateAgentUnit {
        category: AgentCategory::Unclassified,
        relative_path: "(unknown format)".to_string(),
        path: home.to_path_buf(),
        members: Vec::new(),
        bytes,
        mtime_max: mtime,
        protected: false,
        protect_reason: None,
        project_link: ProjectLinkState::NotApplicable,
        action: AgentActionCapability::None,
        note: Some(
            "no Oh My Pi content markers found (config.yml/config.yaml/agent.db/sessions/blobs) \
             at this resolved path; this directory may belong to a different tool, be empty, or \
             use an unsupported Oh My Pi version -- treated as unknown format, not scanned \
             further"
                .to_string(),
        ),
    }]
}

// ---------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------

fn identify_sessions(home: &Path, out: &mut Vec<CandidateAgentUnit>) {
    let base = home.join("sessions");
    let mut files = Vec::new();
    collect_files(&base, 0, &mut files);
    for jsonl in files {
        let Ok(meta) = fs::symlink_metadata(&jsonl) else {
            continue;
        };
        let bytes = meta.len();
        let mtime = mtime_secs(&meta);
        let header = read_header(&jsonl);
        let project_link = resolve_session_link(header);
        let relative_path = relative_to(home, &jsonl);
        out.push(CandidateAgentUnit {
            category: AgentCategory::Sessions,
            relative_path,
            path: jsonl.clone(),
            members: vec![AgentMember {
                path: jsonl,
                bytes,
                kind: AgentMemberKind::Transcript,
            }],
            bytes,
            mtime_max: mtime,
            protected: false,
            protect_reason: None,
            project_link,
            action: AgentActionCapability::SessionRemoval,
            note: None,
        });
    }
}

fn collect_files(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > MAX_WALK_DEPTH || out.len() > MAX_FOLD_ENTRIES {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        if out.len() > MAX_FOLD_ENTRIES {
            return;
        }
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() && !ft.is_symlink() {
            collect_files(&path, depth + 1, out);
        } else if ft.is_file() && path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            out.push(path);
        }
    }
}

struct SessionHeader {
    cwd: Option<String>,
    additional_directories: Vec<String>,
}

/// Skips the fixed 256-byte title slot, then parses the header line.
fn read_header(path: &Path) -> SessionHeader {
    let empty = SessionHeader {
        cwd: None,
        additional_directories: Vec::new(),
    };
    let Ok(mut f) = fs::File::open(path) else {
        return empty;
    };
    let mut buf = vec![0u8; TITLE_SLOT_BYTES + HEADER_READ_BYTES];
    let Ok(n) = f.read(&mut buf) else {
        return empty;
    };
    if n <= TITLE_SLOT_BYTES {
        return empty;
    }
    buf.truncate(n);
    let after_title = &buf[TITLE_SLOT_BYTES..];
    let text = String::from_utf8_lossy(after_title);
    let Some(header_line) = text.lines().next() else {
        return empty;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(header_line) else {
        return empty;
    };
    let cwd = value
        .get("cwd")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let additional_directories = value
        .get("additionalDirectories")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    SessionHeader {
        cwd,
        additional_directories,
    }
}

/// Resolves a session's project linkage from its `cwd`, widening to
/// `ProjectLinkState::Shared` when `additionalDirectories` names a
/// workspace root that resolves to a *different* project identity --
/// never fabricating single ownership when the session's own metadata
/// names more than one project.
fn resolve_session_link(header: SessionHeader) -> ProjectLinkState {
    let primary = resolve_declared_path(header.cwd, "no cwd field found in the session header");
    let mut project_ids: Vec<String> = Vec::new();
    if let ProjectLinkState::Linked { project_id, .. } = &primary {
        project_ids.push(project_id.clone());
    }
    for extra in header.additional_directories {
        if let ProjectLinkState::Linked { project_id, .. } = resolve_declared_path(Some(extra), "")
            && !project_ids.contains(&project_id)
        {
            project_ids.push(project_id);
        }
    }
    if project_ids.len() > 1 {
        ProjectLinkState::Shared { project_ids }
    } else {
        primary
    }
}

// ---------------------------------------------------------------------
// Shared blobs
// ---------------------------------------------------------------------

/// Scans every session body (bounded) for `blob:sha256:<64-hex>` tokens.
/// Returns the referenced-hash counts and whether any session's body
/// exceeded the per-file scan bound (in which case every blob's
/// reference count must be reported as unknown, never a possibly-wrong
/// number -- see the module doc comment).
fn scan_blob_references(home: &Path) -> (HashMap<String, usize>, bool) {
    let base = home.join("sessions");
    let mut files = Vec::new();
    collect_files(&base, 0, &mut files);
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut any_truncated = false;
    for jsonl in files {
        let Ok(mut f) = fs::File::open(&jsonl) else {
            any_truncated = true;
            continue;
        };
        let Ok(meta) = f.metadata() else {
            any_truncated = true;
            continue;
        };
        if meta.len() as usize > MAX_BODY_SCAN_BYTES {
            any_truncated = true;
        }
        let mut buf = vec![0u8; MAX_BODY_SCAN_BYTES];
        let Ok(n) = f.read(&mut buf) else { continue };
        buf.truncate(n);
        let text = String::from_utf8_lossy(&buf);
        for hash in extract_blob_hashes(&text) {
            *counts.entry(hash).or_insert(0) += 1;
        }
    }
    (counts, any_truncated)
}

const BLOB_PREFIX: &str = "blob:sha256:";

fn extract_blob_hashes(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(idx) = rest.find(BLOB_PREFIX) {
        let after = &rest[idx + BLOB_PREFIX.len()..];
        let hash: String = after
            .chars()
            .take(64)
            .take_while(|c| c.is_ascii_hexdigit())
            .collect();
        let advance = hash.len().min(after.len());
        if hash.len() == 64 {
            out.push(hash);
        }
        rest = &after[advance..];
    }
    out
}

fn identify_blobs(
    home: &Path,
    referenced: &HashMap<String, usize>,
    any_truncated: bool,
    out: &mut Vec<CandidateAgentUnit>,
) {
    let base = home.join("blobs");
    let Ok(rd) = fs::read_dir(&base) else { return };
    for entry in rd.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_file() {
            continue;
        }
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        let hash = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        let note = if any_truncated {
            "shared content-addressed blob; reference coverage unknown this pass (at least one \
             session's body exceeded the scan bound) -- not offered for removal"
                .to_string()
        } else {
            match referenced.get(hash) {
                Some(n) if *n > 0 => format!(
                    "shared content-addressed blob; referenced by {n} known session(s) in this \
                     pass's full coverage -- not offered for removal without a supported GC \
                     action"
                ),
                _ => "shared content-addressed blob; no referencing session found in this \
                      pass's full coverage -- not offered for removal without a supported GC \
                      action"
                    .to_string(),
            }
        };
        out.push(CandidateAgentUnit {
            category: AgentCategory::Attachments,
            relative_path: relative_to(home, &path),
            path: path.clone(),
            members: Vec::new(),
            bytes: meta.len(),
            mtime_max: mtime_secs(&meta),
            protected: false,
            protect_reason: None,
            project_link: ProjectLinkState::NotApplicable,
            action: AgentActionCapability::None,
            note: Some(note),
        });
    }
}

// ---------------------------------------------------------------------
// Static top-level categories
// ---------------------------------------------------------------------

fn identify_static_categories(home: &Path, out: &mut Vec<CandidateAgentUnit>) {
    for (rel, note) in [
        ("config.yml", "main configuration"),
        (
            "config.yaml",
            "main configuration (alternate accepted filename)",
        ),
        ("models.yml", "custom model definitions"),
    ] {
        let path = home.join(rel);
        if !path.exists() {
            continue;
        }
        let (bytes, mtime, _truncated) = folded_bytes(&path, MAX_FOLD_ENTRIES);
        out.push(CandidateAgentUnit {
            category: AgentCategory::ProtectedConfig,
            relative_path: rel.to_string(),
            path,
            members: Vec::new(),
            bytes,
            mtime_max: mtime,
            protected: true,
            protect_reason: Some(note.to_string()),
            project_link: ProjectLinkState::NotApplicable,
            action: AgentActionCapability::None,
            note: Some(note.to_string()),
        });
    }

    let db = home.join("agent.db");
    if let Ok(meta) = fs::symlink_metadata(&db)
        && meta.is_file()
    {
        let mut members = vec![AgentMember {
            path: db.clone(),
            bytes: meta.len(),
            kind: AgentMemberKind::Database,
        }];
        let mut bytes = meta.len();
        let mut mtime_max = mtime_secs(&meta);
        for ext in ["-wal", "-shm"] {
            let sidecar = PathBuf::from(format!("{}{ext}", db.display()));
            if let Ok(sm) = fs::symlink_metadata(&sidecar)
                && sm.is_file()
            {
                bytes += sm.len();
                mtime_max = mtime_max.max(mtime_secs(&sm));
                members.push(AgentMember {
                    path: sidecar,
                    bytes: sm.len(),
                    kind: AgentMemberKind::Database,
                });
            }
        }
        out.push(CandidateAgentUnit {
            category: AgentCategory::ProtectedConfig,
            relative_path: "agent.db".to_string(),
            path: db,
            members,
            bytes,
            mtime_max,
            protected: true,
            protect_reason: Some(
                "authentication database (SQLite); contents are never read by this adapter"
                    .to_string(),
            ),
            project_link: ProjectLinkState::NotApplicable,
            action: AgentActionCapability::None,
            note: None,
        });
    }

    let terminal = home.join("terminal-sessions");
    if terminal.is_dir() {
        let (bytes, mtime, truncated) = folded_bytes(&terminal, MAX_FOLD_ENTRIES);
        out.push(CandidateAgentUnit {
            category: AgentCategory::Logs,
            relative_path: "terminal-sessions".to_string(),
            path: terminal,
            members: Vec::new(),
            bytes,
            mtime_max: mtime,
            protected: false,
            protect_reason: None,
            project_link: ProjectLinkState::NotApplicable,
            action: AgentActionCapability::CacheOrLogTrash,
            note: Some(if truncated {
                "terminal breadcrumb files; directory entry count bound reached".to_string()
            } else {
                "terminal breadcrumb files; regenerated automatically".to_string()
            }),
        });
    }

    let seen: HashSet<&str> = [
        "config.yml",
        "config.yaml",
        "models.yml",
        "agent.db",
        "agent.db-wal",
        "agent.db-shm",
        "terminal-sessions",
        "sessions",
        "blobs",
    ]
    .into_iter()
    .collect();
    let Ok(rd) = fs::read_dir(home) else { return };
    let mut residual_bytes = 0u64;
    let mut residual_mtime = 0u64;
    let mut residual_names: Vec<String> = Vec::new();
    for e in rd.flatten() {
        let Some(name) = e
            .path()
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::to_string)
        else {
            continue;
        };
        if seen.contains(name.as_str()) {
            continue;
        }
        let (bytes, mtime, _truncated) = folded_bytes(&e.path(), MAX_FOLD_ENTRIES);
        residual_bytes += bytes;
        residual_mtime = residual_mtime.max(mtime);
        residual_names.push(name);
    }
    if !residual_names.is_empty() {
        residual_names.sort();
        out.push(CandidateAgentUnit {
            category: AgentCategory::Unclassified,
            relative_path: "(unclassified residual)".to_string(),
            path: home.to_path_buf(),
            members: Vec::new(),
            bytes: residual_bytes,
            mtime_max: residual_mtime,
            protected: false,
            protect_reason: None,
            project_link: ProjectLinkState::NotApplicable,
            action: AgentActionCapability::None,
            note: Some(format!(
                "entries with no specific rule in this adapter: {}",
                residual_names.join(", ")
            )),
        });
    }
}

fn relative_to(home: &Path, path: &Path) -> String {
    path.strip_prefix(home)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn touch(path: &Path, content: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn session_bytes(cwd: &str, additional: &[&str], canary: &str) -> Vec<u8> {
        let mut title = vec![b' '; 256];
        let title_json = b"{\"type\":\"title\"}";
        title[..title_json.len()].copy_from_slice(title_json);
        title[255] = b'\n';
        let extra = additional
            .iter()
            .map(|d| format!("\"{d}\""))
            .collect::<Vec<_>>()
            .join(",");
        let header = format!(
            "{{\"type\":\"session\",\"cwd\":\"{cwd}\",\"additionalDirectories\":[{extra}]}}\n"
        );
        let entry = format!(
            "{{\"id\":\"1\",\"parentId\":null,\"timestamp\":0,\"content\":\"{canary}\"}}\n"
        );
        let mut out = title;
        out.extend_from_slice(header.as_bytes());
        out.extend_from_slice(entry.as_bytes());
        out
    }

    #[test]
    fn empty_home_yields_no_units() {
        let dir = tempfile::tempdir().unwrap();
        assert!(identify(dir.path(), 1).is_empty());
    }

    #[test]
    fn no_format_markers_yields_unknown_format_not_a_guess() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("unrelated-file.txt"), b"hello");
        let units = identify(dir.path(), 1);
        assert_eq!(units.len(), 1);
        assert_eq!(units[0].relative_path, "(unknown format)");
        assert_eq!(units[0].action, AgentActionCapability::None);
    }

    #[test]
    fn a_session_is_identified_and_linked_via_cwd() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        let repo = home.join("fixture-repo");
        fs::create_dir_all(repo.join(".git")).unwrap();
        touch(&home.join("config.yml"), b"providers: {}");
        let canary = "CANARY-OMP-DO-NOT-LEAK-77bb";
        let jsonl = home
            .join("sessions/-fixture-repo/1700000000_11111111-1111-4111-8111-111111111111.jsonl");
        touch(
            &jsonl,
            &session_bytes(&repo.display().to_string(), &[], canary),
        );
        let units = identify(home, 1);
        let session = units
            .iter()
            .find(|u| u.category == AgentCategory::Sessions && u.path == jsonl)
            .expect("session identified");
        assert!(matches!(
            session.project_link,
            ProjectLinkState::Linked { .. }
        ));
        assert_eq!(session.action, AgentActionCapability::SessionRemoval);
        let serialized = format!("{units:?}");
        assert!(!serialized.contains(canary), "prompt content leaked");
    }

    #[test]
    fn additional_directories_naming_a_different_project_yields_shared() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        let repo_a = home.join("repo-a");
        let repo_b = home.join("repo-b");
        fs::create_dir_all(repo_a.join(".git")).unwrap();
        fs::create_dir_all(repo_b.join(".git")).unwrap();
        touch(&home.join("config.yml"), b"providers: {}");
        let jsonl = home.join("sessions/x/1_22222222-2222-4222-8222-222222222222.jsonl");
        touch(
            &jsonl,
            &session_bytes(
                &repo_a.display().to_string(),
                &[&repo_b.display().to_string()],
                "x",
            ),
        );
        let units = identify(home, 1);
        let session = units.iter().find(|u| u.path == jsonl).unwrap();
        assert!(matches!(
            session.project_link,
            ProjectLinkState::Shared { .. }
        ));
    }

    #[test]
    fn blobs_report_reference_counts_from_full_coverage() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        touch(&home.join("config.yml"), b"providers: {}");
        let hash = "a".repeat(64);
        touch(&home.join("blobs").join(&hash), b"binary-image-bytes");
        let jsonl = home.join("sessions/x/1_33333333-3333-4333-8333-333333333333.jsonl");
        let mut body = session_bytes("/no/such/dir", &[], "unread");
        body.extend_from_slice(format!("{{\"image_url\":\"blob:sha256:{hash}\"}}\n").as_bytes());
        touch(&jsonl, &body);
        let units = identify(home, 1);
        let blob = units
            .iter()
            .find(|u| u.category == AgentCategory::Attachments)
            .expect("blob unit present");
        assert_eq!(blob.action, AgentActionCapability::None);
        assert!(blob.note.as_deref().unwrap().contains("referenced by 1"));
    }

    #[test]
    fn unreferenced_blob_says_so_without_offering_gc() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        touch(&home.join("config.yml"), b"providers: {}");
        let hash = "b".repeat(64);
        touch(&home.join("blobs").join(&hash), b"binary-image-bytes");
        let units = identify(home, 1);
        let blob = units
            .iter()
            .find(|u| u.category == AgentCategory::Attachments)
            .unwrap();
        assert_eq!(blob.action, AgentActionCapability::None);
        assert!(
            blob.note
                .as_deref()
                .unwrap()
                .contains("no referencing session found")
        );
    }

    #[test]
    fn agent_db_is_protected_and_folds_sidecars() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        touch(&home.join("agent.db"), b"sqlite");
        touch(&home.join("agent.db-wal"), b"wal");
        let units = identify(home, 1);
        let db = units
            .iter()
            .find(|u| u.relative_path == "agent.db")
            .unwrap();
        assert!(db.protected);
        assert_eq!(db.action, AgentActionCapability::None);
        assert_eq!(db.members.len(), 2);
    }

    #[test]
    fn terminal_sessions_are_actionable_cache_or_log_trash() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        touch(&home.join("config.yml"), b"providers: {}");
        touch(&home.join("terminal-sessions").join("t1"), b"breadcrumb");
        let units = identify(home, 1);
        let u = units
            .iter()
            .find(|u| u.relative_path == "terminal-sessions")
            .unwrap();
        assert!(!u.protected);
        assert_eq!(u.action, AgentActionCapability::CacheOrLogTrash);
    }

    #[test]
    fn identification_cost_is_bounded_for_many_sessions() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        touch(&home.join("config.yml"), b"providers: {}");
        let repo = home.join("big-repo");
        fs::create_dir_all(repo.join(".git")).unwrap();
        for i in 0..500 {
            let jsonl = home.join(format!(
                "sessions/-big-repo/{i}_77777777-7777-4777-8{i:03}-777777777777.jsonl"
            ));
            let mut body = session_bytes(&repo.display().to_string(), &[], "unread-canary");
            body.extend_from_slice(&b"x".repeat(200_000));
            touch(&jsonl, &body);
        }
        let start = SystemTime::now();
        let units = identify(home, 1);
        let elapsed = SystemTime::now().duration_since(start).unwrap_or_default();
        eprintln!("[measured] oh_my_pi identify() over 500 synthetic sessions took {elapsed:?}");
        assert_eq!(
            units
                .iter()
                .filter(|u| u.category == AgentCategory::Sessions)
                .count(),
            500
        );
        assert!(elapsed < Duration::from_secs(10), "{elapsed:?}");
    }
}
