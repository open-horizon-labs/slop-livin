//! The explicit major-tool matrix #90/#91 require: every named tool is
//! listed here, always -- never a silent omission and never an empty
//! placeholder adapter that claims support it does not have. Each row
//! states its home-path evidence and primary source(s), researched
//! during implementation (never guessed, never learned from a real
//! `~/.claude`-style directory on this machine).
//!
//! [`AgentToolId::ClaudeCode`] (#92), [`AgentToolId::Codex`] and
//! [`AgentToolId::CodexDesktop`] (#93), [`AgentToolId::OhMyPi`] (#94),
//! [`AgentToolId::OpenCode`] (#95), [`AgentToolId::GeminiCli`],
//! [`AgentToolId::Pi`], [`AgentToolId::Aider`] (#96),
//! [`AgentToolId::GithubCopilotCli`] (#97), [`AgentToolId::Cursor`] and
//! [`AgentToolId::Windsurf`] (#98), and [`AgentToolId::Cline`],
//! [`AgentToolId::RooCode`] and [`AgentToolId::Continue`] (#99) are all
//! [`SupportLevel::Supported`] as of this chunk: every row in this
//! table now has a real identification adapter
//! (`crate::agents::{claude_code, codex, codex_desktop, oh_my_pi,
//! opencode, gemini_cli, pi, aider, copilot_cli, cursor, windsurf,
//! cline, roo_code, continue_dev}`), completing the full named-tool
//! catalog #90/#91 require. `docs/agent-storage.md` renders this same
//! table as a doc; keep the two in sync by hand (this module is the
//! single source of truth, generated into the doc, not the reverse).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentToolId {
    ClaudeCode,
    Codex,
    /// The Codex **desktop app** (`Codex.app`), a materially different
    /// client from the Codex CLI with its own storage -- modeled as its
    /// own row rather than folded into `Codex`'s, per #93's explicit
    /// "do not extrapolate one client's schema to all clients"
    /// acceptance.
    CodexDesktop,
    OhMyPi,
    OpenCode,
    GeminiCli,
    Pi,
    Aider,
    GithubCopilotCli,
    Cursor,
    Windsurf,
    Cline,
    RooCode,
    Continue,
}

