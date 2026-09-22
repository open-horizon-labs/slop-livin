# Build artifacts

A `target/`, a `node_modules/`, a Gradle `build/` or a Maven `target/`
is one row in a swamp report and one number. That number answers "how
big", and nothing else. This document is the reference for the layer
that answers **what is inside it, how old each part is, and what it
would cost to get that part back** (#64, #65, #66, #67, #68).

**Identification is not cleanup.** Nothing in this layer removes
anything, and no adapter declares an action. Selective cleanup for build
artifacts is #73; until it lands, every row here is inspection only, and
the table below says so for every adapter because
`build_adapter_matrix_matches_docs` checks that it does.

## What an adapter may claim

- **What a directory is**: a generated output, a test or coverage
  report, an incremental cache, an installed dependency tree, an entry
  in a store shared across projects, tool metadata, or an unidentified
  residual.
- **When it was last modified**, with the source of that timestamp
  stated (one file's `mtime`, a folded directory's rolled-up newest
  entry, or a time the tool itself recorded).
- **How many bytes**, on one stated accounting basis -- allocated,
  logical, or unique-allocated. Bases are never mixed in one total.
- **What removing it would cost**, in the ecosystem's own words:
  "rebuild with `next build`", "reinstall with `npm ci` -- needs
  registry access", "a test rerun with coverage enabled regenerates it".

## What an adapter never claims

- That anything is unused, obsolete, superseded, stale or safe to
  remove. **Modification age is not use**, and a newer similarly-named
  file does not supersede an older one.
- A **build generation**. Neither npm, nor pnpm, nor Gradle, nor Maven
  records one, so swamp does not invent one from a timestamp or a hash.
- An identity it did not read. A package is named by its own manifest,
  never by its directory's basename; a Maven artifact's coordinates come
  from the repository layout, never from a filename.
- That an artifact can be downloaded again when no evidence says so.
  Maven's `unknown-origin` is a first-class answer.

## How identification is bounded

| Bound | Rule | Guardrail |
|---|---|---|
| No second traversal | Structure comes from the folded walk's own directory rows, or a capped `locations::shallow_list` | `build-adapters-do-not-traverse` |
| No unbounded reads | Content only through `bounded_io::read_manifest`, capped at 256 KiB, counted | `build-adapters-read-bounded-manifests-only` |
| No project code | No `npm`, `gradle`, `mvn` or `cargo` is ever run, and no JavaScript config is loaded | `build-adapters-are-inspection-only` |
| No stale replay | An unchanged container is replayed only under trusted FSEvents coverage, never a directory stamp | `build-adapters-reuse-under-event-coverage` |
| No overstated unit | Units are built through `NestedUnitBuilder`, whose defaults are unsupported coverage, unknown basis, unknown time source, inspection only | `build-units-built-through-builder` |
| No silent adapter | One line in `build_adapters::Registry::with_builtins()`, one matrix row, one row here | `build-adapters-are-pluggable` |
| Same five proofs each | `unknown_layout_is_explicit_not_empty`, `identification_reads_no_more_than_manifest_cap`, `no_project_or_build_code_is_executed`, `variants_never_collapse_by_basename`, `age_is_not_obsolescence` | `build-adapter-test-contract` |

## Role families

Every adapter's roles collapse into the same families, so a view can
group them without knowing which ecosystem produced a row.

| Family | What it holds | Typical consequence of removal |
|---|---|---|
| `outputs` | What a build produced | a rebuild |
| `tests` | Test and coverage output | a test rerun |
| `intermediates` | Caches kept to make the next build faster | a slower build, same result |
| `dependencies` | Installed dependency trees inside a project | a reinstall, which needs registry access |
| `shared-store` | Entries in a store shared across projects | a re-download, and other projects may link to the same bytes |
| `metadata` | What a tool recorded about a build | the tool rewrites it |
| `residual` | Present, measured, not identified | unknown |
| `unknown` | Not measured | unknown |

## Support matrix

| Adapter | Status | Families | Known layouts | Attribution limits | Operation granularity | Actions |
|---|---|---|---|---|---|---|
| `cargo` (Rust) | implemented | container, outputs, tests, intermediates, dependencies, metadata, residual | `target/<profile>/`, `target/<triple>/<profile>/`, `deps/`, `examples/`, `incremental/`, `build/`, `.fingerprint/`, custom `target-dir`/`build-dir` from `.cargo/config[.toml]` or `CARGO_TARGET_DIR` | Cargo's intermediate layout is version-dependent; per-crate dependency sizing is not attempted (#107); a command-line `--target-dir` is invisible to an observer | profile directory, or one target's executable plus its fingerprint | inspection only |
| `node` (Node.js) | implemented | outputs, tests, intermediates, dependencies, shared-store, metadata, residual | `node_modules/` top-level and `@scope/` packages, `node_modules/.pnpm`, `node_modules/.cache`, `dist`, `build`, `out`, `.next`, `.nuxt`, `.svelte-kit`, `.output`, `.vercel/output`, `storybook-static`, `out-tsc`, `coverage`, `.nyc_output`, `playwright-report`, `test-results`, `.next/cache`, `.turbo`, `.parcel-cache`, `.cache`, `.vite`, `.angular`, `.expo`, `.metro`, `tsconfig.tsbuildinfo`, `.eslintcache`, npm `_cacache`, pnpm store | a package's identity is its own `package.json` (the 200 largest top-level packages get a bounded read; the rest say so), and an unreadable one leaves it unknown; workspace hoisting means a top-level package's dependent member is not recorded on disk; pnpm store objects are content-addressed, so which project links one is not derivable; no build generation is inferred | one output directory, one cache directory, or one installed tree | inspection only |
| `gradle` | implemented | outputs, tests, intermediates, shared-store, metadata, residual | `build/{classes,libs,distributions,resources,generated,intermediates,tmp,kotlin,reports,test-results}`, `.gradle/<version>/`, `<gradle-user-home>/caches/{modules-N,transforms-N,jars-N,build-cache-N}`, `modules-2/files-2.1/<group>/<artifact>`, `modules-2/metadata-*`, `<gradle-user-home>/wrapper/dists/`, `daemon/<version>/`, `native/`, `jdks/` | build scripts and plugins are never evaluated, so a reassigned `buildDir` or a plugin's own output directory is an unidentified residual; a `transforms-*`/`build-cache-*` entry is keyed by a hash whose inputs Gradle does not record; which project last wrote a shared cache entry is not recorded | one project build directory, or one cache category directory | inspection only |
| `maven` | implemented | outputs, tests, shared-store, metadata, residual | `target/{classes,test-classes,generated-sources,generated-test-sources,surefire-reports,failsafe-reports,site,maven-status,maven-archiver,test-run-info}`, packaged `*.jar`/`*.war`/`*.ear`/`*.zip`/`*.aar`, `<local-repository>/<group>/<artifact>/<version>/` with `_remote.repositories`, `maven-metadata-local.xml` and `*.lastUpdated` origin evidence | origin is read from `_remote.repositories` entries and `maven-metadata-local.xml`; with neither (or only a `*.lastUpdated` attempt record) it is **unknown** and swamp never promises a re-download; POM properties and parent-inherited versions are not resolved, and an unresolved `${property}` version directory is an explicit residual; plugins are never evaluated, so a plugin's output under `target/` is an unidentified residual | one project target directory, or one repository artifact version | inspection only |
| `python` | planned | -- | container-level rows only: `.venv`, `__pycache__`, `build`, `dist`, `*.egg-info` | no interior identification | whole artifact row | inspection only |
| `go` | planned | -- | container-level rows only: `vendor`, `bin`, `GOCACHE`, `GOMODCACHE` | no interior identification | whole artifact row | inspection only |
| `xcode-swift` | planned | -- | container-level rows only: `DerivedData`, `.build`, `Archives`, iOS DeviceSupport | no interior identification | whole artifact row | inspection only |
| `android` | planned | -- | container-level rows only: `.cxx`, `.externalNativeBuild`, `captures`, `~/.android/avd` | no interior identification | whole artifact row | inspection only |
| `docker-buildkit` | planned | -- | daemon-reported objects only (`crate::docker`); never filesystem provenance | Docker byte accounting comes from the daemon and is not a filesystem measurement | daemon object | inspection only |

A **planned** row means the family's containers are discovered and
measured as whole artifact rows, and nothing identifies their interior.
That is a real gap, printed rather than omitted.

## Origin evidence: Maven's three answers

This is the one claim that decides whether removing something is
recoverable at all, so it is not guessed. The evidence is what Maven's
Resolver (the enhanced local repository manager) writes beside an
artifact:

| Evidence beside the version directory | Origin | What swamp says |
|---|---|---|
| `_remote.repositories` with a repository id (`lib-2.0.jar>central=`) | downloaded | "the next build that needs this version downloads it again from central -- needs access to that repository" |
| `_remote.repositories` with an **empty** id (`app-1.0.jar>=`), or `maven-metadata-local.xml` beside the version, or the artifact's `maven-metadata-local.xml` listing this version | locally installed | "this version was installed from a local build; recreating it needs that project's source and an `mvn install`" |
| only a `*.lastUpdated` file | unknown origin | the same as below, plus "a remote resolution attempt was recorded; that is not evidence the bytes here were downloaded" |
| nothing usable (absent, unparseable, or larger than the 256 KiB manifest cap) | unknown origin | "swamp found no origin evidence beside this artifact, so it cannot say whether it can be downloaded again" |

The presence of `_remote.repositories` alone proves nothing: the
Resolver writes it for `mvn install` too, with an empty repository id
(its `LOCAL_REPO_ID`). Where local and remote evidence are both present
for a version, **local** wins: reporting "downloaded" for an artifact
that only exists because somebody ran `mvn install` is the mistake that
costs an artifact. Cost: one capped listing per version directory, one
bounded read of `_remote.repositories` when present, and only when it is
absent one `stat` and at most one bounded read of the artifact-level
`maven-metadata-local.xml`.

## Shared stores and double counting

pnpm hardlinks its content-addressed objects into each project's
`node_modules/.pnpm`; npm's `_cacache`, Gradle's `modules-N` and Maven's
local repository are each shared by every project on the machine. The
walk charges each inode once, wherever it first met it, so these bytes
are already counted exactly once at the report level. Units inside a
shared store therefore:

- carry `shared-hardlink` (pnpm) or `unknown` membership and **no**
  physical charge, so a view can never add a project's copy to the
  store's own total;
- state that the linking projects are not derivable from the entry;
- declare no action, because the same bytes may be in use by a project
  swamp is not looking at.

**What is and is not joined into a live report today.** Project-local
containers (`node_modules`, `node_modules/.pnpm`, `dist`, a Gradle
`build/`, a Maven `target/`, ...) are identified on every report.
The machine-wide stores -- an npm `_cacache`, a pnpm store, a Gradle
user home, a Maven local repository -- are identified by the same
adapters (`BuildContainer::shared_store`, tested in each adapter and in
`build_adapter_cost`), but the build consumer does not yet receive them:
they are detector-resolved external locations whose measurement and
history window `external::discover_and_measure` owns, and joining them
from the build consumer would be a second observation of the same bytes
in the same pass. Until that ownership seam lands (#47/#57), those
stores appear as whole external units in `--view external`, not as
per-entry rows.

## Collapsed family rows

A container's interior is summarized one row per family, and the rules
are the same in `--view builds`, its `--json` and the TUI:

- **Only nonempty supported candidates are counted.** A unit whose
  layout the adapter does not understand is reported once, as "Not
  identified"; an empty unit is counted as empty. Neither inflates a
  family.
- **Descendants are not counted twice.** The summary stops at the
  outermost unit: a `dist/` and the `dist/assets/` inside it are one
  candidate.
- **Bases are never mixed.** A family holding allocated and logical
  numbers reports no total rather than a wrong one.
- **Leaves reconcile to the container.** Whatever the outermost units do
  not account for -- the container's own loose files, entries no unit
  claims -- is reported as unaccounted bytes on the "Not identified" row.
  Members adding up to more than their container is reported as "not
  reconciled", never as a residual of zero.
- **The oldest known modification is not a last use**, and unknown ages
  are counted separately and sorted last, never ranked as ancient.
- Each row leads with review guidance derived from the family alone
  ("Start here: slower next build", "Review: reinstall from registry",
  "Shared: other projects may link", ...), then the largest member's
  consequence in the adapter's own words, and says how many other
  consequences the family holds.

## Incremental refresh

An unchanged container's units are replayed only when **both** hold:
the pass's trusted event window covers the container and reports
nothing under it, and the stored units were written by the adapter that
claims the container *this* pass. The second condition exists because
the claim is decided by marker files beside the container
(`settings.gradle` appearing next to a Node project's `build/`), outside
the event window: without it a Gradle build directory would replay the
Node adapter's rows until something inside it changed.

A directory the walk could not list inside a container is recorded as
an incomplete, zero-byte row, and completeness rolls up to every
ancestor: a `node_modules` with one unreadable package reports
"the walk could not read all of this directory", never a complete,
smaller tree.

## Where this appears

- `swamp report <root> --view builds` (and `--view deps` for installed
  dependency trees) -- the artifact rows, plus the collapsed per-family
  summary for every identified container.
- `swamp report <root> --view builds --json` / `--view deps --json` --
  each identified row carries an `interior` object: the same family
  rows (`recommendation`, `consequence`, `count`, `bytes`, `basis`,
  `oldest_modified`, `unknown_age`, `action`), the unsupported and
  unaccounted residual, and every unit with its `role`, `adapter`,
  `basis`, `time_source`, `action`, `consequence` and `coverage`.
- `swamp report <root> --view rust` -- the Cargo drill-down, unchanged.
- The TUI project tree, where a Node/Gradle/Maven container expands into
  its family groups (closed until opened), each leading with guidance,
  then count and oldest modification; an opened group lists its members
  oldest first, each leading with its consequence. Cargo containers keep
  their purpose groups. Which presentation a container gets follows the
  roles its units carry, never an adapter id. Every row here is
  inspection only; Space on one is refused.

## Cost

An unchanged container costs nothing: its units are replayed under the
pass's trusted event window, with zero directory listings and zero
manifest reads. A changed container pays one capped listing per
directory the adapter needs to look inside and one bounded manifest read
per named metadata file. Both are counted (`crate::work_counters`).
Through the real pipeline, on a 301-package Node checkout
(`build_adapter_history::cost_report_real_pipeline_unchanged_and_one_group_change`):
an unchanged refresh re-identifies no container and reads zero manifest
bytes; a change inside `dist/` re-identifies `dist/` alone and reads no
`package.json`. The measured numbers are in
`.oh/sessions/2026-09-21-build-adapters-node-jvm.md`.
