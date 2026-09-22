---
id: build-stores-join-by-capability
severity: hard
statement: "A machine-wide store (npm/pnpm, a Gradle home, a Maven repository, Go and Python caches, DerivedData, CoreSimulator, the Android SDK, a BuildKit daemon) reaches a build adapter only through two declared capabilities -- the detector's `build_stores()` and the adapter's `store_kinds()` -- never through a detector id, an adapter id or a path shape; and its interior's history is owned by the one observation that measured the store."
outcome: disk-growth-by-project
audit: build_stores_join_by_capability
---

## Rationale

Until 2026-09-22 every adapter could identify a machine-wide store and
none of it reached a user's report: `consumers/cargo.rs::shared_containers`
returned an empty list, deliberately, because the stores are
detector-resolved external locations that `external::discover_and_measure`
measures and owns the history of, and joining them from the per-root build
consumer would have been a second observation of the same bytes. The
previous chunk named the repair: have the external observation hand the
folded rows it already produces to the matching adapter, matched by a
detector *capability*, never a detector id.

The tempting shortcut is a table: `"npm" => node`, `"maven" => maven`, or
a path test (`path.ends_with("caches")`). Both are the central dispatch
section 13 removed from the agent side, and the path test is also wrong:
`GOMODCACHE`, `GRADLE_USER_HOME`, `maven.repo.local` and a custom
DerivedData location are free-form overrides with no fixed suffix. The
detector already knows which of its locations is which store, because it
derived them; the adapter already knows which layouts it identifies. The
join asks both and decides nothing itself.

## Detection

`crates/source-audit/src/build_audits.rs::build_stores_join_by_capability`,
over derived sets:

- **Join sites**: every production function in `crates/{core,cli,tui}/src`
  that calls a `BuildContainer` constructor whose body sets `shared: true`
  (the constructors are read from `build_adapters/mod.rs`, so a new one is
  governed the day it is written), other than those constructors.
- **Detector ids**: the literal (or the literal of the `const`) each
  `impl Detector`'s `fn id` returns, across `crates/core/src/locations/`.

For each join site, through the whole-workspace call graph:

1. nothing it reaches names a `*_DETECTOR_ID` path (resolved, so a
   `use .. as` rename is still the constant), a detector-id literal, or a
   `const` holding one;
2. it reaches an honoured `store_kinds()` call **and** an honoured
   `build_stores()` call -- `let _ = adapter.store_kinds();` followed by
   a path heuristic is no capability match;
3. it and its same-file helpers compare no string literal (`== "npm"`,
   `.ends_with("repository")`, a `"gradle" =>` arm).

No join site at all is the first failure, not a vacuous pass: the rule
landed before the join, failing, with the enumeration in
`.oh/sessions/2026-09-21-build-adapters-python-go-apple-android-docker.md`.

**Limits.** The graph is lexical (trait-object dispatch, function pointers
and closures stored in structs are not followed; methods resolve to the
impls of types the caller names). A literal comparison two files away
from the join site is caught only if it also names a detector id. History
ownership is not structural here: it is proved at runtime (below), because
"only regions this pass covered" is a property of the data, not the
syntax.

## Runtime tests that complete it

- `crates/core/tests/build_store_join.rs` -- through `report::observe_scope`:
  every declared store kind is joined to exactly one adapter; a custom
  store root with no conventional suffix is identified; store interiors
  get history on their own key family, swept only inside stores this pass
  covered, in both call orders with agent discovery, and a disabled,
  excluded or unreadable store fabricates no disappearance and no
  regrowth; an unchanged store replays with zero listings and zero
  manifest bytes.
- `crates/core/src/build_stores.rs` unit tests (kind <-> adapter matrix).
