# Changelog

Release notes describe behavior at the named version. See the [README](README.md) and [usage reference](docs/usage.md) for current behavior. Timings below are historical observations from one developer's machine, not a benchmark suite.

## Unreleased

### Build artifacts get adapters (#64, #65, #67, #68)

What is inside a build container, for four ecosystems instead of one.

- **`node_modules`, `dist`, `.next`, `coverage`, `.turbo`, a Gradle
  `build/`, a Maven `target/` and a Maven local repository now have an
  interior.** A build container used to be one row and one number. It
  now expands into one row per role family -- outputs, tests,
  intermediates, dependencies, shared store, metadata, residual -- each
  with what removing it would cost *in that ecosystem's own words*
  ("rebuild with `next build`", "reinstall with `npm ci` -- needs
  registry access", "a test rerun with coverage enabled regenerates
  it"), a size with its accounting basis stated, and the oldest known
  **modification** time. Visible in `swamp report --view builds`,
  `--view deps`, their `--json`, and the TUI project tree.
- **Nothing here is actionable.** No build adapter implements an action;
  every unit says inspection only, and a published capability table that
  said otherwise would fail the build's own audit. Cargo's existing
  purpose groups are unchanged and remain the only build-artifact
  cleanup swamp supports.
- **Maven origin is read, not guessed.** `_remote.repositories` is
  parsed (bounded): an entry with a repository id (`lib-2.0.jar>central=`)
  means downloaded from that repository; an entry with an **empty** id
  (`app-1.0.jar>=`) is how Maven's Resolver records `mvn install`, so the
  file's presence alone proves nothing. `maven-metadata-local.xml` beside
  a version, or an artifact-level one listing it, means installed
  locally. A `*.lastUpdated` file records a resolution *attempt* and on
  its own leaves the origin unknown. Local evidence wins over remote for
  the same version, and with no usable evidence swamp says unknown origin
  and never promises a re-download. An unresolved `${property}` version
  directory is an explicit residual unit, not a missing row.
- **A package is named by its own manifest, never by its directory.**
  An unreadable or oversized `package.json` leaves the identity unknown
  rather than guessing from the folder name. No build generation is
  invented anywhere: npm, pnpm, Gradle and Maven record none, and a
  newer similarly-named output does not supersede an older one.
- **pnpm and npm store entries are charged once.** A project's
  `node_modules/.pnpm` is hardlinked into pnpm's content-addressed
  store; those entries carry shared-hardlink membership and no physical
  charge, so no view adds a project's copy to the store's own total.
- **Identification never runs your build.** No `npm`, `gradle`, `mvn` or
  `cargo` subprocess, no JavaScript config loaded, no Gradle script or
  Maven plugin evaluated -- each of those executes code from whatever
  repository is on disk. Where the answer is only available that way,
  the row is an explicit residual with the reason attached, rather than
  a silent omission.
- **An unchanged build container now costs nothing to re-report**, and a
  stale one is no longer replayed forever. Reuse is gated on the same
  trusted FSEvents coverage the agent containers use. The Cargo
  consumer previously reused when the replay's change list happened to
  be empty, which is not the same claim as "nothing changed since your
  rows were written" when the window cannot say when it opened; that
  case now re-identifies.
- **Family rows count what they claim to.** A family's count, bytes and
  oldest modification are over nonempty supported candidates only; units
  in an unrecognised layout and bytes no unit accounts for (a
  container's loose files) are one explicit "Not identified" row, and a
  family's members never double-count a nested unit. In the TUI, Node,
  Gradle and Maven containers open into these family groups (closed
  until opened, members oldest first, unknown ages last); `--view
  builds|deps --json` now carries the same summary and every unit as an
  `interior` object.
- **A marker change is never replayed away.** When a `settings.gradle`
  appears beside a Node project's `build/`, the Gradle adapter now
  identifies it on the next refresh; stored units are replayed only when
  the adapter that wrote them is the one claiming the container.
- **An unreadable directory inside a build container is incomplete
  coverage, not a smaller tree.** The walk used to skip it silently and
  mark every ancestor complete; it now records it and the container says
  "the walk could not read all of this directory".
- Gradle: `daemon/<version>` directories and `modules-2/metadata-*` are
  identified; an unrecognised `modules-2` entry is a named residual.
- Docs: `docs/build-artifacts.md` (new -- the checked capability matrix,
  per-ecosystem layouts, attribution limits and origin evidence),
  `docs/architecture.md`, `docs/usage.md`, and a new
  `skills/swamp/references/build-artifacts.md`.

### Repairs after review 2, part 4

What "unchanged" is allowed to mean.

- **A session transcript that is being appended to is now reported at
  its real size, on the pass that sees the append.** Container and
  external-unit reuse used to be keyed on the recorded directories'
  `mtime`/`ctime`. A directory stamp moves when an entry is created,
  deleted, renamed or replaced -- and not when a file inside it is
  appended to in place, which is exactly how a running agent writes its
  session. So a growing session kept its old byte total until something
  else happened in its project directory. Reuse is now permitted only
  under **trusted event coverage**: this pass's own FSEvents replay must
  cover the path, report no event at or under it, and have opened no
  later than the stored rows were written. FSEvents reports writes, so
  the append is seen.
  - A vouched-for container or external unit now costs *nothing* --
    no listing, no `stat`, no header read. Measured: an unchanged
    5,000-session home in five project directories, 11 listings and
    5,006 stats on the first pass, 5 listings and 6 stats on the second,
    none of them per container; a 20,000-file Cargo registry cache, 6
    listings and 20,004 stats on the first pass, **0 and 0** on the
    second.
  - Where there is no window there is no reuse, and the unit is
    re-measured. The window comes from the walk, so a tool home or cache
    root that lies outside every scan root is re-measured on every pass
    -- correct, and slower than the previous release. Giving each unit
    root its own event cursor is the recorded follow-up.
- **Codex, OpenCode and Pi session trees are now containers too.** Codex
  wraps each `sessions/<yyyy>/<mm>/<dd>/`, OpenCode each
  `storage/session/<project-id>/`, Pi each `sessions/<dir>/`. Each
  container has its own entry budget: Codex's was shared across
  `sessions/` and `archived_sessions/`, which made a day's contents
  depend on how many files the days before it produced. Oh My Pi
  deliberately stays off the seam, because its session bodies feed a
  home-wide shared-blob reference count that a partially replayed pass
  would report wrongly rather than as unknown.
- **The "no subprocess spawns" check no longer needs a PATH shim.**
  Every `Command::new` in the core crate records itself, so the
  disabled-detector test measures spawns through a thread-scoped counter
  instead of a process-wide `PATH` and a shared log file -- which made
  it fail under the ordinary test harness whenever a sibling test
  probed occupancy.

### Repairs after review 2, part 3

The container-level half of the incrementality work, and a support
matrix whose every `Supported` row is now backed by a citation CI
actually reads.

- **An unchanged tool home costs stamp checks per container, not stats
  per session file.** The identification cache removed the header
  *reads* from an unchanged pass; its validity key is each session
  file's own `(size, mtime, ctime, inode)`, so knowing a session was
  unchanged still cost one `stat` per session. `swamp` now records the
  directories each container's identification listed and replays the
  container from the store when none of their stamps have moved.
  Measured over a 5,000-session home in 5 project directories: 11
  listings and 5,031 stats on the first pass, **5 listings and 31 stats**
  on an unchanged second. One appended session re-identifies exactly one
  container and replays the other four.
  - Stated limit, the same one the external reuse has and it bites
    harder here: a file rewritten **in place** -- including an append to
    an open session transcript -- does not move its container's stamp,
    so the stored byte total stands until something is created, removed
    or renamed in that container. It is a reporting lag, never an
    authorization hole: every action re-derives from the live filesystem
    with both caches disabled.
- **Gemini CLI's downloaded-tools cache was at a path that does not
  exist.** It is `~/.gemini/tmp/bin`, not `~/.gemini/bin`, so that unit
  never appeared. Also new: under `SANDBOX=sandbox-exec` the runtime
  directory moves to `~/.cache/.gemini`, which is now detected when this
  process is itself under that sandbox.
- **Codex has seven SQLite state stores, not six.**
  `memories_v2_1.sqlite` was neither folded with its `-wal`/`-shm`
  sidecars nor protected. The sidecars of all seven are also no longer
  counted a second time in the unclassified residual.
- **OpenCode's session diffs were counted nowhere.**
  `storage/session_diff/<session-id>` is a `.json` **file**, not a
  companion directory; `swamp` gated on "is a directory" and swept only
  directories, so every diff's bytes were in no unit at all.
- **Cline sessions now report their project.** The working directory is
  in `state/taskHistory.json` -- a plain file inside the extension
  storage directory `swamp` already reads -- not, as this changelog
  previously implied, locked inside `state.vscdb`. Read once per editor
  host. Upstream spells that path three different ways at the same
  commit, and the unresolved reason says so rather than asserting one.
  Cline's second root (`CLINE_DATA_DIR`, else `CLINE_DIR/data`, else
  `~/.cline/data`) is now detected.
- **GitHub Copilot CLI sessions no longer claim a project.** `swamp`
  parsed `cwd`/`workspace`/`workspaceFolder` out of session files while
  the documentation promised it never guessed at an undocumented schema.
  No upstream source documents any such field and the CLI is closed
  source, so linkage is now unresolved and no selective action is
  offered on session state. The directories are still identified and
  measured.
- **Claude Code's `statsig/`, `logs/` and unmatched `todos/` entries are
  legacy, not caches.** Anthropic's own documentation lists them as "no
  longer written"; `swamp` described `statsig` as community-documented
  and auto-regenerating. Unmatched `todos/` entries are also now
  reported instead of appearing in no unit at all.
- **Continue's empty workspace directory is `unresolved`, not
  `missing`.** Upstream writes `""` itself when it cannot load a
  session, so it is an expected value and not a path that disappeared.
- **Windsurf's current profile root is modeled.** The product was
  renamed, and the read-write profile moved to
  `~/Library/Application Support/Devin`; both it and the legacy
  `.../Windsurf` are now detected, because an installation mid-migration
  has bytes in each. The row stays `unverified`: the documentation
  confirms the roots, not the per-workspace layout.
- **Every matrix citation is pinned and checked.** Each cited upstream
  file is vendored as a minimal excerpt with its blake3 digest under
  `crates/core/tests/fixtures/upstream/`, and CI greps it for the
  symbols the claim depends on. The previous check was that the
  provenance string was longer than thirty characters. A new check also
  fails when a prose paragraph asserts doubt about a tool its own table
  row lists as supported.

### Repairs after review 2

A second independent review, plus a background mutation sweep that
bypassed all 42 source audits on the first try, found that several
claims in the section below were checked by machinery that could not
see the thing it claimed to check. These are the repairs.

- **An unchanged external storage location is no longer re-measured.**
  `swamp` records one stamp per directory it folded and reuses the
  stored measurement when every stamp still matches, so an unchanged
  cache costs one `stat` per directory and no directory listing at all.
  Measured over a 20,000-file Cargo registry cache: 73 ms and 20,004
  stats on the first pass, 1.9 ms and 4 stats on the second. A file
  added anywhere under the location is a real re-measurement, and its
  bytes are reported. Stated limit: a file rewritten *in place* does not
  move its directory's stamp, so if that rewrite also changes the file's
  allocation the reused total is stale until something else in that
  directory changes.
- **The work counters now see the walker.** They were per-thread while
  the walk runs on a worker pool, so a 20,000-file traversal reported
  "2 directories listed". Every incrementality number in this changelog
  from before this release was measured with that instrument; the ones
  above are not.
- **The Maven claim is withdrawn.** `swamp` does not read a
  `_remote.repositories` marker and never did: reading one per artifact
  would mean traversing the whole local repository. A Maven local
  repository's recovery fact is `Unknown` with that limit named, which
  is what `docs/locations.md` always said and what
  `docs/architecture.md` now says too.
- **The external view has rendering tests**, including that an empty
  view says it is empty and that no verdict word ("safe", "unused",
  "stale") can reach it.
- Internal: the source-audit mutation corpus covers all 45 audits (136
  fixtures, each applied to a copy of the real workspace); the CLI's
  agent-storage tests no longer scan the machine's real disk (over
  twenty minutes to 4.7 s).

