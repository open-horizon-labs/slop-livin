//! The explicit major-tool matrix #90/#91 require: every named tool is
//! listed here, always -- never a silent omission and never an empty
//! placeholder adapter that claims support it does not have. Each row
//! states its home-path evidence and primary source(s), researched
//! during implementation (never guessed, never learned from a real
//! `~/.claude`-style directory on this machine).
//!
//! ## Every row used to say `Supported`. Two of them had not earned it.
//!
//! The 2026-09-21 review's objection: having an adapter is not the same
//! as having confirmed the layout that adapter models, and this table
//! conflated the two -- while several rows' own `note` fields admitted
//! an *assumed* Windsurf shape and *unconfirmed* Cline/Roo Code project
//! fields. [`SupportLevel::Unverified`] exists so those are a level
//! rather than a footnote, and
//! `crate::agents::discover_and_measure` withholds every action and
//! every project link for an `Unverified` tool. Identification still
//! runs: knowing roughly where the bytes are is useful, offering to move
//! them on an unconfirmed layout is not.
//!
//! Each row now carries [`MatrixEntry::verification`]: what was checked,
//! against which upstream path and commit, on what date. A row without
//! one cannot be `Supported`, and the test at the bottom of this file
//! enforces that. `docs/agent-storage.md` renders this same table as a
//! doc and `crates/core/tests/agent_matrix_matches_docs.rs` parses the
//! doc back and compares it with this constant, so the two cannot
//! drift.

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
    /// the home directory itself, a `crate::locations` detector) **and**
    /// the layout it models is confirmed against that tool's own source
    /// or documentation, cited in [`MatrixEntry::verification`].
    Supported,
    /// Identification code exists, but the layout it models could not be
    /// confirmed against the tool's own source or documentation.
    /// Units are still identified and measured; no action is offered and
    /// project linkage is reported `Unresolved`, because both would rest
    /// on the layout this level says is unconfirmed. Enforced in
    /// `crate::agents::discover_and_measure`, not left to each adapter.
    Unverified,
    /// Home-path evidence is researched and recorded below; no
    /// identification code exists yet. Never rendered as "unknown" (an
    /// unresearched gap) or silently omitted from the matrix.
    Planned,
}

impl SupportLevel {
    pub fn label(self) -> &'static str {
        match self {
            Self::Supported => "supported",
            Self::Unverified => "unverified",
            Self::Planned => "planned",
        }
    }

    /// Whether any selective action may be offered for this tool's
    /// units. The single place that question is answered.
    pub fn actions_available(self) -> bool {
        matches!(self, Self::Supported)
    }
}

/// What was checked, where, and when -- so a later implementer re-runs
/// the check instead of trusting this table blind. `checked` names the
/// specific claim; `source` is the exact upstream repo path or doc URL;
/// `revision` is a commit SHA, a branch plus a retrieval date, or a
/// retrieval date for a doc page.
#[derive(Debug, Clone, Serialize)]
pub struct Verification {
    pub checked: &'static str,
    pub source: &'static str,
    pub revision: &'static str,
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
    /// The checks behind `support`. A `Supported` row must have at least
    /// one; an `Unverified` row records what was *attempted* and did not
    /// confirm, which is the more useful half.
    pub verification: &'static [Verification],
}

