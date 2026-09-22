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
| `cargo` (Rust) | implemented | container, outputs, tests, intermediates, dependencies, metadata, residual | `target/<profile>/`, `target/<triple>/<profile>/`, `deps/`, `examples/`, `incremental/`, `build/`, `.fingerprint/`, custom `target-dir`/`build-dir` from `.cargo/config[.toml]` or `CARGO_TARGET_DIR` | Cargo's intermediate layout is version-dependent; per-crate dependency sizing is not attempted (#107); a command-line `--target-dir` is invisible to an observer | a profile directory, or one target's executable plus its fingerprint | inspection only |
| `node` (Node.js) | implemented | container, outputs, tests, intermediates, dependencies, shared-store, metadata, residual | `node_modules/` top-level and `@scope/` packages, `node_modules/.pnpm`, `dist`, `build`, `out`, `.next`, `.nuxt`, `.svelte-kit`, `.output`, `.vercel/output`, `storybook-static`, `out-tsc`, `coverage`, `.nyc_output`, `playwright-report`, `test-results`, `.next/cache`, `.turbo`, `.parcel-cache`, `.cache`, `.vite`, `.angular`, `.expo`, `.metro`, `tsconfig.tsbuildinfo`, `.eslintcache`, npm `_cacache`, pnpm store | a package's identity is its own `package.json`, and an unreadable one leaves it unknown; workspace hoisting means a top-level package's dependent member is not recorded on disk; pnpm store objects are content-addressed, so which project links one is not derivable; no build generation is inferred | one output directory, one cache directory, or one installed tree | inspection only |
| `gradle` | implemented | container, outputs, tests, intermediates, shared-store, metadata, residual | `build/{classes,libs,distributions,resources,generated,intermediates,tmp,kotlin,reports,test-results}`, `.gradle/`, `<gradle-user-home>/caches/{modules-N,transforms-N,jars-N,build-cache-N}`, `<gradle-user-home>/wrapper/dists/`, `daemon/`, `native/`, `jdks/` | build scripts and plugins are never evaluated, so a reassigned `buildDir` or a plugin's own output directory is an unidentified residual; a `transforms-*`/`build-cache-*` entry is keyed by a hash whose inputs Gradle does not record; which project last wrote a shared cache entry is not recorded | one project build directory, or one cache category directory | inspection only |
| `maven` | implemented | container, outputs, tests, intermediates, shared-store, metadata, residual | `target/{classes,test-classes,generated-sources,generated-test-sources,surefire-reports,failsafe-reports,site,maven-status,maven-archiver}`, packaged `*.jar`/`*.war`/`*.ear`/`*.aar`, `<local-repository>/<group>/<artifact>/<version>/` with `_remote.repositories`, `*.lastUpdated` and `maven-metadata-local.xml` origin evidence | an artifact with no origin evidence has **unknown** origin and swamp never promises it can be downloaded again; POM properties and parent-inherited versions are not resolved; plugins are never evaluated, so a plugin's output under `target/` is an unidentified residual | one project target directory, or one repository artifact version | inspection only |
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
recoverable at all, so it is not guessed.

| Evidence beside the version directory | Origin | What swamp says |
|---|---|---|
| `_remote.repositories`, or any `*.lastUpdated` | downloaded | "the next build that needs this version downloads it again -- needs access to the repository it came from" |
| `maven-metadata-local.xml` | locally installed | "this version was installed from a local build; recreating it needs that project's source and an `mvn install`" |
| neither | unknown origin | "swamp found no origin evidence beside this artifact, so it cannot say whether it can be downloaded again" |

Where both a remote and a local marker are present, the **local** one
wins: reporting "downloaded" for an artifact that only exists because
somebody ran `mvn install` is the mistake that costs an artifact.

## Shared stores and double counting

pnpm hardlinks its content-addressed objects into each project's
`node_modules/.pnpm`; npm's `_cacache`, Gradle's `modules-N` and Maven's
local repository are each shared by every project on the machine. The
walk charges each inode once, wherever it first met it, so these bytes
are already counted exactly once at the report level. Units inside a
shared store therefore:

- carry `shared-hardlink` membership and **no** physical charge, so a
  view can never add a project's copy to the store's own total;
- state that the linking projects are not derivable from the entry;
- declare no action, because the same bytes may be in use by a project
  swamp is not looking at.

## Where this appears

- `swamp report <root> --view builds` -- the artifact rows, plus the
  collapsed per-family summary for every identified container.
- `swamp report <root> --view builds --json` -- the same, with each
  unit's `role`, `adapter`, `basis`, `time_source`, `action` and
  `consequence`.
- `swamp report <root> --view rust` -- the Cargo drill-down, unchanged.
- The TUI project tree, where a container expands into its family
  groups: recommendation and consequence first, then count, size and
  oldest modification when the width allows.

## Cost

An unchanged container costs nothing: its units are replayed under the
pass's trusted event window, with zero directory listings and zero
manifest reads. A changed container pays one capped listing per
directory the adapter needs to look inside and one bounded manifest read
per named metadata file. Both are counted
(`crate::work_counters`), and the measured numbers are in
`.oh/sessions/2026-09-21-build-adapters-node-jvm.md`.