### Repairs after review

Two independent adversarial reviews blocked the #116-#123 stack with
seventeen falsifying tests. All seventeen now pass, and the repairs are
at the shared layers the reviews pointed at rather than in the adapters
where the symptoms appeared.

- **An approval now buys the bytes that were reviewed.** New
  `swamp_core::recheck` is one live-state recheck model every
  destructive sink shares: reviewed identity and membership, human
  keep/protect intent reloaded from disk, and tri-state occupancy over
  every member. A directory renamed aside and replaced with unrelated
  content, a `swamp protect` entry added after approval, and a file held
  open *inside* a cache directory each previously let an approved plan
  proceed; each now refuses and says which.
- **Occupancy is three-valued.** `OccupancyState::{Free, Occupied,
  Unknown}`, probed with `lsof +D` for directories so the answer covers
  descendants. A probe that times out or is denied permission is
  `Unknown`, and `Unknown` refuses -- the previous recheck fell through
  on exactly that case, and the confirmation showed nothing at all for
  it. Both the detail area and the delete confirmation now surface
  "could not check whether this is in use right now".
- **Protection fails closed and works in both directions.** An
  unreadable or malformed `agent_protect.json` is *unknown*, not an
  empty keep list; every action refuses until it can be read, and
  `swamp protect list` reports the same error. Protecting a file inside
  a directory now stops the directory being removed. Writes are atomic.
