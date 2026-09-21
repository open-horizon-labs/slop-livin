//! The explicit major-tool matrix #90/#91 require: every named tool is
//! listed here, always -- never a silent omission and never an empty
//! placeholder adapter that claims support it does not have. Each row
//! states its home-path evidence and primary source(s), researched
//! during implementation (never guessed, never learned from a real
//! `~/.claude`-style directory on this machine).
//!
//! Only [`AgentToolId::ClaudeCode`] is [`SupportLevel::Supported`] in this
//! chunk (#92): [`crate::agents::claude_code`] implements real
//! identification for it. Every other row is
//! [`SupportLevel::Planned`] -- its home-path note is real, sourced
//! research (not a guess), but no identification code exists yet, and no
//! adapter here claims otherwise. `docs/agent-storage.md` renders this
//! same table as a doc; keep the two in sync by hand (this module is the
//! single source of truth, generated into the doc, not the reverse).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentToolId {
    ClaudeCode,
    Codex,
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
        support: SupportLevel::Planned,
        home_note: "CODEX_HOME, default ~/.codex; contains sessions/, auth.json, \
                     history.jsonl, logs/, config.toml",
        sources: &[
            "https://github.com/openai/codex",
            "https://developers.openai.com/codex/config-advanced",
            "https://developers.openai.com/codex/cli/reference",
        ],
        note: "#93's job; not implemented in this chunk",
    },
    MatrixEntry {
        id: AgentToolId::OhMyPi,
        display_name: "Oh My Pi",
        support: SupportLevel::Planned,
        home_note: "~/.omp (user-confirmed identity: Oh My Pi, a fork of badlogic/pi-mono)",
        sources: &[
            "https://github.com/can1357/oh-my-pi/blob/main/docs/session.md",
            "https://github.com/can1357/oh-my-pi/blob/main/docs/settings.md",
        ],
        note: "sources recorded per #90's own required-sources list; not independently fetched \
               or implemented in this chunk (#94's job)",
    },
    MatrixEntry {
        id: AgentToolId::OpenCode,
        display_name: "OpenCode",
        support: SupportLevel::Planned,
        home_note: "data: OPENCODE_DATA_DIR, else ${XDG_DATA_HOME:-~/.local/share}/opencode \
                     (message/<session>/, session/<projectHash>/<session>.json); config: \
                     ${XDG_CONFIG_HOME:-~/.config}/opencode/opencode.json",
        sources: &[
            "https://opencode.ai/docs/troubleshooting/",
            "https://github.com/anomalyco/opencode/issues/6669",
            "https://github.com/anomalyco/opencode/issues/18633",
        ],
        note: "#95's job; not implemented in this chunk. Upstream's own issue tracker notes \
               XDG_STATE_HOME vs XDG_DATA_HOME inconsistency -- verify against the installed \
               version, not this note, at implementation time",
    },
    MatrixEntry {
        id: AgentToolId::GeminiCli,
        display_name: "Gemini CLI",
        support: SupportLevel::Planned,
        home_note: "~/.gemini (settings.json, GEMINI.md, extensions/, tmp/, chats/); root \
                     overridable via GEMINI_CONFIG_HOME",
        sources: &[
            "https://github.com/google-gemini/gemini-cli/blob/main/docs/reference/configuration.md",
        ],
        note: "#96's job; not implemented in this chunk",
    },
    MatrixEntry {
        id: AgentToolId::Pi,
        display_name: "Pi",
        support: SupportLevel::Planned,
        home_note: "~/.pi/agent/ by default, overridable via PI_AGENT_DIR (badlogic/pi-mono, \
                     also published as earendil-works/pi)",
        sources: &[
            "https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/settings.md",
            "https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/development.md",
        ],
        note: "#96's job; not implemented in this chunk; distinct tool from Oh My Pi, which is \
               a fork of it -- do not conflate the two identities",
    },
    MatrixEntry {
        id: AgentToolId::Aider,
        display_name: "Aider",
        support: SupportLevel::Planned,
        home_note: "~/.aider (~/.aider/caches for model-price/context-window/version-check \
                     caches); per-repo .aider.chat.history.md / .aider.input.history at the \
                     repo root, and .aider.tags.cache.v<N>/ at the git root -- these two are \
                     project-local, not under the home directory",
        sources: &[
            "https://aider.chat/docs/config.html",
            "https://aider.chat/docs/usage/caching.html",
            "https://github.com/Aider-AI/aider",
        ],
        note: "#96's job; not implemented in this chunk. Aider's storage is split between the \
               home directory and each project checkout -- a materially different shape from \
               the other tools in this matrix, worth flagging for whoever implements #96",
    },
    MatrixEntry {
        id: AgentToolId::GithubCopilotCli,
        display_name: "GitHub Copilot CLI",
        support: SupportLevel::Planned,
        home_note: "~/.copilot, overridable via COPILOT_HOME (mcp-config.json, config.json, \
                     OAuth tokens, session/history data)",
        sources: &[
            "https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-config-dir-reference",
        ],
        note: "#97's job; not implemented in this chunk",
    },
    MatrixEntry {
        id: AgentToolId::Cursor,
        display_name: "Cursor",
        support: SupportLevel::Planned,
        home_note: "editor-profile storage, not one fixed directory: macOS ~/Library/Application \
                     Support/Cursor/User/{globalStorage,workspaceStorage}, Linux \
                     ~/.config/Cursor/User/..., plus a separate ~/.cursor/ (chats/, projects/, \
                     CLI state)",
        sources: &[
            "https://github.com/thomas-pedersen/cursor-chat-browser",
            "https://github.com/Callum-Ward/cursaves/blob/main/docs/how-cursor-stores-chats.md",
        ],
        note: "#98's job; not implemented in this chunk. No single official Cursor documentation \
               page describing this layout was found -- these are community-reverse-engineered \
               sources, lower confidence than the other rows, and must be re-verified against \
               an installed Cursor version before implementation",
    },
    MatrixEntry {
        id: AgentToolId::Windsurf,
        display_name: "Windsurf",
        support: SupportLevel::Planned,
        home_note: "~/.codeium/windsurf (MCP config, agent config); editor chat/workspace \
                     storage location unconfirmed -- Windsurf is a VS Code fork and likely \
                     follows a similar Application Support/Windsurf/User/... layout, not \
                     verified against primary documentation this chunk",
        sources: &[
            "https://docs.windsurf.com/",
            "https://registry.coder.com/modules/coder/windsurf",
        ],
        note: "#98's job; not implemented in this chunk. Lower confidence than Claude Code/ \
               Codex/Gemini CLI rows -- home_note's second clause is a documented unknown, not \
               a researched fact",
    },
    MatrixEntry {
        id: AgentToolId::Cline,
        display_name: "Cline",
        support: SupportLevel::Planned,
        home_note: "VS Code extension global storage: macOS ~/Library/Application Support/Code/ \
                     User/globalStorage/saoudrizwan.claude-dev/, Linux ~/.config/Code/User/ \
                     globalStorage/saoudrizwan.claude-dev/, with tasks/<task-id>/ holding \
                     api_conversation_history.json, ui_messages.json, task_metadata.json",
        sources: &[
            "https://github.com/cline/cline/issues/7101",
            "https://github.com/cline/cline/issues/14135",
        ],
        note: "#99's job; not implemented in this chunk. No single official layout-reference doc \
               found (community-documented via GitHub issue threads); also varies by which VS \
               Code-family editor hosts the extension, not just by Cline's own version",
    },
    MatrixEntry {
        id: AgentToolId::RooCode,
        display_name: "Roo Code",
        support: SupportLevel::Planned,
        home_note: "VS Code extension global storage: globalStorage/rooveterinaryinc.roo-cline/ \
                     tasks/<task-id>/ (remote/server hosting: ~/.vscode-server/data/User/ \
                     globalStorage/... instead of the local User/ path)",
        sources: &["https://github.com/RooCodeInc/Roo-Code/issues/4174"],
        note: "#99's job; not implemented in this chunk. Community-documented (GitHub issue), \
               not an official layout-reference page; each task directory has been reported to \
               embed a full git checkpoint repo, which this matrix note flags for whoever \
               implements #99 as a reason the storage is not necessarily small per session",
    },
    MatrixEntry {
        id: AgentToolId::Continue,
        display_name: "Continue",
        support: SupportLevel::Planned,
        home_note: "~/.continue (config.yaml/config.json, sessions/<session-id> plus a session \
                     index file, separate from the session bodies)",
        sources: &[
            "https://docs.continue.dev/customize/deep-dives/configuration",
            "https://docs.continue.dev/reference",
        ],
        note: "#99's job; not implemented in this chunk",
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
    fn only_claude_code_is_supported_this_chunk() {
        for e in MATRIX {
            let expected = if e.id == AgentToolId::ClaudeCode {
                SupportLevel::Supported
            } else {
                SupportLevel::Planned
            };
            assert_eq!(e.support, expected, "{:?}", e.id);
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
