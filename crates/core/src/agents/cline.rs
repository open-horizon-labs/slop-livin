//! Cline identification (#99): a thin wrapper over
//! `crate::agents::vscode_family::identify_extension_globalstorage`,
//! applied to each host location
//! `crate::locations::cline::ClineDetector` resolves (#99's explicit
//! "model each host as a separate detector location" -- `identify` is
//! called once per host by `crate::agents::discover_and_measure`'s
//! multi-location iteration, never merged; the iteration is driven by
//! this adapter's own `decomposes_every_location` capability rather than
//! by a tool-id match in the shared layer).
//!
//! ## The linkage field this adapter used to read does not exist
//!
//! Verified 2026-09-21 against `cline/cline` main
//! @ `d4d3d9f31f309f89d0327e2b48ab6f775030595e`:
//!
//! * `apps/vscode/src/core/storage/disk.ts` confirms `GlobalFileNames`
//!   (`api_conversation_history.json`, `ui_messages.json`,
//!   `task_metadata.json`, `context_history.json`, `settings.json`) and
//!   `ensureTaskDirectoryExists` placing them at
//!   `<globalStorage>/tasks/<taskId>`; `apps/vscode/package.json`
//!   confirms the `saoudrizwan.claude-dev` extension id. Identification
//!   is therefore on firm ground.
//! * **But** `apps/vscode/src/core/context/context-tracking/ContextTrackerTypes.ts`
//!   defines `TaskMetadata` as `{ files_in_context, model_usage,
//!   environment_history }`. There is no `workspace` field, and never
//!   was -- so the `task_metadata.json` `workspace` read this adapter
//!   inherited resolved nothing, ever, and said so with a reason that
//!   named the wrong file.
//! * The real source is the extension's `taskHistory` global state
//!   (`apps/vscode/src/shared/storage/state-keys.ts`), whose
//!   `HistoryItem.cwdOnTaskInitialization`
//!   (`apps/vscode/src/shared/HistoryItem.ts`) is *optional*. That state
//!   lives inside `state.vscdb` -- the SQLite store this catalog refuses
//!   to open -- or, for Cline 4.x, under
//!   `~/.cline/data/state/taskHistory.json`
//!   (`apps/vscode/src/shared/storage/storage-context.ts`:
//!   `CLINE_DATA_DIR` -> `CLINE_DIR/data` -> `~/.cline/data`), a root
//!   this catalog does not model yet.
//!
//! So Cline tasks now carry an `Unresolved` link whose reason names the
//! store that holds the answer. That is a smaller claim than before and
//! a true one.

use super::{
    AdapterCapabilities, AgentAdapter, CandidateAgentUnit, IdentifyCtx,
    vscode_family::{EXTENSION_CAPABILITIES, TaskLinkSource},
};
use std::path::Path;

pub const CLINE_TOOL_ID: &str = crate::locations::cline::CLINE_DETECTOR_ID;

/// Cline records a task's working directory nowhere this catalog reads.
/// The reason is surfaced verbatim, so a user is told *which* store to
/// look in rather than "could not tell".
const TASK_LINK: TaskLinkSource = TaskLinkSource::NotInAnyFileWeRead {
    reason: "Cline records a task's working directory in its taskHistory extension state \
             (HistoryItem.cwdOnTaskInitialization, itself optional), which lives inside \
             state.vscdb or under ~/.cline/data/state/taskHistory.json -- neither is read here. \
             task_metadata.json, checked by earlier versions of this adapter, holds only \
             files_in_context/model_usage/environment_history",
};

pub struct Adapter;

impl AgentAdapter for Adapter {
    fn id(&self) -> &'static str {
        CLINE_TOOL_ID
    }
    fn name(&self) -> &'static str {
        "Cline"
    }
    fn capabilities(&self) -> AdapterCapabilities {
        EXTENSION_CAPABILITIES
    }
    fn identify(&self, home: &Path, ctx: &IdentifyCtx) -> Vec<CandidateAgentUnit> {
        identify(home, ctx)
    }
}