/// This tool's support level, by its registry/detector id, or `None` for
/// an id with no matrix row.
pub fn support_for(tool_id: &str) -> Option<SupportLevel> {
    MATRIX
        .iter()
        .find(|e| e.id.slug() == tool_id)
        .map(|e| e.support)
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
        verification: &[Verification {
            checked: "the ~/.claude directory table (projects/, todos/, file-history/, \
                      shell-snapshots/, plugins/, settings.json, .credentials.json) and the \
                      $CLAUDE_CONFIG_DIR override",
            source: "https://code.claude.com/docs/en/claude-directory",
            revision: "retrieved 2026-09-21",
        }],
    },
    MatrixEntry {
        id: AgentToolId::Codex,
        display_name: "Codex",
        support: SupportLevel::Supported,
        home_note: "CODEX_HOME, default ~/.codex; sessions/ and archived_sessions/ (year/month/ \
                     day rollout-*.jsonl trees), auth.json, history.jsonl, config.toml, log/ \
                     (overridable by the log_dir config key), a deprecated skills/, six SQLite \
                     state stores (state_5/logs_2/goals_1/memories_1/queue_1/ \
                     thread_history_1.sqlite) relocatable via the separate CODEX_SQLITE_HOME",
        sources: &[
            "https://github.com/openai/codex/blob/main/codex-rs/utils/home-dir/src/lib.rs",
            "https://github.com/openai/codex/blob/main/codex-rs/rollout/src/lib.rs",
            "https://github.com/openai/codex/blob/main/codex-rs/rollout/src/list.rs",
            "https://github.com/openai/codex/blob/main/codex-rs/rollout/src/rollout_file_name.rs",
            "https://github.com/openai/codex/blob/main/codex-rs/rollout/src/metadata.rs",
            "https://github.com/openai/codex/blob/main/codex-rs/state/src/lib.rs",
            "https://github.com/openai/codex/blob/main/codex-rs/core/src/config/mod.rs",
            "https://github.com/openai/codex/blob/main/codex-rs/ext/skills/src/host_roots.rs",
        ],
        note: "#93; crate::agents::codex implements identification. The session-header envelope \
               nesting around `cwd` is not pinned to one shape (internal, version-varying wire \
               format); no managed-worktree creation by the CLI itself is confirmed. skills/ \
               under CODEX_HOME is upstream-deprecated in favour of ~/.agents/skills, and log/ \
               can be moved by config -- both are identified where they are, never assumed",
        verification: &[
            Verification {
                checked: "CODEX_SQLITE_HOME is a real env var (SQLITE_HOME_ENV)",
                source: "codex-rs/state/src/lib.rs",
                revision: "openai/codex main @ 30daed37ad8035f041f65a4c4615fbc590dc8552",
            },
            Verification {
                checked: "log/ defaults to codex_home.join(\"log\") and is overridable by the \
                          log_dir config key",
                source: "codex-rs/core/src/config/mod.rs",
                revision: "openai/codex main @ 30daed37ad8035f041f65a4c4615fbc590dc8552",
            },
            Verification {
                checked: "skills/ under the user config folder exists and is commented upstream \
                          as the deprecated location (current: ~/.agents/skills)",
                source: "codex-rs/ext/skills/src/host_roots.rs",
                revision: "openai/codex main @ 30daed37ad8035f041f65a4c4615fbc590dc8552",
            },
        ],
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
        verification: &[Verification {
            checked: "the macOS desktop log directory ~/Library/Logs/com.openai.codex",
            source: "codex-rs/cli/src/doctor/desktop/platform.rs",
            revision: "openai/codex main, read during chunk #93",
        }],
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
        verification: &[Verification {
            checked: "the session file layout and the PI_CODING_AGENT_DIR override, against the \
                      project's own documentation",
            source: "https://github.com/can1357/oh-my-pi/blob/main/docs/session.md",
            revision: "main, read during chunk #94; not re-fetched 2026-09-21",
        }],
    },
    MatrixEntry {
        id: AgentToolId::OpenCode,
        display_name: "OpenCode",
        support: SupportLevel::Supported,
        home_note: "data: ${XDG_DATA_HOME:-~/.local/share}/opencode (auth.json, log/, \
                     storage/{session,message,part,session_diff,project}/ or opencode.db \
                     depending on version, snapshot/<project-id>/<hash> git-backed checkpoints); \
                     config ${XDG_CONFIG_HOME:-~/.config}/opencode, also settable by \
                     OPENCODE_CONFIG_DIR (opaque external unit); cache \
                     ${XDG_CACHE_HOME:-~/.cache}/opencode (opaque external unit)",
        sources: &[
            "https://github.com/sst/opencode/blob/dev/packages/core/src/global.ts",
            "https://github.com/sst/opencode/blob/dev/packages/core/src/flag/flag.ts",
            "https://opencode.ai/docs/config/",
            "https://github.com/sst/opencode/blob/dev/packages/opencode/src/storage/storage.ts",
        ],
        note: "#95; crate::agents::opencode implements identification for both the older file- \
               tree layout and the newer SQLite-backed one, version-gated by which markers are \
               present on disk; an unrecognized layout is reported as one explicit \
               unsupported-version unit rather than guessed at either schema. This row \
               previously named an OPENCODE_DATA_DIR override 'honored defensively'; no such \
               variable exists upstream and the claim is withdrawn",
        verification: &[
            Verification {
                checked: "the data directory is $XDG_DATA_HOME/opencode via the xdg-basedir \
                          package -- there is no OPENCODE_DATA_DIR",
                source: "packages/core/src/global.ts",
                revision: "sst/opencode dev @ fe3f3a41f79ad292cc3c7c629567385a20ec5130",
            },
            Verification {
                checked: "the complete env-var registry: OPENCODE_CONFIG_DIR, OPENCODE_CONFIG, \
                          OPENCODE_CONFIG_CONTENT, OPENCODE_DB, OPENCODE_TEST_HOME, and no \
                          data-dir variable",
                source: "packages/core/src/flag/flag.ts",
                revision: "sst/opencode dev @ fe3f3a41f79ad292cc3c7c629567385a20ec5130",
            },
        ],
    },
    MatrixEntry {
        id: AgentToolId::GeminiCli,
        display_name: "Gemini CLI",
        support: SupportLevel::Supported,
        home_note: "~/.gemini, or the whole of GEMINI_CLI_HOME when set (settings.json, \
                     GEMINI.md, extensions/, trustedFolders.json, bin/, oauth_creds.json); \
                     tmp/<project-id>/ (shell_history, checkpoints/, chats/) and \
                     history/<project-id>/ (shadow Git checkpoint repos), where <project-id> is \
                     a legacy sha256 of the project root or, in current versions, a short slug \
                     registered in projects.json",
        sources: &[
            "https://github.com/google-gemini/gemini-cli/blob/main/docs/reference/configuration.md",
            "https://github.com/google-gemini/gemini-cli/blob/main/docs/cli/checkpointing.md",
            "https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/utils/paths.ts",
            "https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/config/storage.ts",
        ],
        note: "#96; crate::agents::gemini_cli implements identification. GEMINI_CLI_HOME is the \
               real override; GEMINI_DIR is a plain '.gemini' constant upstream, not an env \
               var. The per-project directory name is one-way (sha256 in the legacy form, a \
               registry slug in the current one), so tmp/history directories carry an honest \
               Unresolved linkage rather than a guess; projects.json is the upstream mapping \
               and is not read by this adapter",
        verification: &[
            Verification {
                checked: "the OAuth credential filename is oauth_creds.json (OAUTH_FILE, \
                          getOAuthCredsPath)",
                source: "packages/core/src/config/storage.ts",
                revision: "google-gemini/gemini-cli main @ \
                           d5b3e3accb26000d273abf16e0f1dd83aa5428a9",
            },
            Verification {
                checked: "homedir() honours GEMINI_CLI_HOME; GEMINI_DIR is a constant, not an \
                          env var",
                source: "packages/core/src/utils/paths.ts",
                revision: "google-gemini/gemini-cli main @ \
                           d5b3e3accb26000d273abf16e0f1dd83aa5428a9",
            },
            Verification {
                checked: "tmp/<hash> and history/<hash> are the legacy naming; current versions \
                          use short ids from a ProjectRegistry at <runtimeDir>/projects.json and \
                          migrate the hash directories across",
                source: "packages/core/src/config/storage.ts",
                revision: "google-gemini/gemini-cli main @ \
                           d5b3e3accb26000d273abf16e0f1dd83aa5428a9",
            },
        ],
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
               linkage parses only Pi's own documented offset-zero JSON header; a header it \
               cannot parse is an explicit unknown-format outcome. This row previously \
               described a fallback to Oh My Pi's 256-byte title-slot shape; that fallback is \
               removed, because one tool's format change must never silently change another \
               tool's identification. The shared byte-offset mechanics live in the neutral \
               crate::agents::pi_family, which names no tool",
        verification: &[Verification {
            checked: "the PI_CODING_AGENT_DIR override and the settings/session layout, against \
                      the project's own documentation",
            source: "packages/coding-agent/docs/settings.md",
            revision: "badlogic/pi-mono main, read during chunk #96; not re-fetched 2026-09-21",
        }],
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
               caches, and the adapter's declared project_local_units capability, called per \
               known project worktree root, for the per-repo files -- this issue's own explicit \
               'attach to the worktree artifact model' acceptance",
        verification: &[Verification {
            checked: "the per-repo history/tags-cache filenames and the home-level cache \
                      directory, in Aider's own source",
            source: "aider/args.py, aider/repomap.py, aider/models.py",
            revision: "Aider-AI/aider main, read during chunk #96; not re-fetched 2026-09-21",
        }],
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
        verification: &[Verification {
            checked: "the full config-directory listing and the COPILOT_HOME / \
                      COPILOT_CACHE_HOME overrides, in GitHub's own CLI reference",
            source: "https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-config-dir-reference",
            revision: "retrieved during chunk #97",
        }],
    },
    MatrixEntry {
        id: AgentToolId::Cursor,
        display_name: "Cursor",
        support: SupportLevel::Unverified,
        home_note: "editor-profile storage, macOS only this chunk: ~/Library/Application \
                     Support/Cursor (User/globalStorage/state.vscdb, \
                     User/workspaceStorage/<id>/{state.vscdb,workspace.json}, User/History, \
                     Cache/CachedData/CachedExtensionVSIXs/logs), plus a separate ~/.cursor/ \
                     (chats/, projects/, CLI state, not decomposed)",
        sources: &[
            "https://cursor.com/docs/troubleshooting/troubleshooting-guide",
            "https://github.com/thomas-pedersen/cursor-chat-browser",
            "https://github.com/Callum-Ward/cursaves/blob/main/docs/how-cursor-stores-chats.md",
        ],
        note: "#98; crate::agents::cursor identifies through crate::agents::vscode_family, but \
               no official Cursor documentation names any of these paths, so this row is \
               Unverified: units are identified and measured, no action is offered and project \
               linkage is reported Unresolved. The layout is inherited from VS Code and is very \
               probably right -- 'very probably' is not the bar for moving a developer's chat \
               history. Linux (~/.config/Cursor/...) is deferred to the independent Linux track \
               (#77-#89)",
        verification: &[Verification {
            checked: "searched Cursor's official documentation for the profile layout: the \
                      troubleshooting guide says data is cached locally but names no path, and \
                      contains none of 'Application Support/Cursor', 'globalStorage', \
                      'workspaceStorage' or 'state.vscdb'. Only forum.cursor.com community \
                      threads corroborate it",
            source: "https://cursor.com/docs/troubleshooting/troubleshooting-guide",
            revision: "retrieved 2026-09-21 -- NOT CONFIRMED",
        }],
    },
    MatrixEntry {
        id: AgentToolId::Windsurf,
        display_name: "Windsurf",
        support: SupportLevel::Unverified,
        home_note: "~/.codeium/windsurf (MCP/agent config, not decomposed) plus the VS-Code-fork \
                     editor profile at ~/Library/Application Support/Windsurf, macOS only this \
                     chunk. Upstream renamed the product Devin Desktop on 2026-06-02 and moved \
                     the read-write profile to ~/Library/Application Support/Devin, keeping the \
                     Windsurf directory as a legacy read-only location",
        sources: &[
            "https://docs.devin.ai/desktop/devin-desktop-faq",
            "https://registry.coder.com/modules/coder/windsurf",
        ],
        note: "#98; crate::agents::windsurf identifies through crate::agents::vscode_family. The \
               profile root and User/globalStorage are now primary-source confirmed, but \
               User/workspaceStorage and the Cache/CachedData/CachedExtensionVSIXs/logs \
               siblings this adapter also models are not named by that page, so the row stays \
               Unverified: no action is offered and project linkage is reported Unresolved. An \
               installation whose actual layout differs surfaces as an honest '(unsupported \
               layout version)' residual rather than a silent miscount",
        verification: &[Verification {
            checked: "the per-user IDE data directory table: macOS ~/Library/Application \
                      Support/Windsurf (legacy) and .../Devin (current), listing \
                      User/settings.json, User/keybindings.json, User/snippets/, globalStorage/, \
                      Workspaces/ and argv.json. 'workspaceStorage' does not appear on the page, \
                      and neither do the Cache/logs siblings -- PARTIALLY CONFIRMED. \
                      docs.windsurf.com 307-redirects here",
            source: "https://docs.devin.ai/desktop/devin-desktop-faq",
            revision: "retrieved 2026-09-21",
        }],
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
                     context_history.json, checkpoints. Cline 4.x adds a second root -- \
                     CLINE_DATA_DIR, else CLINE_DIR/data, else ~/.cline/data -- which this \
                     catalog does not model yet",
        sources: &[
            "https://github.com/cline/cline/blob/main/apps/vscode/src/core/storage/disk.ts",
            "https://github.com/cline/cline/blob/main/apps/vscode/src/core/context/context-tracking/ContextTrackerTypes.ts",
            "https://github.com/cline/cline/blob/main/apps/vscode/src/shared/HistoryItem.ts",
            "https://github.com/cline/cline/blob/main/apps/vscode/src/shared/storage/storage-context.ts",
        ],
        note: "#99; crate::agents::cline identifies through crate::agents::vscode_family, \
               decomposing every resolved host location rather than just the first (the \
               adapter's own decomposes_every_location capability). This row previously claimed \
               project linkage from a task_metadata.json 'workspace' field; that field does not \
               exist in Cline's schema and the claim is withdrawn -- tasks now carry an \
               Unresolved link whose reason names the store that does hold the answer",
        verification: &[
            Verification {
                checked: "GlobalFileNames and ensureTaskDirectoryExists -> \
                          <globalStorage>/tasks/<taskId> with api_conversation_history.json, \
                          ui_messages.json, task_metadata.json, context_history.json",
                source: "apps/vscode/src/core/storage/disk.ts",
                revision: "cline/cline main @ d4d3d9f31f309f89d0327e2b48ab6f775030595e",
            },
            Verification {
                checked: "TaskMetadata is { files_in_context, model_usage, environment_history } \
                          -- there is NO workspace field, refuting this row's previous linkage \
                          claim",
                source: "apps/vscode/src/core/context/context-tracking/ContextTrackerTypes.ts",
                revision: "cline/cline main @ d4d3d9f31f309f89d0327e2b48ab6f775030595e",
            },
            Verification {
                checked: "the working directory is HistoryItem.cwdOnTaskInitialization (optional) \
                          in the taskHistory extension state, which lives in state.vscdb or \
                          ~/.cline/data/state/taskHistory.json",
                source: "apps/vscode/src/shared/HistoryItem.ts, \
                         apps/vscode/src/shared/storage/storage-context.ts",
                revision: "cline/cline main @ d4d3d9f31f309f89d0327e2b48ab6f775030595e",
            },
        ],
    },
    MatrixEntry {
        id: AgentToolId::RooCode,
        display_name: "Roo Code",
        support: SupportLevel::Supported,
        home_note: "VS Code extension global storage, same per-host modeling as Cline: \
                     globalStorage/rooveterinaryinc.roo-cline/tasks/<task-id>/ \
                     (api_conversation_history.json, ui_messages.json, task_metadata.json, \
                     history_item.json, plus tasks/_index.json); remote/server hosting \
                     ~/.vscode-server/data/User/globalStorage/... . The roo-cline. \
                     customStoragePath setting can relocate tasks/ entirely, in which case this \
                     catalog simply does not find it",
        sources: &[
            "https://github.com/RooCodeInc/Roo-Code/blob/main/src/shared/globalFileNames.ts",
            "https://github.com/RooCodeInc/Roo-Code/blob/main/src/utils/storage.ts",
            "https://github.com/RooCodeInc/Roo-Code/blob/main/src/core/task-persistence/TaskHistoryStore.ts",
            "https://github.com/RooCodeInc/Roo-Code/blob/main/src/core/context-tracking/FileContextTrackerTypes.ts",
        ],
        note: "#99; crate::agents::roo_code identifies through crate::agents::vscode_family. \
               Per-task project linkage is now read from the confirmed \
               tasks/<id>/history_item.json 'workspace' field, correcting this row's own prior \
               task_metadata.json claim. A task directory has been reported to embed a full Git \
               checkpoint repo, so its folded byte total is not necessarily small",
        verification: &[
            Verification {
                checked: "the per-task filenames including history_item.json and _index.json, \
                          and getTaskDirectoryPath -> <basePath>/tasks/<taskId> with the \
                          roo-cline.customStoragePath override",
                source: "src/shared/globalFileNames.ts, src/utils/storage.ts",
                revision: "RooCodeInc/Roo-Code main @ b867ec9145750d0ae1ff7f02d35406e9bf2a0b16",
            },
            Verification {
                checked: "the workspace path is written to tasks/<id>/history_item.json (and \
                          indexed in tasks/_index.json), while TaskMetadata is \
                          { files_in_context } only",
                source: "src/core/task-persistence/TaskHistoryStore.ts, \
                         src/core/context-tracking/FileContextTrackerTypes.ts",
                revision: "RooCodeInc/Roo-Code main @ b867ec9145750d0ae1ff7f02d35406e9bf2a0b16",
            },
        ],
    },
    MatrixEntry {
        id: AgentToolId::Continue,
        display_name: "Continue",
        support: SupportLevel::Supported,
        home_note: "~/.continue, or CONTINUE_GLOBAL_DIR when set (config.yaml/config.json, \
                     sessions/<session-id>.json plus a sessions/sessions.json index, index/ \
                     embeddings/tag caches, dev_data/ anonymized usage events with \
                     devdata.sqlite)",
        sources: &[
            "https://github.com/continuedev/continue/blob/main/core/util/paths.ts",
            "https://github.com/continuedev/continue/blob/main/core/util/history.ts",
            "https://github.com/continuedev/continue/blob/main/core/index.d.ts",
            "https://docs.continue.dev/customize/deep-dives/configuration",
        ],
        note: "#99; crate::agents::continue_dev implements identification. This row previously \
               said no confirmed per-session workspace-linkage field was found; one exists and \
               is now read -- Session.workspaceDirectory, written into each session file and \
               mirrored into the sessions index",
        verification: &[
            Verification {
                checked: "getContinueGlobalPath (CONTINUE_GLOBAL_DIR else ~/.continue), \
                          getSessionsFolderPath, getSessionFilePath, getSessionsListPath, \
                          getIndexFolderPath and getDevDataPath",
                source: "core/util/paths.ts",
                revision: "continuedev/continue main @ 5522c6f44ca0ac3528b37244818fbfa39b5af470",
            },
            Verification {
                checked: "Session.workspaceDirectory and BaseSessionMetadata.workspaceDirectory \
                          are required fields, written and filtered on by the history module",
                source: "core/index.d.ts, core/util/history.ts",
                revision: "continuedev/continue main @ 5522c6f44ca0ac3528b37244818fbfa39b5af470",
            },
        ],
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

    /// The support level of every tool, stated one row at a time.
    ///
    /// This replaced a loop asserting `Supported` for everything, which
    /// could not fail and therefore said nothing -- and which was wrong
    /// for two rows. A level change now has to be made here, in a diff,
    /// beside the `Verification` that justifies it.
    const EXPECTED_LEVELS: &[(AgentToolId, SupportLevel)] = &[
        (AgentToolId::ClaudeCode, SupportLevel::Supported),
        (AgentToolId::Codex, SupportLevel::Supported),
        (AgentToolId::CodexDesktop, SupportLevel::Supported),
        (AgentToolId::OhMyPi, SupportLevel::Supported),
        (AgentToolId::OpenCode, SupportLevel::Supported),
        (AgentToolId::GeminiCli, SupportLevel::Supported),
        (AgentToolId::Pi, SupportLevel::Supported),
        (AgentToolId::Aider, SupportLevel::Supported),
        (AgentToolId::GithubCopilotCli, SupportLevel::Supported),
        // No official Cursor documentation names the profile layout.
        (AgentToolId::Cursor, SupportLevel::Unverified),
        // The profile root and globalStorage are confirmed; the
        // workspaceStorage and cache/log siblings this adapter also
        // models are not.
        (AgentToolId::Windsurf, SupportLevel::Unverified),
        (AgentToolId::Cline, SupportLevel::Supported),
        (AgentToolId::RooCode, SupportLevel::Supported),
        (AgentToolId::Continue, SupportLevel::Supported),
    ];

    #[test]
    fn every_tool_has_the_support_level_this_test_names() {
        assert_eq!(MATRIX.len(), EXPECTED_LEVELS.len());
        for (id, level) in EXPECTED_LEVELS {
            assert_eq!(
                entry(*id).support,
                *level,
                "{id:?}'s support level changed; if that is deliberate, change it here too and \
                 say why in its Verification"
            );
        }
        assert!(
            MATRIX.iter().any(|e| e.support == SupportLevel::Unverified),
            "the Unverified level must actually be in use, or this table is back to claiming \
             everything is supported"
        );
    }

    #[test]
    fn a_supported_row_cites_what_confirmed_it() {
        for e in MATRIX {
            if e.support != SupportLevel::Supported {
                continue;
            }
            assert!(
                !e.verification.is_empty(),
                "{:?} is Supported with nothing recorded as having confirmed it",
                e.id
            );
            for v in e.verification {
                assert!(!v.checked.trim().is_empty(), "{:?}", e.id);
                assert!(!v.source.trim().is_empty(), "{:?}", e.id);
                assert!(
                    !v.revision.trim().is_empty(),
                    "{:?} cites {} with no commit or retrieval date",
                    e.id,
                    v.source
                );
            }
        }
    }

    #[test]
    fn an_unverified_row_records_what_was_attempted() {
        for e in MATRIX {
            if e.support != SupportLevel::Unverified {
                continue;
            }
            assert!(
                !e.verification.is_empty(),
                "{:?} is Unverified without saying what was checked and did not confirm",
                e.id
            );
            assert!(
                !e.support.actions_available(),
                "{:?} is Unverified yet actions would be offered",
                e.id
            );
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