- **`[scan] defaults = false` means explicit-only scope**, with a new
  `enabled_detectors` allow-list. The earlier reading -- drop the
  built-in default roots, keep inferring from every other detector --
  is reverted; see the dated correction in
  `.oh/sessions/2026-09-21-scope-and-detector-registry.md`.
- **Discovery consumes the authorized scope.** External and agent
  discovery read `EffectiveScope::authorized_roots()` instead of raw
  detector candidates, so an excluded tool home yields zero units, a
  disabled detector yields zero units, and an explicit `--root` no
  longer quietly widens back out to the whole configured catalog.
- **History sweeps are owned.** `growth::ObservationOwnership` (key
  family plus completely-covered roots) guards the shared current
  table's tombstone loop, so two observations over one store can no
  longer tombstone each other's rows and report the resurrection as
  regrowth. `report::observe_scope` runs the walk and both discoveries
  as one pass.
- **Every TUI refresh preserves the scope.** Background, live-watch and
  post-action re-observation all go through the scope-aware path, so
  excluded subtrees and pruned external locations stay absent; external
  and agent units are refreshed from the same pass, so the agents view
  can no longer be stale behind a "live" header; and `prune_removed`
  drops exactly the successful agent and external rows.
- **Evidence says what it knows, and from where.** Rendered facts carry
  their subtype, so allocated bytes and estimated reclaimable bytes no
  longer render identically. Docker rows keep daemon provenance and an
  explicit unknown for host backing store instead of a filesystem
  number nobody measured. An unresolved Maven `${property}` or
  parent-inherited version is a stated identity gap rather than a
  silently dropped dependency. Consumer facts survive a per-root report
  refresh.
- **Every evidence capability is now reached by the pipeline, or
  gone.** Seventeen public functions across the evidence modules had no
  production caller while the docs described them as delivered. Access
  time is now read for every filesystem artifact row, stating the mount
  option when `noatime`/`relatime` makes the answer unavailable rather
  than omitting the question. Docker rows carry the daemon's own
  `last_used` and running-container facts, and only Docker rows do. A
  proposal for an external unit takes its manager-lock and
  simulator-booted readings then, not during identification, so an
  ordinary report still spawns no process per detected unit; it also
  names each installed toolchain version as separately reinstallable and
  states Maven's own downloaded-versus-`mvn install` ambiguity. On a
  copy-on-write volume, a unit's allocated bytes are now reported as a
  ceiling on what removal frees rather than an exact figure, and a
  sparse unit's apparent length is kept apart from the blocks actually
  charged. A plan carries `selection`, reconciling its per-unit sum
  against shared inodes so one physical file cannot count twice; an
  execution carries the `statvfs` before/after reading as a sourced
  observation, distinct from any estimate. Rendered facts are grouped by
  domain in a fixed order, and a short-lived reading past its own
  recheck window now says so at the confirmation.
  `external_associations::{parse_pom_xml, join_cache_entry}` and
  `toolchain_declarations::conflicting_tools` were deleted instead: the
  first two were strictly worse duplicates of `parse_pom_xml_with_gaps`
  and of the per-unit consumer join that `consumer_wiring` actually
  performs, and the third's capability is delivered by
  `VersionMatch::Conflicting`. The claims that named them are corrected
  in `.oh/sessions/2026-09-21-decision-evidence-follow-ups.md`.
- **The store holds Parquet tables and small control files, nothing
  else.** `external_consumers.json`,
  `toolchain_declarations_cache.json` and
  `dependency_identities_cache.json` are gone, replaced by columnar
  current-state tables keyed by identity plus a source `(size, mtime)`
  fingerprint (`swamp_core::assoc_store`). The Xcode DerivedData join
  is now cached at all: it used to spawn one `plutil` per locally-built
  project on every report, propose and TUI refresh. No migration --
  derived data is re-derived, and the removed files are ignored.
- **Agent adapters are a registry, not a match.** One tool's
  identification code is now one `agents::AgentAdapter`, registered
  once in `agents::registry::Registry::with_builtins`. This replaced a
  fourteen-arm `match tool_id` in `agents/mod.rs`, a **second**
  fourteen-arm match in `actions.rs` for the execution recheck, a
  hardcoded two-id `multi_location_tool`, and a bespoke call path for
  Aider -- four places to edit per tool, and forgetting the recheck one
  produced a tool that identified fine and then refused to re-verify at
  execution. Multi-host decomposition (Cline, Roo Code) and
  project-local units (Aider) are now declared capabilities. Pi no
  longer falls back to Oh My Pi's header shape: the shared byte-offset
  mechanics live in a neutral helper, and a format an adapter cannot
  identify is an explicit unknown-format outcome.
- **An unchanged agent home now costs zero header reads.** A new
  Parquet identification table memoises each adapter's derived
  value -- a session's declared `cwd`, a task's workspace path --
  against the source file's own `(len, mtime_ns, ctime_ns, inode)` plus
  an adapter version. Measured on 5,000 synthetic sessions: 740,000
  header bytes on the first pass, **0** on an unchanged second pass,
  and one capped read for one appended session. Two independent
  adversarial passes found the first fingerprint (`size`,
  whole-second `mtime`) could not see a same-length rewrite inside the
  same second and would serve a stale project; the fingerprint and a
  no-sleep regression test both come from that.
