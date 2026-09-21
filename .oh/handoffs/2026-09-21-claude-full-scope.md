# Claude execution handoff: complete Swamp's remaining agreed scope

## Request

Take ownership of implementing, reviewing, testing and shipping **all** the remaining work below in `open-horizon-labs/swamp`. Do not stop at a plan, prototype, Rust-only increment, or the first three agent adapters. Independently useful releases are encouraged; reducing the agreed completion scope is not. Use the existing issues as the acceptance source, preserve their S&T lineage, and update them with actual evidence as work lands.

The aim is to help a developer understand where agentic development consumes disk, what grew, which projects it belongs to, and what they can give up with understandable consequences. This is a developer-storage management tool, not a perfect forensic audit or a universal oracle for disuse.

## Starting state — verify before acting

- PR #114 is merged at `9367631601426e72b5fc6efa732b64c3a016e6e6`; v0.6.3 is the release for the accumulated Cargo/TUI improvements. Check the release and Homebrew status before any further shipping action. Do not reimplement or reopen this PR.
- Included: build details in the ordinary project tree; compiler-cache / compiled-test-and-example / build-script purpose groups; exact supported-group selection from profile rows; age, size and consequence previews; responsive layout; background review/deletion, progress, between-group cancellation; static audit for known blocking TUI paths; usage screenshot and explanations.
- Workspace tests passed before merge, including 54 TUI unit tests, 20 integration/frame tests and four static-guard tests; strict workspace Clippy, formatting and all 20 source audits passed. Release CI tests the optimized native build separately.
- #64–#66 have criterion-level reconciliation tables: substantial code exists; some acceptance evidence and generic/non-Rust behavior remain unfinished. Read those tables before assigning implementation. They were deliberately left open, not falsely completed.
- #72 and #100 no longer depend on every adapter being finished. #73 and #101 actions can follow each ready adapter's supported identification and safety contract. Full catalogs still gate epic closure.
- #40 and #77 are higher-level outcomes. #74, #90 and #78 are independent implementation epics. #103/#104 are standalone work, not hidden inside an epic.

Repository: https://github.com/open-horizon-labs/swamp

Local checkout: `/Users/muness1/src/open-horizon-labs/swamp`.
Previous task worktree: `/Users/muness1/src/open-horizon-labs/swamp-tui-build-decisions`.
The old `slop-livin` checkout path does not exist. Inspect `git status` and worktrees first. The main checkout has user-owned staged `.oh` planning files: **do not reset, overwrite or discard them**. Read them for context, preserve them, and reconcile/commit them deliberately if still uncommitted. Prefer a clean worktree from current `origin/main` for implementation. Do not assume the old task worktree's branch is still the implementation branch.

## Execution order and full scope

### 1. CLI + skill replacement, then remove MCP: #103 → #104

Read both issues completely. Map every MCP capability to an existing/tested CLI JSON contract; fill actual gaps. Ship a compact installable SKILL.md with lazily loaded references. Verify examples and errors against fixtures, pagination/filtering and partial-result behavior. Then remove the MCP crate, binary, dependencies, packaging, CI targets and live instructions. Do not retain a compatibility server or migration framework.

Reconcile **every open issue/session** that still demands MCP, especially #52, #60, #72, #78/#87/#88, #90/#100, so nobody recreates it. Repurpose dedicated MCP work to equivalent CLI/skill functionality or explicitly resolve it with evidence; do not lose domain requirements. Preserve historical release/changelog facts. Do not edit installed client configuration or remove a user's installed binary without separate authorization.

Keep sink-level grant/scope/recheck enforcement. A shell agent can invoke CLI approval commands, so SKILL prose or an actor label is not a secure human identity boundary. Document the real trust model instead of claiming a transport-specific guarantee.

### 2. Main priority: project-linked agent storage, epic #90 / #91–#102

Implement shared discovery/identity/history first, using existing seams. Deliver each tool through identification → project-linked views → supported cleanup without waiting for the whole catalog.

Required order:

1. Claude Code (`~/.claude` and verified overrides), #92.
2. Codex (`~/.codex` and verified overrides), #93.
3. **Oh My Pi** (`~/.omp`, confirmed by user), #94.
4. OpenCode, #95.
5. All remaining named tools: Gemini CLI, Pi and Aider (#96); GitHub Copilot CLI (#97); Cursor and Windsurf (#98); Cline, Roo Code and Continue (#99).

#91 is shared discovery/model/history, #100 presentation, #101 precise actions, #102 full validation. All are required. Split multi-tool tasks into focused children if needed, preserving scope and lineage.

Model sessions, attachments/shared blobs, checkpoints/recovery state, caches/logs, managed worktrees and protected configuration separately. Link agent unit → project/worktree and project → agent storage using real source evidence; expose unresolved, shared, moved/missing and stale relationships. A tool-home total without project linkage is insufficient. Deduplicate worktrees/blobs without inventing a single owner.

Use primary upstream source/docs and sanitized fixtures for actual versioned formats. Collect minimum identity/reference metadata; never export private prompts, transcripts, attachment contents or credentials into reports/logs/tests. Do not read real conversation content merely to construct fixtures. Unknown format support must be explicit, not silently treated as an empty cache.

Cache removal and deletion of unique conversation/checkpoint history have different consequences. Protect credentials/config/skills/automation definitions by default. Require exact human-approved scope and explicit history/resume/rewind loss warnings for unique data. Preserve active sessions, database integrity and shared references. No wholesale tool-home deletion, guessed SQLite/WAL/SHM cleanup, or blob GC from incomplete references. Native supported operations may be preferable, but must not widen the approved selection.

### 3. Coverage and decision evidence alongside agent storage: #40 / #41–#63

Do **all** these issues, not just enough roots for agent homes. Start with #41–#44 and shared seams needed by agent storage; continue through the full detector/evidence catalog and #62/#63 validation.

Required default/config semantics: built-in existing developer roots and detected stores; user additions; exclusions win; disable a detector independently of excluding its paths; `defaults=false` for explicit-only scope; explicit command roots replace inferred roots while exclusions still apply; empty scope never silently becomes cwd/home. Explain effective coverage and baseline changes.

Required catalog: `~/src`, `~/Library/Developer`, `~/Library/Caches`; mise, asdf, pyenv, uv, Conda; rbenv, RVM, ruby-install/chruby, nvm, rustup; Cargo registry/git, npm, pnpm, Gradle caches/wrapper, Maven local repository, Go module/build, Python caches; Xcode and Android SDK/build/simulator/device stores; Homebrew installs/downloads; Hugging Face models/datasets/caches; Ollama; Docker host backing storage. Honor verified overrides and leftover stores even when executables are absent. Platform-specific defaults must not leak to other builds.

Required evidence: size/growth/regrowth and accounting basis; source-qualified modification/access/reference/use facts; consumer/version/default relationships; active occupancy; recovery prerequisites; freshness/coverage/unknowns; human keep/protect intent. Modification age is useful without claiming actual use. Do not fabricate growth/tombstones when scope, permissions, classification or ownership evidence changes. Shared/external units retain identity/history independent of project attribution.

### 4. Independent parallel Linux track: #77 / #78 / #79–#89

Start platform boundaries (#79) and native CI (#87), then complete all traversal/accounting, unprivileged watching, continuity/overflow reconciliation, user scheduling, platform-native paths/discovery, Trash, occupancy and packaging issues (#80–#89). Do not stop at a successful Linux compile.

Target Linux x86_64; Ubuntu is the user's probable environment, not a machine fact already verified. Establish and document a tested distro/glibc baseline. No root requirement or assumed systemd availability; scheduling must be optional with honest alternatives. macOS keeps its optimized FSEvents/walk path. Dependencies and optimizations are target-gated; each artifact includes its platform's implementation, not every backend.

Reuse a vetted Trash library where appropriate and verify behavior/licensing. Preserve recoverability, cross-device failures, exact scope and shared sink checks. Watcher gaps/overflow must trigger honest reconciliation and history coverage, not fabricated continuity. Coordinate MCP removal so packaging never restores it.

### 5. Finish cross-ecosystem build understanding: #74 / #64–#76

Build on the implemented Cargo model, folded measurements/history, cleanup and TUI. **Node #68 first** to challenge Cargo-specific assumptions, then Gradle/Maven #67, Python/Go #69, Apple/Android #70, Docker/BuildKit #71. Resolve remaining #64–#66 acceptance gaps; complete shared presentation #72, actions #73 and independent accounting/performance/usefulness checks #75/#76.

Reusable decision aid: **age + size + removal consequences**, backed by adapter-specific identification. Separate unit identity, timestamp provenance, accounting basis, coverage, action capability and consequence. Aggregate candidate count, bytes on a common stated basis and oldest known modification time without overlapping descendants. Explain actions in terms users understand, not only internal directory names.

Do not transplant Cargo disposal assumptions to other tools. Identification must work without cleanup support. Unsupported destructive operations remain unavailable; display why. Do not mark the epic complete because Cargo or Node is done.

### 6. On-demand per-crate diagnostics: #107

After the preceding priorities, implement bounded on-demand dependency/variant inspection. Ordinary observations retain folded dependency aggregates. Materialize detail only for explicit inspection or selected action review. Do not make this diagnostic a prerequisite for the other epics or add an exhaustive persistent compiler-file index.

## Architectural and product constraints

- Reuse the existing folded-directory walk, event invalidation, report/event-bus consumers and **current + reverse-delta Parquet** history. No new SQLite store, giant JSON artifact cache, parallel database or per-file persistent inventory. Do not “optimize” by compressing the wrong model.
- Ordinary unchanged work should scale with roots/changed containers, not all files. Reuse cached folded aggregates; bounded deeper analysis belongs on demand. Benchmark both unchanged and one-group-change refresh/storage cost and compare to the baseline before accepting a new layer.
- Current enrichment and passage of time do not generate byte-history deltas. Coverage changes are not storage changes. Be explicit about allocated, logical, unique and reclaimable bytes; purpose views and directory views are alternative views of the same storage.
- **Old is enough to suggest review.** Do not gate useful advice on perfect obsolescence, supersession or last-access proof. Label modification time correctly; unknown/future times are not ancient. Explain what deletion costs; do not promise perfect safety.
- Hardlinks or uncertain freed bytes are not blanket cleanup refusals. Preserve unselected links and precise scope; show accounting uncertainty. Identity changes, active writers, missing authority, protections and genuinely unsafe operations remain real refusal reasons.
- Inspection is not authorization. Keep exact selection, rechecks, grants, durable outcomes and recovery semantics. Tests use disposable fixtures, not real user cleanup. Do not run untrusted project hooks/config/builds during observation.
- No blocking review, scans or cleanup on TUI event/render paths. Keep progress, responsive navigation where appropriate, cancellation at durable group boundaries and explicit partial outcomes. Extend the existing AST audit when adding relevant blocking APIs; it is a bounded guard, not proof of all responsiveness.
- Use available terminal space; preserve useful narrow views. Show builds by default in project context. Profile selection means supported descendants, not an invisible whole-directory fallback. A preview subset is not the selected total.
- This is a brand-new personal project: no migration/compatibility machinery requested. Preserve real user data and unrelated working-tree changes nonetheless.
- Keep `.oh` records in Git. Capture meaningful decisions, performance measurements and metis without manufacturing new process work.

## Execution discipline

Read issue bodies, local `.oh` outcomes/guardrails/sessions, `DESIGN.md`, `docs/architecture.md`, `docs/usage.md` and the relevant code before changing it. Claim work and avoid duplicate implementations. Use independent worktrees for parallel tracks; assign disjoint ownership and do not delegate recursively. You may use your available Claude agents for parallel independent work, but retain a single integration owner and review their actual diff/tests.

For each deliverable: implement → adversarial fixtures/native checks → review against user aim and constraints → PR → merge → release/install verification → evidence-backed issue updates. Keep moving through the complete scope. A partial release is progress, not permission to stop or close an incomplete epic. When a genuine external prerequisite/authority is unavailable, record the exact blocker, continue other independent work, and ask only for the missing decision; do not invent test results or bypass authorization.

Review explicitly: would Muness still need a separate agent conversation or manual project-by-project archaeology to understand what grew and decide what to remove? Would he be disappointed by hidden builds, vague “inspect” labels, excessive caveats instead of useful recommendations, slow scans, bloated state, misleading totals or a frozen TUI? Challenge those failures before calling the work done.

Useful verification entry points:

```sh
cargo fmt --all --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo run --locked -p swamp-source-audit
```

On this Mac, temporary Git-fixture commits may hang on global SSH signing. For test processes only, use `GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=commit.gpgsign GIT_CONFIG_VALUE_0=false`; do not modify global Git settings. Reuse an existing build directory with Cargo's `--target-dir` option rather than setting `CARGO_TARGET_DIR` globally: config tests intentionally inspect that environment variable. Coordinate concurrent builds before sharing a target directory.

Release through the repository workflow and verify downloads/checksums and Homebrew. Do not hardcode a release version into README/usage installation commands. Ship CLI/skill artifacts consistently after MCP removal. Do not claim a locally overwritten old-version executable is the published release.

## Completion

All issue acceptance criteria and named tool/ecosystem/platform scopes above are complete with recorded evidence; any actual human-required validation is explicitly surfaced, not fabricated. CLI/skill replacement is shipped and MCP removed; agent-storage project linkage and supported cleanup work across the full catalog; coverage/history/evidence work is complete; Linux and macOS ship tested native artifacts; non-Rust build adapters and #107 are delivered. Open epics close only when their full acceptance is satisfied. Final report lists releases, issue closures, native checks, measured costs and any genuine residual limitations.
