//! Pi identification (#96): sessions, protected configuration, and a
//! locally-installed npm package cache under the home
//! `crate::locations::pi::PiDetector` resolves. **Distinct tool from Oh
//! My Pi** (`crate::agents::oh_my_pi`) -- see that detector's sibling
//! doc comment for the disclosed override-variable collision risk.
//!
//! Layout sourced from primary docs during implementation (never a real
//! `~/.pi` on this machine -- PRIVACY IS A HARD RULE); see
//! `crate::locations::pi`'s doc comment for citations.
//!
//! ## Explicit format/version detection (#96's own instruction: "reuse
//! the OMP adapter's parsing where formats match, but detect version/
//! format explicitly")
//!
//! Pi's own README documents its session files only as "JSONL files with
//! a tree structure. Each entry has an `id` and `parentId`" -- it does
//! **not** document Oh My Pi's 256-byte fixed-width title slot ahead of
//! the header line (`crate::agents::oh_my_pi`'s own citation). This
//! adapter therefore tries, per session file, in order:
//! 1. Parse line 1 (byte offset 0) directly as JSON, looking for a `cwd`
//!    field -- Pi's own documented shape.
//! 2. Only if that fails, retry Oh My Pi's title-slot-skip shape (in
//!    case a particular Pi release shares it, since Oh My Pi is a fork
//!    of this very package) -- reused, not assumed.
//! 3. If neither yields a `cwd`, this session's linkage is `Unresolved`,
//!    naming both shapes checked, never a guess.
//!
//! `sessions/` is documented as "organized by working directory" -- this
//! adapter deliberately does not decode a directory name into a project
//! path (no encoding scheme is confirmed by primary source), relying
//! only on each session file's own declared `cwd`.

use super::{
    AgentActionCapability, AgentCategory, AgentMember, AgentMemberKind, CandidateAgentUnit,
    ProjectLinkState, folded_bytes, mtime_secs, resolve_declared_path,
};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

pub const PI_TOOL_ID: &str = crate::locations::pi::PI_DETECTOR_ID;

const MAX_FOLD_ENTRIES: usize = 200_000;
const MAX_WALK_DEPTH: usize = 4;
const HEADER_READ_BYTES: usize = 8192;
/// Oh My Pi's own fixed-width title slot, tried only as a fallback --
/// see the module doc comment.
const TITLE_SLOT_BYTES: usize = 256;

const FORMAT_MARKERS: &[&str] = &[
    "settings.json",
    "trust.json",
    "models.json",
    "sessions",
    "npm",
];

pub fn identify(home: &Path, _observed_at: u64) -> Vec<CandidateAgentUnit> {
    if !home.is_dir() {
        return Vec::new();
    }
    if !FORMAT_MARKERS.iter().any(|rel| home.join(rel).exists()) {
        return unknown_format_residual(home);
    }
    let mut units = Vec::new();
    identify_sessions(home, &mut units);
    identify_static_categories(home, &mut units);
    units
}