- **Adapters cannot reach past their own context.** Every adapter takes
  an `IdentifyCtx`: bounded single-level listings, folded byte totals,
  and content only through the capped header reader. No adapter calls
  `read_dir`, `read_to_string`, `std::env`, `actions::` or a logging
  macro, and units are built by `AgentUnitBuilder`, whose constructor
  applies protected-by-default categories that a struct literal could
  silently omit. Each adapter proves the same five things about itself
  by name.
- **Two tools lost their "Supported" badge, and three had a false claim
  withdrawn.** New `SupportLevel::Unverified` means an adapter exists
  but the layout it models is not confirmed against the tool's own
  source or documentation: units are still identified and measured, and
  **no action is offered, with project linkage reported unresolved**.
  Cursor is `Unverified` (no official documentation names any of the
  paths it models) and Windsurf is `Unverified` (its profile root and
  `globalStorage` are now officially documented -- under the product's
  new Devin Desktop name -- but `workspaceStorage` and the cache/log
  siblings are not). Corrected against upstream source: Cline and Roo
  Code both read a `task_metadata.json` `workspace` field that exists in
  neither schema, so Cline now says which store holds the answer and
  Roo Code reads the confirmed `history_item.json`; Continue's
  per-session `workspaceDirectory` does exist and is now read, so
  Continue sessions link for the first time; `OPENCODE_DATA_DIR` does
  not exist and the detector no longer looks for it; Gemini CLI's
  credential file is confirmed `oauth_creds.json` and its project-id
  directories are slugs, not only hashes. Every row cites what it was
  verified against, and a test parses the published table back.
- **Consumers match on detector capabilities, not detector ids.**
  `Detector::manager_conventions()` and `Detector::recovery_hint()`
  replaced 22 `*_DETECTOR_ID` matches in `consumer_wiring.rs`, so
  adding a detector no longer means editing a wiring table.
- **One human protection predicate, everywhere.** The integration
  owner's own mutation check found that removing one containment
  direction passed every audit and every test, because the ordinary
  filesystem proposal path went through a second `bool` predicate the
  audit was not reading. That predicate is deleted; marking an ordinary
  row that contains a protected file is now refused in the TUI at the
  moment you mark it, with the reason, rather than at execution.
- **Enforcement landed before the repairs.** 22 new AST audits, each
  with a `.oh/guardrails/<id>.md` and mutation tests proving both
  directions, plus runtime test files named explicitly in
  `scripts/check.sh`. All 22 pass as of this entry; the two that could
  not be satisfied as first written are recorded as decisions in their
  guardrail docs rather than quietly weakened.

- **Closed the decision-evidence follow-ups** (#56-#58, #60): live
  wiring of tool-version declarations and dependency-lockfile/shared-
  store associations into the report/external-unit pipeline (new
  `crates/core/src/consumer_wiring.rs`), with a per-worktree cache
  keyed by declaration/lockfile mtimes so an unchanged worktree is
  never re-parsed; Maven `pom.xml` dependency parsing via the new
  `roxmltree` dependency (MIT/Apache-2.0); Docker image/build-cache/
  volume rows now carry `recovery` facts from the same generic
  per-row pass (`recovery::docker_image_recovery`/
  `docker_build_cache_recovery`, alongside the existing
  `docker_volume_recovery`), joined and unjoined objects alike; a
  Recovery assessment's own smallest-useful follow-up check now
  survives into the attached fact's `note` instead of being dropped;
  the TUI's selected-row detail area and inline delete-confirmation
  row now render `evidence` (`render::render_evidence_lines`/
  `render::evidence_warnings`), ordered so a narrow terminal shows the
  most decision-relevant facts first; the bespoke-shaped JSON views
  (`--view kinds`/`builds`/`deps`/`unowned`/`worktrees`/`docker`) now
  carry `evidence` per row, matching the default report view. See
  [docs/architecture.md](docs/architecture.md)'s "Decision evidence
  contract" section for the remaining named gaps (uv/Conda project
  declarations, pyenv/rbenv/nvm/asdf/mise global defaults, npm/pnpm
  store opacity).

