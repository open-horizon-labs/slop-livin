# Contributing

Use a recent stable Rust toolchain on macOS. The release workflow builds Apple silicon binaries. Start with the [architecture guide](docs/architecture.md) for the data flow and [PRODUCT.md](PRODUCT.md) for the product contract.

## Code map

| Location | Responsibility |
|---|---|
| `crates/core/src/report.rs` | Report types, entry points, aggregation helpers |
| `crates/core/src/walk.rs`, `fs_events.rs` | Traversal, event replay, live watching |
| `crates/core/src/growth.rs` | Incremental reconstruction and Parquet history |
| `crates/core/src/bus/`, `consumers/` | Observation pipeline and enrichment stages |
| `crates/core/src/ecosystem.rs` | Project markers and artifact classification |
| `crates/core/src/actions.rs`, `execution.rs` | Plans, authorization, execution, and recovery records |
| `crates/cli/`, `crates/tui/` | User and agent interfaces (the CLI's `--json` output plus `skills/swamp/` is the supported agent interface; see #104) |
| `crates/core/src/fs_gate/` | The capability gate: the only code that touches the filesystem or starts a process ([architecture](docs/architecture.md#capability-gates)) |
| `crates/source-audit/` | Exact path-reference audits, the compile-fail runner and the mutation sweep |
| `crates/harvest/`, `vendor/` | Artifact-name research against vendored upstream lists |
| `skills/swamp/` | The installable agent skill: `SKILL.md` plus lazily loaded `references/*.md` |

## Checks

```bash
scripts/check.sh
```

The script runs formatting checks, workspace tests, Clippy (all targets, then the production build on its own, where the capability gate's lints apply), source audits, the compile-fail cases and the mutation sweep, and checks for selected destructive shortcuts and output vocabulary. Plain `cargo build` does not run all these checks. `SWAMP_TARGET_DIR=<dir>` points every cargo run in it at a shared target directory.

New filesystem or process access goes through `crates/core/src/fs_gate/`; outside it, `std::fs`, `std::process` and friends do not pass Clippy or the audit. A compile-fail case's expected output is regenerated with `TRYBUILD=overwrite cargo test -p swamp-source-audit --test compile_fail` after an intended API change (the snapshots are rustc-version sensitive).

For a focused change, run the relevant tests first. Examples:

```bash
cargo test -p swamp-core --test fsevents_incremental
cargo test -p swamp-core --test report_growth
cargo test -p swamp --test agent_json_contract
cargo test -p swamp-tui --test frames
cargo run -q -p swamp-source-audit -- --list
```

When a deliberate UI change alters a committed frame:

```bash
UPDATE_FRAMES=1 cargo test -p swamp-tui --test frames
git diff -- crates/tui/tests/frames
cargo test -p swamp-tui --test frames
```

Review the generated diff before accepting it. A passing snapshot update alone cannot establish that the new layout is correct.

## Documentation

Keep the README focused on the reader's first understanding and first use. Put commands and operational details in [usage](docs/usage.md), mechanisms and tradeoffs in [architecture](docs/architecture.md), and behavior changes under the appropriate changelog version. Update PRODUCT and DESIGN when their contracts change.

Check commands against their handlers as well as `--help`: parser support does not prove a flag affects every view. Check the JSON contract against `crates/cli/tests/agent_json_contract.rs` and `skills/swamp/references/commands-and-json.md`, not against memory of what a flag used to do. Label examples as illustrative when they are not captured output. Record benchmark environment and method before publishing performance figures.

The [accuracy report](docs/accuracy.md) records the evidence behind the documentation. The archived Epic #1 notes under `docs/` describe early decisions and are not specifications for current behavior.