pub fn identify(ext_home: &Path, ctx: &IdentifyCtx) -> Vec<CandidateAgentUnit> {
    super::vscode_family::identify_extension_globalstorage(ext_home, ctx, TASK_LINK)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::{AgentActionCapability, IdentificationCache, ProjectLinkState, contract};
    use std::fs;

    fn touch(path: &Path, content: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn host(dir: &Path) -> std::path::PathBuf {
        dir.join("Library/Application Support/Code/User/globalStorage/saoudrizwan.claude-dev")
    }

    fn run(ext_home: &Path) -> Vec<CandidateAgentUnit> {
        let cache = IdentificationCache::disabled();
        identify(ext_home, &IdentifyCtx::new(1, &cache))
    }

    #[test]
    fn a_task_is_identified_with_its_host_label() {
        let dir = tempfile::tempdir().unwrap();
        let ext_home = host(dir.path());
        touch(
            &ext_home.join("tasks/t1/api_conversation_history.json"),
            b"[]",
        );
        let units = run(&ext_home);
        assert_eq!(units.len(), 1);
        assert!(units[0].relative_path.starts_with("VS Code/tasks/"));
        assert_eq!(units[0].action, AgentActionCapability::SessionRemoval);
    }

    #[test]
    fn every_host_is_decomposed_not_deduplicated() {
        assert!(
            Adapter.capabilities().decomposes_every_location,
            "Cline storage can exist in several editor hosts at once; merging them would \
             under-report"
        );
    }

    // --- the five contract tests ---------------------------------------

    #[test]
    fn unknown_format_is_explicit_not_empty() {
        let dir = tempfile::tempdir().unwrap();
        let ext_home = host(dir.path());
        touch(&ext_home.join("unrelated.json"), b"{}");
        let units = run(&ext_home);
        assert_eq!(units.len(), 1, "an unrecognized layout must still surface");
        assert!(
            units[0]
                .relative_path
                .ends_with("(unsupported layout version)"),
            "{}",
            units[0].relative_path
        );
        assert!(
            units[0]
                .note
                .as_deref()
                .is_some_and(|n| n.contains("no tasks/ directory found")),
            "{:?}",
            units[0].note
        );
    }

    #[test]
    fn canary_content_never_appears_in_output() {
        let canary = "CANARY-CLINE-DO-NOT-LEAK-2c58";
        let dir = tempfile::tempdir().unwrap();
        let ext_home = host(dir.path());
        touch(
            &ext_home.join("tasks/t1/api_conversation_history.json"),
            format!("[{{\"role\":\"user\",\"content\":\"{canary}\"}}]").as_bytes(),
        );
        touch(
            &ext_home.join("tasks/t1/ui_messages.json"),
            format!("[{{\"text\":\"{canary}\"}}]").as_bytes(),
        );
        touch(
            &ext_home.join("tasks/t1/task_metadata.json"),
            format!("{{\"files_in_context\":[\"{canary}\"]}}").as_bytes(),
        );
        contract::no_content_leak(&run(&ext_home), canary);
    }

    #[test]
    fn identification_reads_no_more_than_header_cap() {
        let dir = tempfile::tempdir().unwrap();
        let ext_home = host(dir.path());
        for i in 0..100 {
            touch(
                &ext_home.join(format!("tasks/t{i}/api_conversation_history.json")),
                &b"x".repeat(100_000),
            );
            touch(
                &ext_home.join(format!("tasks/t{i}/task_metadata.json")),
                b"{\"files_in_context\":[]}",
            );
        }
        let (units, counters) = contract::measured(|| run(&ext_home));
        assert_eq!(units.len(), 100);
        // Cline's linkage is not in any file this adapter reads, so a
        // task costs no content read at all -- which is also why this
        // number cannot creep back up unnoticed.
        assert_eq!(
            counters.header_bytes_read, 0,
            "no Cline task file is read for identification"
        );
        contract::within_header_cap(counters, 0);
    }

    #[test]
    fn protected_categories_default_protected() {
        // An extension's globalStorage directory holds tasks, not the
        // extension's credentials or settings (those are VS Code global
        // state inside state.vscdb), so there is no default-protected
        // category here.
        let dir = tempfile::tempdir().unwrap();
        let ext_home = host(dir.path());
        touch(&ext_home.join("tasks/t1/ui_messages.json"), b"[]");
        contract::protection_defaults_hold_with_no_protected_category(
            &run(&ext_home),
            CLINE_TOOL_ID,
        );
    }

    #[test]
    fn project_link_is_declared_or_unresolved_never_basename_guess() {
        let dir = tempfile::tempdir().unwrap();
        let ext_home = host(dir.path());
        let repo = dir.path().join("basename-only-repo");
        fs::create_dir_all(repo.join(".git")).unwrap();
        // A task directory named after a real checkout, and a
        // `task_metadata.json` carrying a `workspace` field pointing at
        // it: neither may produce a link, because neither is where Cline
        // actually records it.
        touch(
            &ext_home.join("tasks/basename-only-repo/task_metadata.json"),
            format!("{{\"workspace\":\"{}\"}}", repo.display()).as_bytes(),
        );
        let units = run(&ext_home);
        let ProjectLinkState::Unresolved { reason } = &units[0].project_link else {
            panic!("expected Unresolved, got {:?}", units[0].project_link);
        };
        assert!(
            reason.contains("taskHistory") && reason.contains("state.vscdb"),
            "the reason must name the store that holds the answer: {reason}"
        );
        contract::linkage_is_declared_or_explicit(&units, "basename-only-repo");
    }
}