- **Added the current-state decision-evidence contract** (#53-#61):
  activity, consumer, current-use, recovery and reclaimability facts,
  each carrying a source, observation/event time and freshness/
  coverage limit -- never a bare value or a safety verdict. Populated
  from data every pass already collects (folded `mtime_max`, Docker's
  own `last_used`/container references, existing consumer
  associations, Docker joins, a unit's `bytes`/`hardlinked` flag) and
  attached to artifact rows, external units, agent-storage units and
  nested build-artifact units. New domain modules:
  `crates/core/src/evidence.rs` (the shared contract),
  `activity.rs` (#54, folded modification age and access-time
  reliability detection via `statfs`/`/proc/mounts`),
  `occupancy.rs` extensions (#55, structured lsof/Docker-container/
  manager-lock/simulator-booted current-use evidence, all bounded and
  read-only), `toolchain_declarations.rs` (#56, `.tool-versions`/
  `mise.toml`/`.python-version`/`.nvmrc`/`rust-toolchain` parsing
  matched to measured installations with manager-specific
  alias/range semantics), `external_associations.rs` (#57, Xcode
  DerivedData `WorkspacePath` and dependency-lockfile joins),
  `recovery.rs` (#58, per-unit sourced recovery paths with named
  prerequisites and a concrete follow-up check, replacing the blanket
  per-kind label), and `reclaimability.rs` (#59, logical/allocated/
  estimated-reclaimable/observed-freed accounting with selection-set
  inode deduplication). `report::attach_decision_evidence` wires
  Activity/Reclaimability/Recovery into every report through
  `bus::run_report`'s single choke point; current-use is taken fresh
  at proposal time and rechecked fresh again immediately before
  `execute` acts, so a fact that changes between propose and execute
  (something opens a unit after proposal) is always caught. `swamp
  protect` is extended from agent-storage-only to ordinary filesystem
  artifact rows. See [docs/usage.md](docs/usage.md)'s "Decision
  evidence" section and
  [docs/architecture.md](docs/architecture.md)'s "Decision evidence
  contract" section for the full picture (see the entry above for the
  gaps this originally left, since closed).

- **Added the full developer-storage detector catalog** (#45-#49):
  language version managers (mise, asdf, pyenv, uv, Conda, rbenv, RVM,
  ruby-install, nvm, and rustup extended to distinguish toolchains/
  downloads/tmp), shared dependency/build caches (Cargo home refined
  into five categorized locations, npm, pnpm, Gradle, Maven, Go, pip),
  Apple/Android developer tooling (Xcode, CoreSimulator, Android SDK),
  and package/model/VM stores (Homebrew extended with Cellar/Caskroom,
  Hugging Face, Ollama, and Docker Desktop's sparse VM backing file
  measured allocated-not-apparent, plus OrbStack). See
  [docs/locations.md](docs/locations.md) for the full table, sources,
  and documented limits (pnpm's per-volume stores, Maven's undecidable
  downloaded-vs-local split, Docker Desktop's relocatable disk image).
  Fixed alongside it: a detector-resolved location nested inside
  another kept root, or inside another detector's own base directory,
  used to be measured twice (once in that root's/location's own
  whole-directory total, again as its own separate external unit) --
  it is now pruned from the outer measurement and counted exactly once
  (`scope::EffectiveScope::external_pruned_subtrees`,
  `walk::resize_artifact_excluding`).
- **Supported multi-root reports, coverage inspection, and live
  refresh in the TUI** (#51). `swamp ui` with no explicit root now
  opens over the *whole* configured scope, not just its first present
  root: project/shared/external/agent-tool storage from every present
  root is visible together, including a root with no Git checkout in
  it at all. The header's coverage clause now reflects this pass's
  actual per-root walk outcome (`Partial`/`Missing`/`Excluded`/
  `Inaccessible`), not just pre-walk presence; the live FSEvents watch
  and cached-startup/background refresh both cover every included
  root independently, so a change under one root never erases or
  stale-marks another's rows.
- **Modeled agent-tool storage (Claude Code) and shipped a supported
  cleanup path** (#91, #92, #100, #101). `swamp report --view agents`
  identifies sessions, caches, logs, checkpoints and protected
  configuration under an agent-coding tool's home directory (Claude
  Code's `~/.claude` or `$CLAUDE_CONFIG_DIR` this release), linking
  each session to a swamp project where its transcript declares a
  `cwd` -- never a basename guess. History reuses the exact same
  current+reverse-delta growth-store key family `external.rs`'s
  detector-resolved units already use (an `"agent:"`-prefixed category
  string, never a second store). A required 13-tool matrix
  (`crates/core/src/agents/matrix.rs`) names every major coding-agent
  tool with a sourced home-path note; only Claude Code has real
  identification code this release, the rest are explicitly `Planned`.
  New `swamp protect add/list/remove` for human keep intent (survives
  refresh, independent of the growth store) and `swamp propose-agents
  --path <unit-path>` for a real, actionable plan -- cache/log
  categories move to Trash as a whole directory; an individual session
  removal moves its exact member set (transcript, subagent dir,
  file-history, todos) together, with membership re-verified fresh at
  execution -- through the same `swamp approve`/`swamp execute` every
  other plan uses. Credentials, settings, skills, commands and
  automation definitions are protected by default and have no
  supported action; neither does a database-like (SQLite/WAL/SHM)
  filename, or an active session (an `lsof`-style occupancy check on
  the transcript). Identification never reads past a session
  transcript's first line, and never puts prompt/response/attachment/
  credential content into a report, plan, or the ledger. The TUI gained
  two new minimal, read-only views: `ViewKind::External` (`'9'`, the
  minimal design chunk B2 recorded but did not implement) and
  `ViewKind::Agents` (no dedicated digit; reached by cycling with `v`),
  plus a header scope-coverage clause (`2 roots (1 missing)`) for its
  own root, scoped down from the full multi-root vision (#50) to what
  is available without walking anything. See `docs/agent-storage.md`
  for the full contract and known gaps (the other 12 named tools, TUI
  mark/confirm for agent actions, `~/.claude.json` living outside the
  modeled home directory).
- **Identified Codex, Codex's desktop app, Oh My Pi and OpenCode
  storage, and wired agent-storage actions into the TUI** (#93, #94,
  #95, #101). Four new adapters bring `swamp report --view agents` (CLI
  and TUI) to five supported tools: Codex (`CODEX_HOME`, default
  `~/.codex`) identifies live/archived rollout sessions in their
  year/month/day date trees and six SQLite state stores
  (`state_5.sqlite`, `logs_2.sqlite`, `goals_1.sqlite`,
  `memories_1.sqlite`, `queue_1.sqlite`, `thread_history_1.sqlite`),
  each folded with its `-wal`/`-shm` sidecars into one protected,
  non-actionable unit; the Codex desktop app is modeled as its own row
  covering only its confirmed macOS log directory
  (`~/Library/Logs/com.openai.codex`), never extrapolating the CLI's
  schema onto it. Oh My Pi (`~/.omp/agent`, confirmed by the user as a
  fork of `badlogic/pi-mono`) identifies sessions with a fixed 256-byte
  title-slot header, verifies content markers before treating anything
  under `~/.omp` as its own format (an explicit "unknown format" unit
  otherwise, since other tools can plausibly use that path), and tracks
  its content-addressed `blobs/` store's per-session reference counts
  with a bounded, per-session body scan -- never offering blob removal
  in this release, since complete reference coverage is not
  established. OpenCode identifies both its older file-tree layout
  (`storage/session/<project>/<session>.json`, linked via
  `storage/project/<id>.json`'s declared `worktree` field -- no
  session-body read needed at all) and its newer SQLite-backed one
  (`opencode.db`), version-gated by which markers are present on disk,
  plus its git-backed `snapshot/<project>/` checkpoint store
  (identified, linked, never actionable -- removing it loses `/undo`
  history). Two new `AgentMemberKind` variants (`Database`,
  `SessionData`) and a shared `agents::resolve_declared_path` helper
  (factored out of Claude Code's original implementation) support all
  four adapters without duplicating the project-linkage git-walk logic.
  The TUI's Agents view is now markable: `Space`/`Backspace` mark a
  supported unit and open the confirm banner with its real consequences
  (loss warnings, linked project), `Enter` executes through the
  existing background-worker path (never blocking the event/render
  thread), and a protected/unsupported row's footer names
  `propose_agents`'s own refusal reason. See `docs/agent-storage.md`
  for the full per-tool detail and remaining scope boundaries (blob
  GC, snapshot removal, TUI bulk marking, unconfirmed env var names).
- **Identified the nine remaining named agent tools and closed the
  full required-tool catalog** (#96, #97, #98, #99). `swamp report
  --view agents` now identifies Gemini CLI (`~/.gemini`, per-project-
  hash `tmp/`/`history/` state -- the hash is a confirmed one-way
  `sha256(project root)`, so linkage is an honest `unresolved` rather
  than a guessed reversal), Pi (`~/.pi/agent`, distinct from Oh My Pi
  despite sharing an override variable name, with explicit two-shape
  session-header detection), Aider (`~/.aider/caches` plus, uniquely,
  per-repo `.aider.chat.history.md`/`.aider.input.history`/
  `.aider.tags.cache.v{3,4}/` attached to each project worktree instead
  of a tool home), GitHub Copilot CLI (`~/.copilot`, correcting this
  catalog's own `history-session-state/` guess to the real
  `session-state/`/`command-history-state/`), Cursor and Windsurf
  (shared `state.vscdb`/`workspace.json` identification via a new
  `agents::vscode_family` module, Windsurf's own layout assumed rather
  than independently confirmed), and Cline/Roo Code/Continue (Cline and
  Roo Code decompose *every* editor host their extension is installed
  into -- Code, Code Insiders, Cursor, Windsurf, a remote
  `~/.vscode-server` target -- never merging genuinely separate
  storage). `discover_and_measure` gained a `project_worktrees`
  parameter (for Aider's per-repo units, sourced from each caller's
  already-loaded `Report` or, for `propose-agents --path`, a cheap
  upward `.git` walk) and multi-location decomposition for Cline/Roo
  Code, both narrow, opt-in extensions rather than a redesign. No new
  `AgentMemberKind` variant was needed. The TUI's `Shift+A` bulk
  marking now reaches the Agents view too, marking every actionable row
  and naming any protected/unsupported/active skips in the footer. See
  `docs/agent-storage.md` for full per-tool detail, sources and
  every explicit remaining unknown (Windsurf's assumed layout, the
  `task_metadata.json` `workspace` field, macOS-only coverage for the
  editor-family tools this chunk).
- **Completed #100/#101 project-linkage/action acceptance and unified
  `propose`, plus independent #102 validation** across all 14 named
  agent-tool ids. `swamp report --project <name>` (text or `--json`, no
  `--view` needed) now includes this project's own linked agent storage
  -- a collapsed "Agent storage (linked)" row per contributing tool in
  the text/TUI tree (`crate::tree::agent_rows_for_project`), and an
  `agent_storage: {units, total_bytes}` object in JSON -- previously
  visible only via `--view agents`. `swamp propose`'s `root` is now
  optional: a bare `--path` (no root) routes to the agent-storage
  proposer, then the external-unit proposer, refusing by name if
  neither matches; `--external` forces the latter, inspection-only
  route explicitly (closing a gap left at the Rust API level).
  `propose-agents` remains as a thin, deprecated alias into the same
  code. `propose_agents` now refuses an agent-storage plan whose
  selected units' own paths nest (parent/child overlap), the same
  discipline the Cargo-group check already applied to filesystem units.
  A partially-failed session removal (some members moved, then a later
  rename fails) now writes a `restore.json` recovery manifest into its
  Trash envelope and reports the envelope/moved bytes in the execute
  result, instead of only a bare error string. New independent test
  suites: `crates/core/tests/agent_refusal_matrix.rs` (every named
  refusal reason, across all 14 tool ids), `crates/core/tests/
  agent_storage_validation.rs` and `crates/tui/tests/
  agent_storage_validation.rs` (custom-root redirection, malformed
  metadata, unknown-schema-never-actionable, shared-resource reference
  states, a canary sweep across render text/JSON/plan/execute/ledger
  *and* real TUI frames, nested-accounting agreement, incremental
  growth history, and stable history after relinking a session to a
  different project). See `docs/agent-storage.md`'s new "#100/#101
  completion and #102 validation" and "Human review still needed"
  sections for exactly what changed and what remains for a human to
  check.
- **Made multi-root observation coverage-aware** (#42). `report`,
  `observe`, and `ui` with no explicit root now observe the whole
  configured scope coherently in one call, not just its first present
  root: every present root is walked, every root's own current+reverse-
  delta growth store is written (each root still keeps its own
  physical store; this is a coherent orchestration, not a merged
  store), and `report --json` gains a `scope_coverage` array naming
  each root's outcome (`complete`/`partial`/`excluded`/`missing`/
  `inaccessible`) with a reason whenever it is not simply `complete`. A
  root that loses read access (`chmod 000`, or a worktree inside an
  otherwise-readable root) is distinguished from one that is genuinely
  deleted: losing and regaining access never fabricates a deletion or a
  later regrowth, and a root dropped from scope (or newly excluded) is
  a coverage change, never a storage change. `EffectiveScope::pruned_subtrees`
  (#41, recorded but not previously consumed) is now wired into the
  walker, so an `exclude` entry inside a kept root is genuinely not
  measured. `swamp schedule --every` installed with no explicit roots
  no longer freezes a resolved root list into the LaunchAgent's argv:
  every scheduled fire re-resolves the configured scope, so a
  `config.toml` edit takes effect on the next run.
- **Modeled external/shared storage as first-class measured units**
  (#43): the Cargo registry, rustup toolchains, Homebrew, and future
  detector-resolved locations are measured independently of any
  project/worktree, with size/growth/regrowth history in the same
  current+reverse-delta growth store (a new key family, not a second
  store) and declared consumer associations (zero/one/many, counted
  once, never duplicating the unit or resetting its history). New
  `report --view external` (text and `--json`). External units are
  inspection-only: a plan can name one, but `execute` refuses every one
  of them unconditionally with "no supported selective action for
  `<category>`" -- registry/detector output never authorizes removal.
- **Removed `crates/mcp`/`swamp-mcp`.** The CLI's `--json` output is now the sole supported agent interface. `report --json` honors `--view`/`--project`/`--filter` (it previously ignored them and dumped the whole report); gains `--limit`/`--offset` with `total`/`truncated` envelope fields for bounded results; and gains two JSON-only views, `--view projects` and `--view grown`, covering the former `list_projects` and `what_grew` MCP tools. `propose --json` carries the same `state`/`next_step`/`observed_at` fields the MCP `propose` tool added. `plans --json` and the new `grant list --json` wrap their arrays with a `total` field.
- Added an installable agent skill at `skills/swamp/` (`SKILL.md` plus lazily loaded `references/*.md`): the inspect-first workflow, the full former-MCP-tool-to-CLI-command mapping and JSON schemas, the filter grammar, the propose/approve/execute/grant lifecycle, coverage/history semantics, and the real (transport-independent) authorization trust model.
- Rewrote the `human_only_authorization` source audit from "the MCP server never calls these functions" to a transport-independent check: authorization-minting functions are called only from the CLI's own approve/grant command handling or the TUI's confirmed-execution path, regardless of which binary a caller invokes.
- **Replaced the hand-rolled `config.toml` line parser with a real TOML parser** (#41). Every previously supported scalar key (`since`, `retention_days`, `large_file_min_bytes`, `observe_timeout_sec`) keeps its meaning and default; a config file that fails to parse, or whose `[scan]` table has a wrongly-typed field (e.g. `defaults = "yes"` instead of a bool), is now a visible, nonzero-exit error at every scope-resolving CLI entry point (`scope`, `report`, `observe`, `ui`, `schedule`, `config show`) rather than a silent fallback to defaults.
- **Added an effective-scope model and a new `swamp scope [--json]` command** (#41). `report`/`observe`/`ui`/`schedule` now resolve a root when none is given, instead of defaulting to the current directory: built-in default roots (`~/src`, `~/Library/Developer`, `~/Library/Caches` on macOS) plus enabled location-detector results, plus `config.toml`'s new `[scan]` table (`defaults`, `include`, `exclude`, `disabled_detectors`). Exclusions always win, including over an explicit command-line root; a detector disabled by `disabled_detectors` does not hide a path still reachable through another enabled root; a root nested inside another in-scope root is folded into its parent for measurement while the fold is retained as an inclusion reason. An effective scope that resolves to nothing at all is a visible, explicit error -- never a silent fallback to the current directory or home. `observe`/`schedule` walk every present root in the resolved scope; `report`/`ui` remain single-root for this change and use the scope's first present root (full multi-root `report`/`ui` support is #42/#50). `report`/`observe` persist the resolved scope and print a `coverage changed since last observation: ...` note on stderr when it differs from the last one -- a coverage change, never a byte-history delta or tombstone.
- **Added a source-aware location-detector registry** (`crates/core/src/locations/`, #44): a small, statically registered catalog (built-in default roots, Cargo home, rustup, Homebrew) that proposes developer-storage locations with stable IDs, storage categories, and resolution provenance (built-in convention / env var / config field / bounded read-only tool query), without ever authorizing measurement or removal. Detectors are read-only: no project scripts, shell startup files, or install commands; a tool query (e.g. `brew --prefix`) is optional, allow-listed, bounded by a timeout, and its failure is reported rather than fatal. A conventional-path proposal does not depend on the tool being installed, so a leftover cache is still found after it is removed.

## v0.6.3

- Show Cargo build details directly in project trees, grouped by cleanup consequence: compiler caches, compiled tests and examples, and build-script output. Keep the directory view available without counting it as additional storage.
- Show candidate counts, allocated sizes, modification ages and removal consequences. Use available terminal space for candidate previews; improve Unicode alignment, narrow layouts and scrolling.
- Allow selecting a cleanup category or profile to review its supported groups. Profile selection does not delete the entire profile directory or silently include unsupported outputs.
- Run cleanup review and execution in background workers. Show progress, completed/refused counts and the current path; Escape or Ctrl-C stops between groups without interrupting an in-flight move. Cancelled reviews preserve the prior selection, and failed or unattempted cleanup selections remain available for review.
- Add a repository-specific AST audit against known blocking cleanup/review calls on TUI event and rendering paths, with regression fixtures in workspace tests.
- Document build details with a real screenshot and explain age, shared-link accounting, selection scope and Trash behavior. Age suggests what to review; it does not prove disuse. Dependencies remain a folded aggregate, not a per-crate size map.

## v0.6.2

- Allow reviewed Cargo groups containing hardlinks to move to same-filesystem Trash. Unselected links remain intact; uncertain reclaimed space no longer blocks cleanup. Content, membership, identity, lock and authorization checks remain in place.
- Isolate observations, history and replay checkpoints by canonical scan root, fixing proposals after switching between a project and its parent in one store. Root aliases share a scope; each new scope starts its own baseline.
- Add `cleanup-check --offset` paging and `--within` discovery scope, candidate totals, unchecked counts and coverage limits. Show hardlink/accounting warnings with results so a small checked page cannot be mistaken for the total cleanup opportunity.

## v0.6.1

- Added `cleanup-check`: a bounded review of individual Cargo groups that produces unapproved plans or specific refusals, without widening selections or deleting anything.
- Distinguished category totals, unchecked groups and blocked outputs in Rust reports, JSON and the Builds TUI. Rust text reports show 30 rows by default (`--all` restores the complete list), with accounting guidance before the rows.
- Replaced confusing Cargo category-selection errors with instructions to choose individual groups. Added hardlink and lock reason codes, retry guidance, exact reviewed members and recovery details.
- Kept existing hardlink, freshness, lock and human-approval protections. This release improves cleanup discovery; it does not add support for removing hardlinked groups or prove that old builds are unused.

## v0.6.0

- Added Rust build drilldown for Cargo profiles, folded dependencies, test/example executables, incremental-cache and build-script groups, and final executables/libraries. Group history does not inflate project totals. Dependency sizes remain a directory aggregate, not a per-crate breakdown.
- Added reviewed selective cleanup for evidenced test/example executables and individual incremental/build-script groups. Cleanup checks contents, producer evidence, locks, hardlinks and occupancy before moving the selection to Trash with restore metadata. Shared dependencies and final outputs remain inspection-only; age is not proof that a build is unused.
- Kept ordinary compiler files folded in reports and history, with trusted unchanged-container reuse instead of a persisted per-file inventory.
- Made incremental hardlinked-artifact refreshes update directory allocations without a whole-artifact rewalk. Unique-byte totals are explicitly marked stale until a full scan reconciles them; stale unique measurements create history gaps and cannot spend standing-grant budgets.
- Compacted small reverse deltas and delayed replay-checkpoint publication until report persistence succeeds.
- Fixed Cargo cleanup lock lifetime under concurrent process creation. Added native Cargo build/cleanup/rebuild verification and parallel stress coverage.

## v0.5.2

- Narrowed Ruby dependency attribution to `vendor/bundle`, preserving unrelated vendored source.
- Added declarative project-name fallbacks for Gradle settings, Cabal, and Python `setup.cfg` manifests.
- Made strict JSON manifest names structural and top-level only.
- Added regressions for shared Rust workspace targets and independent nested project targets.

## v0.5.1

- Fixed project badges with linked worktree counts so the worktree glyph and multi-digit count remain visually separated in the terminal UI.

## v0.5.0

- Fixed persistence of existing artifact byte changes in `current.parquet`. Added regression coverage for successive updates and unchanged observations after an update.
- Renamed the repository, source packages, binaries, environment variables, and release artifacts to `swamp`. The v0.5.0 archive contains `swamp` and `swamp-mcp`.
- Reorganized the documentation into a product overview, usage reference, architecture guide, and contributor guide. Corrected outdated UI, filter, installation, and history claims.

## v0.4.0

### Docker removal

- Added image and volume removal to the CLI, MCP, and TUI through Docker. This includes objects without project attribution.
- Added object-specific recovery information to plans, confirmations, and ledger records. Filesystem paths go to Trash; Docker removals do not. Images may be pulled or rebuilt if their sources remain available; swamp makes no copy of volume contents.
- Refused individual build-cache removal because the action path does not support that unit.
- Added live object checks before removal and surfaced Docker's refusal text.
- Separated trashed bytes, permanent removals, and measured free-space change in results.
- Added a Docker fixture with attributed and unattributed images, a volume, dangling images, build cache, and a container that prevents image removal.

### Git tracking and actions

- Split the worktree remainder into tracked `source`, `ignored`, and `untracked` buckets using directory-level Git status with large-file corrections. Apportioned totals preserve the walk's measured bytes.
- Prevented deletion of the ignored/untracked aggregate buckets as single paths.
- Reused the Git exclude stack within a checkout.
- Allowed direct project actions to select its actionable artifacts. When none exist, a direct action can offer the checkout; bulk marking skips that fallback.

### Terminal UI

- Replaced row sparklines with logarithmic change bars: growth extends right in red, shrink left in green, and changes below 1 MB use a small tick.
- Sorted growth by signed value, so increases precede decreases.
- Made Right open/expand and Left collapse/return, matching the displayed navigation.
- Fixed negative growth formatting, duplicated carried-forward GitHub signals, and empty container names.

### Storage and checks

- Changed report-history Parquet writes to close a temporary file before renaming it over the destination. This protects readers from unfinished individual files; it is not a multi-file transaction.
- Corrected source checks that matched their own explanatory comments and removed their ripgrep dependency.
- Corrected the fixture's dangling-image case on Docker installations using the containerd image store.

## v0.3.0

### Live and incremental observation

- Added a live FSEvents watch while the TUI is open, with observation after 400 ms of quiet.
- Retained directory detail inside folded artifacts so updates can re-list changed interior directories.
- Added a whole-artifact fallback for hardlinked units and local byte measurements to support deduplicated incremental accounting.
- Re-listed known remainder directories without walking their entire worktree when possible.
- Built one history index per growth-annotation pass and skipped unchanged current-file writes.
- Reused and aged Git signals for untouched worktrees, cached Docker facts for five minutes, and limited discovery around changed directories.

The release recorded these observations on one `~/src` tree with 55 projects and about 44 GB. Hardware details, repeated samples, and a reproducible benchmark harness were not supplied with the table.

| Case | v0.2.0 | v0.3.0 |
|---|---|---|
| Source file touched | 7.5 s | 75 ms |
| Change inside a 16 GB `target/` | 7.5 s | 2.4 s |
| No change | 7.5 s | 86 ms |

### Artifact recognition

- Added recognition of regular `CACHEDIR.TAG` files with the required signature.
- Vendored ignore/language lists and added a harvest utility to report candidate artifact names without editing the classification table.
- Expanded marker-gated recognition across ecosystems, including ESP-IDF, Godot, Jekyll, Elm, Erlang, OCaml, Clojure, and Nim.
- Added ESP-IDF `managed_components/` and build variants, CMake build variants, and Python `requirements*` markers.

### UI and reporting

- Changed growth to red and shrink to green; used a dark selection background to preserve those colors.
- Added history charts based on observed changes and ecosystem badges after project names.
- Preserved selection during background refresh.
- Included directory rows in the startup observation.
- Corrected progress counters and limited percentage display to cases where the previous total is a usable estimate.
- Added `--version` and release-version smoke checks.

## v0.2.0

- Added project ecosystem detection, marker-gated artifact classification, type filters and sorting, a types view, and manifest-based names for repositories without remotes.
- Added size and age predicates, project globs, more sorts, and persisted UI choices.
- Added the keep-executables option for supported Rust and Python outputs.
- Added observation progress and UI refresh after removal.
- Forced a full walk after classification-rule changes.
- Added time-series display with unobserved buckets represented separately from zero changes.
- Reorganized report construction into consumers on an in-memory event bus. See [ADR 001](docs/ADRs/001-event-bus-report-pipeline.md).
- Added source audits for selected implementation constraints and a mutation script for walker checks.
- Added configuration commands and MCP report filtering.

## v0.1.0

- Introduced a terminal UI, CLI, and stdio MCP server for disk growth by project, checkout/worktree, and artifact.
- Added Parquet history with reverse deltas, FSEvents-based incremental observation, and optional scheduled observation.
- Grouped clones by remote and exposed worktree activity, Git tracking, and cached GitHub facts.
- Added Docker attribution by Compose/source evidence, with unmatched objects reported as unowned. Docker removal arrived in v0.4.0.
- Added filesystem reconciliation and an optional `du` comparison.
- Added action plans, CLI approval and standing grants, execution, and ledger records.

The initial release targeted Apple silicon macOS with unsigned binaries. History began with the first observation. Performance figures were observations from one machine.