fn unknown_format_residual(home: &Path) -> Vec<CandidateAgentUnit> {
    let has_entries = fs::read_dir(home)
        .map(|mut rd| rd.next().is_some())
        .unwrap_or(false);
    if !has_entries {
        return Vec::new();
    }
    let (bytes, mtime, _t) = folded_bytes(home, MAX_FOLD_ENTRIES);
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
            "no Pi content markers found (settings.json/trust.json/models.json/sessions/npm) at \
             this resolved path; this directory may belong to a different tool, be empty, or use \
             an unsupported version -- treated as unknown format, not scanned further"
                .to_string(),
        ),
    }]
}

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
        let project_link = resolve_session_link(&jsonl);
        out.push(CandidateAgentUnit {
            category: AgentCategory::Sessions,
            relative_path: relative_to(home, &jsonl),
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

fn cwd_from_json_line(line: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    value
        .get("cwd")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Tries Pi's own documented shape (line 1 at offset 0) first, then Oh
/// My Pi's title-slot-skip shape as a fallback -- see the module doc
/// comment. Returns which shape (if either) matched, for the honest
/// `Unresolved` reason when neither does.
fn resolve_session_link(path: &Path) -> ProjectLinkState {
    let Ok(mut f) = fs::File::open(path) else {
        return resolve_declared_path(None, "could not open session file to read its header");
    };
    let mut buf = vec![0u8; TITLE_SLOT_BYTES + HEADER_READ_BYTES];
    let Ok(n) = f.read(&mut buf) else {
        return resolve_declared_path(None, "could not read session file header");
    };
    buf.truncate(n);
    let text = String::from_utf8_lossy(&buf);

    if let Some(first_line) = text.lines().next()
        && let Some(cwd) = cwd_from_json_line(first_line)
    {
        return resolve_declared_path(Some(cwd), "");
    }
    if n > TITLE_SLOT_BYTES {
        let after_title = String::from_utf8_lossy(&buf[TITLE_SLOT_BYTES..]);
        if let Some(header_line) = after_title.lines().next()
            && let Some(cwd) = cwd_from_json_line(header_line)
        {
            return resolve_declared_path(Some(cwd), "");
        }
    }
    resolve_declared_path(
        None,
        "no cwd field found at byte offset 0 (Pi's own documented shape) or after Oh My Pi's \
         256-byte title slot (checked as a fallback)",
    )
}

fn identify_static_categories(home: &Path, out: &mut Vec<CandidateAgentUnit>) {
    for (rel, note) in [
        ("settings.json", "main configuration"),
        ("trust.json", "per-project trust decisions"),
        ("models.json", "custom model/provider definitions"),
    ] {
        let path = home.join(rel);
        if let Ok(meta) = fs::symlink_metadata(&path)
            && meta.is_file()
        {
            out.push(CandidateAgentUnit {
                category: AgentCategory::ProtectedConfig,
                relative_path: rel.to_string(),
                path,
                members: Vec::new(),
                bytes: meta.len(),
                mtime_max: mtime_secs(&meta),
                protected: true,
                protect_reason: Some(note.to_string()),
                project_link: ProjectLinkState::NotApplicable,
                action: AgentActionCapability::None,
                note: None,
            });
        }
    }

    let npm = home.join("npm");
    if npm.is_dir() {
        let (bytes, mtime, truncated) = folded_bytes(&npm, MAX_FOLD_ENTRIES);
        out.push(CandidateAgentUnit {
            category: AgentCategory::Caches,
            relative_path: "npm".to_string(),
            path: npm,
            members: Vec::new(),
            bytes,
            mtime_max: mtime,
            protected: false,
            protect_reason: None,
            project_link: ProjectLinkState::NotApplicable,
            action: AgentActionCapability::CacheOrLogTrash,
            note: Some(if truncated {
                "user-scoped npm package installs, reinstallable; directory entry count bound \
                 reached"
                    .to_string()
            } else {
                "user-scoped npm package installs, reinstallable".to_string()
            }),
        });
    }

    let seen: std::collections::HashSet<&str> = [
        "settings.json",
        "trust.json",
        "models.json",
        "npm",
        "sessions",
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
        let (bytes, mtime, _t) = folded_bytes(&e.path(), MAX_FOLD_ENTRIES);
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

    #[test]
    fn empty_home_yields_no_units() {
        let dir = tempfile::tempdir().unwrap();
        assert!(identify(dir.path(), 1).is_empty());
    }

    #[test]
    fn no_markers_yields_unknown_format_not_a_guess() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("unrelated.txt"), b"hello");
        let units = identify(dir.path(), 1);
        assert_eq!(units.len(), 1);
        assert_eq!(units[0].relative_path, "(unknown format)");
    }

    #[test]
    fn session_links_via_pis_own_offset_zero_header() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        touch(&home.join("settings.json"), b"{}");
        let repo = home.join("fixture-repo");
        fs::create_dir_all(repo.join(".git")).unwrap();
        let canary = "CANARY-PI-DO-NOT-LEAK-21cc";
        let jsonl = home.join("sessions/-fixture-repo/1700000000.jsonl");
        touch(
            &jsonl,
            format!(
                "{{\"id\":\"1\",\"parentId\":null,\"cwd\":\"{}\"}}\n{{\"id\":\"2\",\"parentId\":\"1\",\"content\":\"{canary}\"}}\n",
                repo.display()
            )
            .as_bytes(),
        );
        let units = identify(home, 1);
        let session = units
            .iter()
            .find(|u| u.category == AgentCategory::Sessions)
            .expect("session identified");
        assert!(matches!(
            session.project_link,
            ProjectLinkState::Linked { .. }
        ));
        assert_eq!(session.action, AgentActionCapability::SessionRemoval);
        let serialized = format!("{units:?}");
        assert!(!serialized.contains(canary), "content leaked");
    }

    #[test]
    fn session_falls_back_to_the_omp_title_slot_shape() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        touch(&home.join("settings.json"), b"{}");
        let repo = home.join("fixture-repo");
        fs::create_dir_all(repo.join(".git")).unwrap();
        let mut title = vec![b' '; 256];
        let title_json = b"{\"type\":\"title\"}";
        title[..title_json.len()].copy_from_slice(title_json);
        title[255] = b'\n';
        let header = format!("{{\"type\":\"session\",\"cwd\":\"{}\"}}\n", repo.display());
        let mut body = title;
        body.extend_from_slice(header.as_bytes());
        let jsonl = home.join("sessions/x/1.jsonl");
        touch(&jsonl, &body);
        let units = identify(home, 1);
        let session = units
            .iter()
            .find(|u| u.category == AgentCategory::Sessions)
            .expect("session identified via fallback shape");
        assert!(matches!(
            session.project_link,
            ProjectLinkState::Linked { .. }
        ));
    }

    #[test]
    fn session_with_neither_shape_is_unresolved_not_guessed() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        touch(&home.join("settings.json"), b"{}");
        let jsonl = home.join("sessions/x/1.jsonl");
        touch(&jsonl, b"not a session header at all\n");
        let units = identify(home, 1);
        let session = units.iter().find(|u| u.path == jsonl).unwrap();
        assert!(matches!(
            session.project_link,
            ProjectLinkState::Unresolved { .. }
        ));
    }

    #[test]
    fn npm_is_actionable_cache() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        touch(&home.join("npm/pkg/index.js"), b"module.exports = {}");
        let units = identify(home, 1);
        let u = units.iter().find(|u| u.relative_path == "npm").unwrap();
        assert!(!u.protected);
        assert_eq!(u.action, AgentActionCapability::CacheOrLogTrash);
    }

    #[test]
    fn settings_trust_and_models_are_protected() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        touch(&home.join("settings.json"), b"{}");
        touch(&home.join("trust.json"), b"{}");
        touch(&home.join("models.json"), b"{}");
        let units = identify(home, 1);
        for rel in ["settings.json", "trust.json", "models.json"] {
            assert!(
                units
                    .iter()
                    .find(|u| u.relative_path == rel)
                    .unwrap()
                    .protected
            );
        }
    }

    #[test]
    fn identification_cost_is_bounded_for_many_sessions() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        touch(&home.join("settings.json"), b"{}");
        let repo = home.join("big-repo");
        fs::create_dir_all(repo.join(".git")).unwrap();
        for i in 0..500 {
            let jsonl = home.join(format!("sessions/-big-repo/{i}.jsonl"));
            let mut body =
                format!("{{\"id\":\"1\",\"cwd\":\"{}\"}}\n", repo.display()).into_bytes();
            body.extend_from_slice(&b"x".repeat(200_000));
            touch(&jsonl, &body);
        }
        let start = SystemTime::now();
        let units = identify(home, 1);
        let elapsed = SystemTime::now().duration_since(start).unwrap_or_default();
        eprintln!("[measured] pi identify() over 500 synthetic sessions took {elapsed:?}");
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