impl AgentToolId {
    pub fn slug(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude-code",
            Self::Codex => "codex",
            Self::CodexDesktop => "codex-desktop",
            Self::OhMyPi => "oh-my-pi",
            Self::OpenCode => "opencode",
            Self::GeminiCli => "gemini-cli",
            Self::Pi => "pi",
            Self::Aider => "aider",
            Self::GithubCopilotCli => "github-copilot-cli",
            Self::Cursor => "cursor",
            Self::Windsurf => "windsurf",
            Self::Cline => "cline",
            Self::RooCode => "roo-code",
            Self::Continue => "continue",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SupportLevel {
    /// Real identification code exists (`crate::agents::<tool>` +, for
    /// the home directory itself, a `crate::locations` detector).
    Supported,
    /// Home-path evidence is researched and recorded below; no
    /// identification code exists yet. Never rendered as "unknown" (an
    /// unresearched gap) or silently omitted from the matrix.
    Planned,
}

/// One row of the required major-tool matrix. Serialize-only: this table
/// is a compiled-in constant, never parsed back from JSON.
#[derive(Debug, Clone, Serialize)]
pub struct MatrixEntry {
    pub id: AgentToolId,
    pub display_name: &'static str,
    pub support: SupportLevel,
    /// Documented home directory / override, in prose (paths vary by
    /// platform for several of these tools; see `note`).
    pub home_note: &'static str,
    /// Primary sources consulted for `home_note`, recorded so a later
    /// implementer can re-verify against the current documentation
    /// rather than trusting this table blind.
    pub sources: &'static [&'static str],
    /// Caveats: version-dependence, platform variance, or "community-
    /// sourced, no single official layout doc found" honesty notes.
    pub note: &'static str,
}

/// The full required matrix, in the epic's stated priority order
/// (Claude Code, Codex, Oh My Pi, OpenCode, then the remaining named
/// tools). Extending this list for a *new* tool beyond the named set is
/// ordinary catalog review (#90's own text); this constant is not itself
/// meant to be exhaustive of every coding agent that will ever exist.
pub const MATRIX: &[MatrixEntry] = &[
    MatrixEntry {
        id: AgentToolId::ClaudeCode,
        display_name: "Claude Code",
        support: SupportLevel::Supported,
        home_note: "~/.claude, or $CLAUDE_CONFIG_DIR if set; a sibling ~/.claude.json also \
                     exists outside the home directory and is not modeled (see \
                     crate::locations::claude_code doc comment)",
        sources: &[
            "https://code.claude.com/docs/en/claude-directory",
            "https://code.claude.com/docs/en/settings",
            "https://code.claude.com/docs/en/checkpointing",
            "https://code.claude.com/docs/en/authentication",
        ],
        note: "current documented layout as of this chunk; the transcript JSONL schema itself \
               is explicitly documented upstream as internal/unstable across versions",
    },
    MatrixEntry {
        id: AgentToolId::Codex,
        display_name: "Codex",
        support: SupportLevel::Supported,
        home_note: "CODEX_HOME, default ~/.codex; sessions/ and archived_sessions/ (year/month/ \
                     day rollout-*.jsonl trees), auth.json, history.jsonl, config.toml, log/, \
                     six SQLite state stores (state_5/logs_2/goals_1/memories_1/queue_1/ \
                     thread_history_1.sqlite) relocatable via the separate CODEX_SQLITE_HOME",
        sources: &[
            "https://github.com/openai/codex/blob/main/codex-rs/utils/home-dir/src/lib.rs",
            "https://github.com/openai/codex/blob/main/codex-rs/rollout/src/lib.rs",
            "https://github.com/openai/codex/blob/main/codex-rs/rollout/src/list.rs",
            "https://github.com/openai/codex/blob/main/codex-rs/rollout/src/rollout_file_name.rs",
            "https://github.com/openai/codex/blob/main/codex-rs/rollout/src/metadata.rs",
            "https://github.com/openai/codex/blob/main/codex-rs/state/src/sqlite.rs",
            "https://github.com/openai/codex/blob/main/codex-rs/app-server/src/codex_home_metrics.rs",
        ],
        note: "#93; crate::agents::codex implements identification. The session-header envelope \
               nesting around `cwd` is not pinned to one shape (internal, version-varying wire \
               format); no managed-worktree creation by the CLI itself is confirmed",
    },
    MatrixEntry {
        id: AgentToolId::CodexDesktop,
        display_name: "Codex desktop app",
        support: SupportLevel::Supported,
        home_note: "macOS only, confirmed: ~/Library/Logs/com.openai.codex (date-tree session \
                     logs). Settings/session storage beyond logs is not confirmed by primary \
                     source and is not modeled -- logs-only support, stated explicitly rather \
                     than silently treated as empty",
        sources: &[
            "https://github.com/openai/codex/blob/main/codex-rs/cli/src/doctor/desktop.rs",
            "https://github.com/openai/codex/blob/main/codex-rs/cli/src/doctor/desktop/platform.rs",
        ],
        note: "#93; crate::agents::codex_desktop implements identification for the confirmed \
               log directory only, a deliberately partial Supported row",
    },
    MatrixEntry {
        id: AgentToolId::OhMyPi,
        display_name: "Oh My Pi",
        support: SupportLevel::Supported,
        home_note: "~/.omp/agent (user-confirmed identity: Oh My Pi, a fork of badlogic/ \
                     pi-mono), or the whole of PI_CODING_AGENT_DIR when set; sessions/ \
                     <encoded-cwd>/<ts>_<session-id>.jsonl, content-addressed blobs/<sha256>, \
                     terminal-sessions/, config.yml/config.yaml, models.yml, agent.db (SQLite \
                     auth store)",
        sources: &[
            "https://github.com/can1357/oh-my-pi/blob/main/docs/session.md",
            "https://github.com/can1357/oh-my-pi/blob/main/docs/settings.md",
        ],
        note: "#94; crate::agents::oh_my_pi implements identification, including a content-marker \
               check that reports an explicit unknown-format unit rather than guessing when \
               ~/.omp is not actually Oh My Pi's own layout, and bounded per-session blob-\
               reference accounting that never offers blob removal in this chunk",
    },
    MatrixEntry {
        id: AgentToolId::OpenCode,
        display_name: "OpenCode",
        support: SupportLevel::Supported,
        home_note: "data: OPENCODE_DATA_DIR (unconfirmed env var name, honored defensively), \
                     else ${XDG_DATA_HOME:-~/.local/share}/opencode (auth.json, log/, \
                     storage/{session,message,part,session_diff,project}/ or opencode.db \
                     depending on version, snapshot/<project-id>/<hash> git-backed checkpoints); \
                     config ${XDG_CONFIG_HOME:-~/.config}/opencode (opaque external unit); cache \
                     ${XDG_CACHE_HOME:-~/.cache}/opencode (opaque external unit)",
        sources: &[
            "https://opencode.ai/docs/troubleshooting/",
            "https://github.com/anomalyco/opencode/issues/6669",
            "https://github.com/anomalyco/opencode/issues/18633",
            "https://github.com/sst/opencode/blob/dev/packages/opencode/src/storage/storage.ts",
            "https://deepwiki.com/sst/opencode/2.9-storage-and-database",
        ],
        note: "#95; crate::agents::opencode implements identification for both the older file- \
               tree layout and the newer SQLite-backed one, version-gated by which markers are \
               present on disk; an unrecognized layout is reported as one explicit \
               unsupported-version unit rather than guessed at either schema",
    },
    MatrixEntry {
        id: AgentToolId::GeminiCli,
        display_name: "Gemini CLI",
        support: SupportLevel::Supported,
        home_note: "~/.gemini, or the whole of GEMINI_CLI_HOME when set (settings.json, \
                     GEMINI.md, extensions/, trustedFolders.json, bin/); tmp/<project-hash>/ \
                     (shell_history, checkpoints/, chats/) and history/<project-hash>/ (shadow \
                     Git checkpoint repos), both keyed by sha256(project root path)",
        sources: &[
            "https://github.com/google-gemini/gemini-cli/blob/main/docs/reference/configuration.md",
            "https://github.com/google-gemini/gemini-cli/blob/main/docs/cli/checkpointing.md",
            "https://github.com/google-gemini/gemini-cli/blob/main/docs/cli/session-management.md",
            "https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/utils/paths.ts",
        ],
        note: "#96; crate::agents::gemini_cli implements identification. GEMINI_CLI_HOME is the \
               real override (correcting this row's own prior GEMINI_CONFIG_HOME guess); the \
               project hash is a confirmed one-way sha256, so tmp/history directories carry an \
               honest Unresolved linkage rather than a guess. OAuth/account credential file names \
               are not documented upstream as of this chunk and are protected defensively by \
               filename pattern instead of an exact confirmed name",
    },
    MatrixEntry {
        id: AgentToolId::Pi,
        display_name: "Pi",
        support: SupportLevel::Supported,
        home_note: "~/.pi/agent/ by default, overridable via PI_CODING_AGENT_DIR (confirmed \
                     primary-source name, shared with Oh My Pi's own override -- see \
                     crate::locations::pi; PI_AGENT_DIR, the issue text's name, is honored as an \
                     unconfirmed secondary override); sessions/ (organized by working directory), \
                     settings.json, trust.json, models.json, npm/ (badlogic/pi-mono, also \
                     published as earendil-works/pi)",
        sources: &[
            "https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/settings.md",
            "https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/README.md",
        ],
        note: "#96; crate::agents::pi implements identification, distinct from Oh My Pi (a fork \
               of this project) even though the two share an override variable name -- see \
               crate::locations::pi's doc comment for the disclosed collision risk. Session \
               linkage tries Pi's own documented offset-zero JSON header first, then Oh My Pi's \
               256-byte title-slot shape as an explicit fallback, never assumed",
    },
    MatrixEntry {
        id: AgentToolId::Aider,
        display_name: "Aider",
        support: SupportLevel::Supported,
        home_note: "~/.aider/caches (model_prices_and_context_window.json, versioncheck; both \
                     wholly re-downloadable), plus an optional home-level .aider.conf.yml; \
                     per-repo .aider.chat.history.md / .aider.input.history and \
                     .aider.tags.cache.v{3,4}/ at the git root -- project-local, not under the \
                     home directory",
        sources: &[
            "https://github.com/Aider-AI/aider/blob/main/aider/models.py",
            "https://github.com/Aider-AI/aider/blob/main/aider/versioncheck.py",
            "https://github.com/Aider-AI/aider/blob/main/aider/args.py",
            "https://github.com/Aider-AI/aider/blob/main/aider/repomap.py",
        ],
        note: "#96; crate::agents::aider implements both halves: identify() for the home-level \
               caches, and identify_repo_units() attached per known project worktree root (via \
               crate::agents::discover_and_measure's project_worktrees parameter) for the \
               per-repo files, per this issue's own explicit 'attach to the worktree artifact \
               model' acceptance",
    },
    MatrixEntry {
        id: AgentToolId::GithubCopilotCli,
        display_name: "GitHub Copilot CLI",
        support: SupportLevel::Supported,
        home_note: "~/.copilot, overridable via COPILOT_HOME (config.json, settings.json, \
                     mcp-config.json, permissions-config.json, providers.json, \
                     copilot-instructions.md, agents/, hooks/, skills/, extensions/, \
                     installed-plugins/, plugin-data/, mcp-oauth-config/, mcp-secrets/, ide/, \
                     session-state/, command-history-state/, session-store.db, logs/); separate \
                     platform-conventional cache (~/Library/Caches/copilot on macOS), \
                     overridable via COPILOT_CACHE_HOME, independent of COPILOT_HOME",
        sources: &[
            "https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-config-dir-reference",
        ],
        note: "#97; crate::agents::copilot_cli implements identification. session-state/ and \
               command-history-state/ are the real directory names, correcting this row's own \
               prior history-session-state/ guess",
    },
    MatrixEntry {
        id: AgentToolId::Cursor,
        display_name: "Cursor",
        support: SupportLevel::Supported,
        home_note: "editor-profile storage, macOS only this chunk: ~/Library/Application \
                     Support/Cursor (User/globalStorage/state.vscdb, \
                     User/workspaceStorage/<id>/{state.vscdb,workspace.json}, User/History, \
                     Cache/CachedData/CachedExtensionVSIXs/logs), plus a separate ~/.cursor/ \
                     (chats/, projects/, CLI state, not decomposed)",
        sources: &[
            "https://github.com/thomas-pedersen/cursor-chat-browser",
            "https://github.com/Callum-Ward/cursaves/blob/main/docs/how-cursor-stores-chats.md",
        ],
        note: "#98; crate::agents::cursor implements identification via the shared \
               crate::agents::vscode_family module. No single official Cursor documentation \
               page describing this layout was found -- community-reverse-engineered, lower \
               confidence than primary-source-backed rows. Linux (~/.config/Cursor/...) is \
               deferred to the independent Linux track (#77-#89)",
    },
    MatrixEntry {
        id: AgentToolId::Windsurf,
        display_name: "Windsurf",
        support: SupportLevel::Supported,
        home_note: "~/.codeium/windsurf (MCP/agent config, not decomposed) plus an assumed \
                     VS-Code-fork editor profile at ~/Library/Application Support/Windsurf, \
                     macOS only this chunk",
        sources: &["https://registry.coder.com/modules/coder/windsurf"],
        note: "#98; crate::agents::windsurf implements identification, reusing \
               crate::agents::vscode_family. Lower confidence than every other row: \
               docs.windsurf.com redirected to docs.devin.ai during this chunk's research, so \
               the Application Support/Windsurf shape is assumed (VS Code fork), not \
               independently confirmed -- an installation that differs surfaces as an honest \
               '(unsupported layout version)' residual rather than a silent miscount",
    },
    MatrixEntry {
        id: AgentToolId::Cline,
        display_name: "Cline",
        support: SupportLevel::Supported,
        home_note: "VS Code extension global storage, one location per known editor host \
                     (macOS: Code, Code - Insiders, Cursor, Windsurf, plus \
                     ~/.vscode-server/data/... for a remote/devcontainer target): \
                     globalStorage/saoudrizwan.claude-dev/tasks/<task-id>/ holding \
                     api_conversation_history.json, ui_messages.json, task_metadata.json, \
                     checkpoints",
        sources: &[
            "https://github.com/cline/cline/issues/7101",
            "https://github.com/cline/cline/issues/14135",
        ],
        note: "#99; crate::agents::cline implements identification via \
               crate::agents::vscode_family, decomposing every resolved host location rather \
               than just the first (crate::agents::multi_location_tool). No single official \
               layout-reference doc found (community-documented via GitHub issue threads); the \
               task_metadata.json 'workspace' field used for project linkage is this chunk's \
               own research, not independently re-confirmed against a schema doc",
    },
    MatrixEntry {
        id: AgentToolId::RooCode,
        display_name: "Roo Code",
        support: SupportLevel::Supported,
        home_note: "VS Code extension global storage, same per-host modeling as Cline: \
                     globalStorage/rooveterinaryinc.roo-cline/tasks/<task-id>/ (remote/server \
                     hosting: ~/.vscode-server/data/User/globalStorage/... confirmed directly by \
                     the cited issue)",
        sources: &["https://github.com/RooCodeInc/Roo-Code/issues/4174"],
        note: "#99; crate::agents::roo_code implements identification via \
               crate::agents::vscode_family. Community-documented (GitHub issue), not an \
               official layout-reference page; a task directory has been reported to embed a \
               full Git checkpoint repo, so its folded byte total is not necessarily small",
    },
    MatrixEntry {
        id: AgentToolId::Continue,
        display_name: "Continue",
        support: SupportLevel::Supported,
        home_note: "~/.continue (config.yaml/config.json, sessions/<session-id> plus a session \
                     index file, separate from the session bodies; index/ embeddings/tag caches; \
                     dev_data/ anonymized usage events)",
        sources: &[
            "https://docs.continue.dev/customize/deep-dives/configuration",
            "https://docs.continue.dev/reference",
        ],
        note: "#99; crate::agents::continue_dev implements identification. config.yaml's \
               location is confirmed by primary docs; sessions/index/dev_data's presence is \
               treated as a version marker (this catalog's own prior research, not \
               independently re-confirmed this chunk) rather than an asserted schema. No \
               confirmed per-session workspace-linkage field was found, so sessions carry an \
               honest Unresolved link rather than a guess from the session id",
    },
];

pub fn entry(id: AgentToolId) -> &'static MatrixEntry {
    MATRIX
        .iter()
        .find(|e| e.id == id)
        .expect("every AgentToolId has a MATRIX row")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_named_tool_is_present_exactly_once() {
        let ids = [
            AgentToolId::ClaudeCode,
            AgentToolId::Codex,
            AgentToolId::CodexDesktop,
            AgentToolId::OhMyPi,
            AgentToolId::OpenCode,
            AgentToolId::GeminiCli,
            AgentToolId::Pi,
            AgentToolId::Aider,
            AgentToolId::GithubCopilotCli,
            AgentToolId::Cursor,
            AgentToolId::Windsurf,
            AgentToolId::Cline,
            AgentToolId::RooCode,
            AgentToolId::Continue,
        ];
        assert_eq!(
            MATRIX.len(),
            ids.len(),
            "matrix must list every named tool, no more no less"
        );
        for id in ids {
            let matches = MATRIX.iter().filter(|e| e.id == id).count();
            assert_eq!(matches, 1, "{id:?} must appear exactly once");
        }
    }

    #[test]
    fn every_named_tool_is_now_supported() {
        // #96-#99 complete the full named-tool catalog #90/#91 require;
        // no row is Planned any more.
        for e in MATRIX {
            assert_eq!(e.support, SupportLevel::Supported, "{:?}", e.id);
        }
    }

    #[test]
    fn every_row_has_at_least_one_source_and_a_nonempty_home_note() {
        for e in MATRIX {
            assert!(!e.sources.is_empty(), "{:?} has no recorded source", e.id);
            assert!(
                !e.home_note.trim().is_empty(),
                "{:?} has no home_note",
                e.id
            );
            for s in e.sources {
                assert!(
                    s.starts_with("https://"),
                    "{:?} source {s:?} is not a URL",
                    e.id
                );
            }
        }
    }
}
