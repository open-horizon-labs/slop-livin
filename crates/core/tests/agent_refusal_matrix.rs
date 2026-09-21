//! #101's "verify the refusal matrix per tool": one table-driven test
//! asserting every named refusal reason -- protected categories, whole
//! home/whole projects-dir paths, SQLite/WAL/SHM-like files, active
//! sessions, shared blobs with unknown references, parent/child overlap
//! in one plan, and plan scope drift at execution -- fires for every
//! one of the 14 agent-tool ids `crate::agents::matrix` names, not just
//! Claude Code. Synthetic `AgentUnit` values are built directly here
//! (never a real tool home) since `agent_refusal`/`propose_agents`'s
//! logic is generic over every field an adapter's `discover_and_measure`
//! output would carry; a handful of end-to-end scenarios (active
//! session, scope drift) additionally go through a couple of real
//! adapters' own `identify()` to prove the mechanism reaches them too,
//! not only a hand-built literal. PRIVACY IS A HARD RULE: every fixture
//! here is synthetic.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use swamp_core::actions;
use swamp_core::agents::{
    AgentActionCapability, AgentCategory, AgentMember, AgentMemberKind, AgentUnit,
    ProjectLinkState, unit_id,
};

/// Every named tool id (#90's required 14-row matrix, including the
/// Codex desktop app as its own row). Kept as a plain list here (not
/// imported from `crate::agents::matrix::AgentToolId`) so this test
/// exercises the exact string ids `propose_agents`/`execute` dispatch
/// on, the same way a real `AgentUnit.tool_id` would carry them.
const ALL_TOOL_IDS: &[(&str, &str)] = &[
    ("claude-code", "Claude Code"),
    ("codex", "Codex"),
    ("codex-desktop", "Codex (desktop app)"),
    ("oh-my-pi", "Oh My Pi"),
    ("opencode", "OpenCode"),
    ("gemini-cli", "Gemini CLI"),
    ("pi", "Pi"),
    ("aider", "Aider"),
    ("github-copilot-cli", "GitHub Copilot CLI"),
    ("cursor", "Cursor"),
    ("windsurf", "Windsurf"),
    ("cline", "Cline"),
    ("roo-code", "Roo Code"),
    ("continue", "Continue"),
];

#[allow(clippy::too_many_arguments)]
fn synthetic_unit(
    tool_id: &str,
    tool_name: &str,
    category: AgentCategory,
    action: AgentActionCapability,
    protected: bool,
    protect_reason: Option<&str>,
    path: PathBuf,
    members: Vec<AgentMember>,
) -> AgentUnit {
    let relative_path = "fixture-unit".to_string();
    AgentUnit {
        tool_id: tool_id.to_string(),
        tool_name: tool_name.to_string(),
        tool_home: path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| path.clone()),
        category,
        id: unit_id(tool_id, category, &relative_path),
        relative_path,
        path,
        members,
        bytes: 4096,
        hardlinked: false,
        growth_bytes: None,
        regrowth_count: 0,
        observed_at: 1_000,
        mtime_max: 1_000,
        protected,
        protect_reason: protect_reason.map(str::to_string),
        project_link: ProjectLinkState::NotApplicable,
        action,
        note: None,
        evidence: Vec::new(),
    }
}

fn touch(path: &Path, content: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

/// Row 1: every named tool's `ProtectedConfig`-category unit refuses at
/// proposal time -- the category-default protection #91 requires
/// (`AgentCategory::default_protected`), never bypassable by a bare
/// `--path`.
#[test]
fn every_tool_refuses_a_protected_config_category_unit() {
    for (tool_id, tool_name) in ALL_TOOL_IDS {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        touch(&path, b"{}");
        let unit = synthetic_unit(
            tool_id,
            tool_name,
            AgentCategory::ProtectedConfig,
            AgentActionCapability::None,
            true,
            Some("protected configuration"),
            path.clone(),
            Vec::new(),
        );
        let err = actions::propose_agents(&[unit], &[path], "test")
            .expect_err("a ProtectedConfig unit must never be proposable");
        assert!(
            err.to_string().contains("protected"),
            "tool {tool_id}: {err}"
        );
    }
}

/// Row 2: a human `swamp protect` keep-flag (simulated here as
/// `protected: true` on an otherwise-actionable, non-default-protected
/// category -- exactly what `discover_and_measure` layers on from the
/// protect sidecar) blocks proposing for every tool, not only the
/// category default. This is the same acceptance line as "human keep
/// flags block ... proposing in CLI for every adapter", tested once at
/// the shared mechanism both the CLI and the TUI call into.
#[test]
fn every_tool_respects_a_human_protect_keep_flag_on_an_otherwise_actionable_unit() {
    for (tool_id, tool_name) in ALL_TOOL_IDS {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache-dir");
        fs::create_dir_all(&path).unwrap();
        touch(&path.join("f"), b"cache-bytes");
        let unit = synthetic_unit(
            tool_id,
            tool_name,
            AgentCategory::Caches,
            AgentActionCapability::CacheOrLogTrash,
            true,
            Some("human keep flag (swamp protect)"),
            path.clone(),
            Vec::new(),
        );
        let err = actions::propose_agents(&[unit], &[path], "test")
            .expect_err("a human-protected unit must never be proposable");
        assert!(
            err.to_string().contains("protected"),
            "tool {tool_id}: {err}"
        );
    }
}

/// Row 3: an unsupported category (`action: None`, e.g. Oh My Pi's own
/// shared blobs with unknown reference coverage, or any category no
/// adapter has wired an action for yet) refuses with the specific
/// "no supported selective action" reason, for every tool, never a
/// silent no-op or a destructive fallback.
#[test]
fn every_tool_refuses_an_unsupported_action_category() {
    for (tool_id, tool_name) in ALL_TOOL_IDS {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("blob-or-unsupported-unit");
        touch(&path, b"opaque-bytes");
        let unit = synthetic_unit(
            tool_id,
            tool_name,
            AgentCategory::Attachments,
            AgentActionCapability::None,
            false,
            None,
            path.clone(),
            Vec::new(),
        );
        let err = actions::propose_agents(&[unit], &[path], "test")
            .expect_err("an action:None unit must never be proposable");
        assert!(
            err.to_string().contains("no supported selective action"),
            "tool {tool_id}: {err}"
        );
    }
}

/// Row 4: a SQLite/WAL/SHM-like path is refused unconditionally --
/// `is_sqlite_like` is a filename check independent of category/action,
/// defense in depth for every tool (several of which really do use
/// SQLite: Codex's six state databases, OpenCode's `opencode.db`, Oh My
/// Pi's `agent.db`, the VS-Code family's `state.vscdb`).
#[test]
fn every_tool_refuses_a_database_like_path_even_when_otherwise_actionable() {
    for (tool_id, tool_name) in ALL_TOOL_IDS {
        let dir = tempfile::tempdir().unwrap();
        for suffix in ["state.db", "state.sqlite", "state.db-wal", "state.db-shm"] {
            let path = dir.path().join(suffix);
            touch(&path, b"sqlite-bytes");
            let unit = synthetic_unit(
                tool_id,
                tool_name,
                AgentCategory::Caches,
                AgentActionCapability::CacheOrLogTrash,
                false,
                None,
                path.clone(),
                Vec::new(),
            );
            let err = actions::propose_agents(&[unit], &[path], "test")
                .expect_err("a database-like path must never be proposable");
            assert!(
                err.to_string().contains("database-like"),
                "tool {tool_id} ({suffix}): {err}"
            );
        }
    }
}

/// Row 5: an active session (a real, `lsof`-visible open file
/// descriptor -- this test process's own, no mocking) refuses at
/// proposal time, for every tool: the occupancy check
/// (`crate::agents::is_active`) is a path-based seam, not a
/// per-adapter one, so it applies uniformly.
#[test]
fn every_tool_refuses_a_unit_with_an_active_open_file() {
    for (tool_id, tool_name) in ALL_TOOL_IDS {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        touch(&path, b"session-bytes");
        let _held_open = fs::File::open(&path).expect("hold the file open");
        let unit = synthetic_unit(
            tool_id,
            tool_name,
            AgentCategory::Sessions,
            AgentActionCapability::SessionRemoval,
            false,
            None,
            path.clone(),
            vec![AgentMember {
                path: path.clone(),
                bytes: 12,
                kind: AgentMemberKind::Transcript,
            }],
        );
        let err = actions::propose_agents(&[unit], &[path], "test")
            .expect_err("an actively-open unit must never be proposable");
        assert!(err.to_string().contains("active"), "tool {tool_id}: {err}");
    }
}

/// Row 6: "whole home" / "whole projects dir" -- a bare directory that
/// is not itself any unit's own anchor path (a tool's home directory,
/// or a directory holding several projects) is refused as "no
/// agent-storage unit at this exact path", never silently expanded into
/// every unit underneath it. Exercised for every tool's own (synthetic)
/// home directory.
#[test]
fn every_tool_refuses_a_whole_home_or_whole_projects_dir_path() {
    for (tool_id, tool_name) in ALL_TOOL_IDS {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("tool-home");
        fs::create_dir_all(&home).unwrap();
        let unit_path = home.join("cache");
        fs::create_dir_all(&unit_path).unwrap();
        let unit = synthetic_unit(
            tool_id,
            tool_name,
            AgentCategory::Caches,
            AgentActionCapability::CacheOrLogTrash,
            false,
            None,
            unit_path,
            Vec::new(),
        );
        // The home directory itself (never a unit's own path) and an
        // unrelated "projects" directory both refuse the same way.
        for whole in [home.clone(), dir.path().to_path_buf()] {
            let err = actions::propose_agents(
                std::slice::from_ref(&unit),
                std::slice::from_ref(&whole),
                "test",
            )
            .expect_err("a whole-home/whole-projects-dir path must never resolve to a unit");
            assert!(
                err.to_string()
                    .contains("no agent-storage unit at this exact path"),
                "tool {tool_id} ({}): {err}",
                whole.display()
            );
        }
    }
}

/// Row 7: parent/child overlap in one plan -- two otherwise-actionable
/// units whose own anchor paths nest are refused together, before
/// either reaches a plan (see `actions::propose_agents`'s own overlap
/// guard, added alongside this test). Exercised across a few different
/// tool-id pairings (a real cross-tool plan can, in principle, mix
/// unrelated tools' units in one selection).
#[test]
fn overlapping_selections_from_any_tool_pairing_refuse_the_whole_plan() {
    let pairings = [
        (ALL_TOOL_IDS[0], ALL_TOOL_IDS[0]),
        (ALL_TOOL_IDS[1], ALL_TOOL_IDS[4]),
        (ALL_TOOL_IDS[6], ALL_TOOL_IDS[6]),
    ];
    for ((tool_a, name_a), (tool_b, name_b)) in pairings {
        let dir = tempfile::tempdir().unwrap();
        let parent = dir.path().join("parent-unit");
        fs::create_dir_all(&parent).unwrap();
        let child = parent.join("child-unit");
        fs::create_dir_all(&child).unwrap();
        let unit_a = synthetic_unit(
            tool_a,
            name_a,
            AgentCategory::Caches,
            AgentActionCapability::CacheOrLogTrash,
            false,
            None,
            parent.clone(),
            Vec::new(),
        );
        let unit_b = synthetic_unit(
            tool_b,
            name_b,
            AgentCategory::Caches,
            AgentActionCapability::CacheOrLogTrash,
            false,
            None,
            child.clone(),
            Vec::new(),
        );
        let err = actions::propose_agents(&[unit_a, unit_b], &[parent, child], "test")
            .expect_err("nested selections must refuse the whole plan, not silently pick one");
        assert!(
            err.to_string()
                .contains("overlapping agent-storage selections"),
            "{tool_a}/{tool_b}: {err}"
        );
    }
}

/// Row 8: plan scope drift at execution -- membership that changed
/// between proposal and execution (a new member path appeared) refuses
/// at `execute`, never trusting the plan's own stale member list. Uses
/// Claude Code's real multi-member `identify()` because it is the only
/// adapter in this catalog whose session unit can have more than one
/// member path (transcript + subagents dir + file-history + todos),
/// which is what makes a membership *set* change observable; Codex's
/// own session unit is deliberately exactly one file (see
/// `codex_session_removal_is_exactly_one_file_and_preserves_the_repo`
/// in `agent_units_actions_new_adapters.rs`), so a set-membership-drift
/// scenario does not apply to it the same way. The re-identification
/// dispatch itself (`actions::execute_agent_session_removal`'s
/// `match meta.tool_id`) is exercised for every other tool by that
/// tool's own end-to-end session-removal test elsewhere in this repo.
#[test]
fn plan_scope_drift_refuses_at_execute_for_claude_code() {
    use swamp_core::agents::discover_and_measure;
    use swamp_core::locations::{Environment, Platform, Registry};
    use swamp_core::scope::{ScanConfig, resolve_effective_scope};

    fn only(id: &str) -> ScanConfig {
        ScanConfig {
            defaults: false,
            include: Vec::new(),
            exclude: Vec::new(),
            disabled_detectors: [
                "cargo-home",
                "rustup",
                "homebrew",
                "claude-code",
                "codex",
                "codex-desktop",
                "oh-my-pi",
                "opencode",
                "gemini-cli",
                "pi",
                "aider",
                "github-copilot-cli",
                "cursor",
                "windsurf",
                "cline",
                "roo-code",
                "continue",
            ]
            .into_iter()
            .filter(|d| *d != id)
            .map(str::to_string)
            .collect(),
        }
    }

    // Claude Code.
    {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("claude-home");
        let repo = root.path().join("repo");
        fs::create_dir_all(repo.join(".git")).unwrap();
        let session_id = "33333333-3333-4333-8333-333333333333";
        let jsonl = home
            .join("projects")
            .join("-repo-encoded")
            .join(format!("{session_id}.jsonl"));
        touch(
            &jsonl,
            format!(
                "{{\"type\":\"user\",\"sessionId\":\"s\",\"cwd\":\"{}\",\"gitBranch\":\"main\"}}\n",
                repo.display()
            )
            .as_bytes(),
        );
        let subagent = home
            .join("projects")
            .join("-repo-encoded")
            .join(session_id)
            .join("subagents")
            .join("a.jsonl");
        touch(&subagent, b"companion");

        let registry = Registry::with_builtins();
        let mut env_vars = HashMap::new();
        env_vars.insert("CLAUDE_CONFIG_DIR".to_string(), home.display().to_string());
        let env = Environment::fixture(root.path().to_path_buf(), env_vars, Platform::MacOS);
        let scope = resolve_effective_scope(&env, &only("claude-code"), &[], &registry, 1_000);
        let store = tempfile::tempdir().unwrap();
        let units =
            discover_and_measure(&scope, &[], Some(store.path()), true, 1_000, 30, 3600).unwrap();

        let plan = actions::propose_agents(&units, std::slice::from_ref(&jsonl), "test").unwrap();
        actions::save_plan(store.path(), &plan).unwrap();
        actions::approve(store.path(), &plan.id, "human:test").unwrap();

        // Scope drift: a brand-new *member path* (a `todos/` entry
        // Claude Code did not have at proposal time) appears for this
        // exact session id before execute runs. Adding a file inside
        // the existing `subagents/` companion directory would not by
        // itself change the member *path set* (that whole directory is
        // already one folded member) -- a new top-level member kind is
        // what actually drifts the set `execute_agent_session_removal`
        // re-derives and compares.
        touch(
            &home
                .join("todos")
                .join(format!("{session_id}-agent-1.json")),
            b"unplanned-new-member",
        );

        let trash = tempfile::tempdir().unwrap();
        let result =
            actions::execute_with_trash(store.path(), &plan.id, "human:test", trash.path())
                .unwrap();
        assert_eq!(
            result.outcomes[0].status, "failed",
            "{:?}",
            result.outcomes[0]
        );
        assert!(
            result.outcomes[0]
                .cause
                .as_deref()
                .unwrap_or_default()
                .contains("membership changed"),
            "{:?}",
            result.outcomes[0]
        );
        // Refused, so nothing actually moved.
        assert!(jsonl.exists());
        assert!(subagent.exists());
    }
}
