//! The build-artifact adapter guardrails (`GUARDRAILS_SPEC.md` section
//! 18): the mirror of the agent-adapter set in `repair_audits.rs`, for
//! `crates/core/src/build_adapters/`.
//!
//! Why a mirror and not a shared rule set: the two families forbid
//! nearly the same shapes for different reasons, and the *reasons* are
//! what the failure messages have to say. An agent adapter must not read
//! a whole file because a transcript is private; a build adapter must
//! not read a whole file because a `node_modules` tree has half a
//! million of them and a manifest read is the per-project cost. An agent
//! adapter must not spawn a process because it would be running someone
//! else's tool over their sessions; a build adapter must not spawn one
//! because `npm`, `gradle` and `cargo` all *execute project code* when
//! asked a question, and identification is an observation, never a
//! build.
//!
//! Every rule here resolves references through `crate::resolve` (section
//! 17), so `use std::fs::read_dir as list` is reported as
//! `std::fs::read_dir`, and every rule has at least three rejection
//! fixtures in the mutation corpus including one alias/rename.

use crate::ast::{self, SourceFile};
use std::collections::BTreeSet;
use std::path::Path;

/// Modules under `crates/core/src/build_adapters/` that are *not*
/// adapters: the shared model, the capability matrix, the static
/// registry, the one bounded reader, and the neutral cross-adapter
/// helper the spec allow-lists by name.
///
/// `jvm_common.rs` is the allow-listed neutral helper: Gradle and Maven
/// genuinely share coordinate parsing (`group/artifact/version` from a
/// repository path), and the alternative to a neutral module is Maven's
/// adapter calling Gradle's -- exactly the Pi/Oh-My-Pi coupling section
/// 13 had to remove.
const BUILD_NON_ADAPTERS: &[&str] = &[
    "mod.rs",
    "matrix.rs",
    "registry.rs",
    "bounded_io.rs",
    "jvm_common.rs",
];

pub(crate) fn build_adapter_files(root: &Path) -> Vec<String> {
    ast::rust_files_under(root, "crates/core/src/build_adapters")
        .into_iter()
        .filter(|rel| {
            let name = rel.rsplit('/').next().unwrap_or(rel);
            !BUILD_NON_ADAPTERS.contains(&name)
        })
        .collect()
}

fn module_name(rel: &str) -> String {
    rel.rsplit('/')
        .next()
        .unwrap_or(rel)
        .trim_end_matches(".rs")
        .to_string()
}

fn parse(root: &Path, rel: &str) -> Result<SourceFile, String> {
    ast::parse(root, rel)
}

fn maybe_parse(root: &Path, rel: &str) -> Option<SourceFile> {
    ast::parse(root, rel).ok()
}

/// Token-stream text search, in `syn`'s spaced form (`fs :: rename`).
fn find_first(haystack: &str, needles: &[&str]) -> Option<(usize, String)> {
    needles
        .iter()
        .filter_map(|n| haystack.find(n).map(|i| (i, (*n).to_string())))
        .min_by_key(|(i, _)| *i)
}

/// The directory must exist and hold adapters before any of these rules
/// mean anything. Landing the audits first (spec section 18: "Land
/// FIRST, failing, then implement") means this is the *first* failure
/// the repo-level run reports, and it names what is missing rather than
/// passing vacuously over an empty directory.
fn adapters_or_err(root: &Path) -> Result<Vec<String>, String> {
    let files = build_adapter_files(root);
    if files.is_empty() {
        return Err(
            "crates/core/src/build_adapters/ has no adapter modules: section 18 requires a \
             `BuildAdapter` trait with a static registry and one module per ecosystem family \
             (Cargo ported onto the trait, then Node, Gradle and Maven)"
                .into(),
        );
    }
    Ok(files)
}

// ---------------------------------------------------------------------
// build_adapters_are_pluggable
// ---------------------------------------------------------------------

/// Statement: a build adapter is one line in a static registry, names no
/// other adapter, and nothing dispatches to it by matching an ecosystem
/// id.
///
/// The precedent is exact. `agents/mod.rs` grew a fourteen-arm
/// `match tool_id` and a second copy of it in `actions.rs`, and adding a
/// tool meant editing four places. The Cargo build code is at the same
/// fork today: `consumers/cargo.rs` calls `cargo_artifacts::folded_units`
/// by name, and the obvious way to add Node is a second named call, then
/// a `match ecosystem` once there are four. So the rule lands before the
/// second adapter exists, not after.
pub fn build_adapters_are_pluggable(root: &Path) -> Result<(), String> {
    let adapters = adapters_or_err(root)?;
    let names: Vec<String> = adapters.iter().map(|r| module_name(r)).collect();

    // (1) No adapter names another adapter. One ecosystem's layout
    // change must never be able to move another's identification.
    for rel in &adapters {
        let me = module_name(rel);
        let f = parse(root, rel)?;
        for other in &names {
            if other == &me {
                continue;
            }
            for form in [
                format!("super :: {other} ::"),
                format!("crate :: build_adapters :: {other} ::"),
            ] {
                for func in ast::functions(&f.ast) {
                    if func.body.contains(&form) || func.sig.contains(&form) {
                        return Err(format!(
                            "{rel}::{} reaches into adapter `{other}`: an adapter names no other \
                             adapter. Genuinely shared parsing goes in a neutral helper \
                             (`jvm_common.rs`) that neither adapter reaches the other through",
                            func.name
                        ));
                    }
                }
            }
        }
    }

    // (2) No central `match` over adapter ids outside the registry.
    let mut dispatch_files = vec![
        "crates/core/src/build_adapters/mod.rs".to_string(),
        "crates/core/src/actions.rs".to_string(),
        "crates/core/src/render.rs".to_string(),
        "crates/core/src/consumers/cargo.rs".to_string(),
    ];
    dispatch_files.extend(ast::rust_files_under(root, "crates/tui/src"));
    dispatch_files.extend(ast::rust_files_under(root, "crates/cli/src"));
    for rel in dispatch_files {
        let Some(f) = maybe_parse(root, &rel) else {
            continue;
        };
        for func in ast::functions(&f.ast) {
            if !func.body.contains("match ") {
                continue;
            }
            for other in &names {
                let upper = format!("{}_ADAPTER_ID", other.to_ascii_uppercase());
                if func.body.contains(&upper) {
                    return Err(format!(
                        "{rel}::{} matches on `{upper}`: build-adapter dispatch goes through \
                         `build_adapters::Registry`, never a central ecosystem match",
                        func.name
                    ));
                }
            }
        }
    }

    // (3) Registry <-> module set equality, exactly once each.
    let registry = parse(root, "crates/core/src/build_adapters/registry.rs").map_err(|e| {
        format!("{e}; section 18 requires a static `build_adapters::Registry::with_builtins()`")
    })?;
    for name in &names {
        let count: usize = [format!("{name}::Adapter"), format!("{name} :: Adapter")]
            .iter()
            .map(|needle| {
                registry
                    .text
                    .match_indices(needle.as_str())
                    .filter(|(at, _)| {
                        registry.text[..*at]
                            .chars()
                            .next_back()
                            .is_none_or(|c| !c.is_alphanumeric() && c != '_')
                    })
                    .count()
            })
            .sum();
        if count != 1 {
            return Err(format!(
                "build_adapters/registry.rs registers `{name}` {count} times; exactly once (an \
                 unregistered adapter identifies nothing and no test notices)"
            ));
        }
    }

    // (4) Registry ids == matrix ids == docs table rows. The matrix is
    // the published support claim; the registry is what runs.
    let matrix = parse(root, "crates/core/src/build_adapters/matrix.rs")?;
    let matrix_ids: BTreeSet<String> = ast::string_literals(&matrix.ast)
        .into_iter()
        .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_lowercase() || c == '-'))
        .collect();
    if matrix_ids.is_empty() {
        return Err(
            "build_adapters/matrix.rs exposes no adapter ids to compare with the registry".into(),
        );
    }
    for name in &names {
        let id = name.replace('_', "-");
        if !matrix_ids.contains(&id) && !matrix_ids.contains(name) {
            return Err(format!(
                "build_adapters/matrix.rs has no row for adapter `{name}`: an adapter with no \
                 capability row is an undocumented support claim"
            ));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------
// build_adapters_are_inspection_only
// ---------------------------------------------------------------------

const BUILD_ACTION_REFERENCES: &[&str] = &["actions ::", "trash ::", "Plan {", "Grant {", "Ledger"];

/// Resolved paths a build adapter may never call.
///
/// `Command::new` is the one that matters most here and is the one an
/// author will reach for first: `npm ls --json`, `gradle
/// dependencies`, `mvn help:evaluate` and `cargo metadata` all answer
/// exactly the questions an adapter wants, and all four *evaluate the
/// project's own build definition* to do it. Running an untrusted
/// build script during observation is the handoff's own hard
/// constraint, so it is a call-level rule, not a review note.
const BUILD_FORBIDDEN_CALLS: &[&str] = &[
    "fs::rename",
    "fs::remove_file",
    "fs::remove_dir",
    "fs::remove_dir_all",
    "fs::write",
    "fs::create_dir",
    "fs::create_dir_all",
    "fs::set_permissions",
    "fs::copy",
    "fs::hard_link",
    "Command::new",
    "Command::output",
    "Command::status",
    "Command::spawn",
    "process::Command",
];

pub fn build_adapters_are_inspection_only(root: &Path) -> Result<(), String> {
    let mut problems: Vec<String> = Vec::new();
    for rel in adapters_or_err(root)? {
        let f = parse(root, &rel)?;
        for func in ast::functions(&f.ast) {
            if let Some((_, call)) = find_first(&func.body, BUILD_ACTION_REFERENCES) {
                problems.push(format!(
                    "{rel}::{} references `{}`: identification never acts. An adapter declares an \
                     action capability on the unit; only the shared sink executes anything, after \
                     its own live rechecks",
                    func.name,
                    call.trim()
                ));
            }
        }
        for c in crate::resolve::production_calls(&f.ast) {
            if c.method && !c.path.starts_with("Command") {
                continue;
            }
            if let Some(bad) = BUILD_FORBIDDEN_CALLS
                .iter()
                .find(|p| crate::resolve::path_ends_with(&c.path, p))
            {
                problems.push(format!(
                    "{rel}::{} calls `{}` (resolved: {bad}): a build adapter identifies from \
                     read-only metadata. It never writes, never deletes, and never runs `npm`, \
                     `gradle`, `mvn` or `cargo` -- each of those evaluates the project's own build \
                     definition to answer",
                    c.func, c.written
                ));
            }
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        problems.sort();
        problems.dedup();
        Err(problems.join("\n  "))
    }
}

// ---------------------------------------------------------------------
// build_adapters_read_bounded_manifests_only
// ---------------------------------------------------------------------

const UNBOUNDED_READS: &[&str] = &[
    "fs :: read_to_string (",
    "fs :: read (",
    ". read_to_end (",
    ". read_to_string (",
    "serde_json :: from_reader (",
    "BufReader :: new (",
    ". lines ( )",
];

/// Statement: a build adapter's only content access is the shared
/// capped `build_adapters::bounded_io::read_manifest(path, cap)`, for
/// named manifest/lockfile/fingerprint files.
///
/// The cost argument and the safety argument point the same way. A
/// `pnpm-lock.yaml` in a large monorepo is tens of megabytes and a
/// `node_modules` tree holds one `package.json` per package; an adapter
/// that reads whole files reads gigabytes to answer "which packages are
/// installed". And an unbounded read of an arbitrary path under a
/// build directory is an unbounded read of whatever a build happened to
/// put there.
pub fn build_adapters_read_bounded_manifests_only(root: &Path) -> Result<(), String> {
    let mut problems: Vec<String> = Vec::new();
    for rel in adapters_or_err(root)? {
        let f = parse(root, &rel)?;
        for func in ast::functions(&f.ast) {
            if let Some((_, call)) = find_first(&func.body, UNBOUNDED_READS) {
                problems.push(format!(
                    "{rel}::{} reads file contents with `{}`: an adapter's only content access is \
                     `build_adapters::bounded_io::read_manifest(path, cap)`, capped, for named \
                     manifest and fingerprint files",
                    func.name,
                    call.trim()
                ));
            }
        }
        // Resolved, so `use std::fs::read_to_string as slurp` is caught.
        for c in crate::resolve::production_calls(&f.ast) {
            if c.method {
                continue;
            }
            for bad in ["fs::read_to_string", "fs::read", "io::read_to_string"] {
                if crate::resolve::path_ends_with(&c.path, bad) {
                    problems.push(format!(
                        "{rel}::{} calls `{}` (resolved: {bad}): content reads go through the \
                         capped `bounded_io::read_manifest`",
                        c.func, c.written
                    ));
                }
            }
        }
    }
    let bounded = parse(root, "crates/core/src/build_adapters/bounded_io.rs").map_err(|e| {
        format!(
            "{e}; build adapters need one shared capped manifest reader \
             (`bounded_io::read_manifest`)"
        )
    })?;
    if !bounded.text.contains("MAX_MANIFEST_BYTES") {
        problems.push(
            "build_adapters/bounded_io.rs has no `MAX_MANIFEST_BYTES` cap constant".to_string(),
        );
    }
    if problems.is_empty() {
        Ok(())
    } else {
        problems.sort();
        problems.dedup();
        Err(problems.join("\n  "))
    }
}

// ---------------------------------------------------------------------
// build_adapters_do_not_traverse
// ---------------------------------------------------------------------

const BUILD_TRAVERSAL_CALLS: &[&str] = &[":: read_dir (", "read_dir (", "walkdir", "jwalk"];

/// Statement: directory structure reaches a build adapter through the
/// folded walk rows in its context, or through the capped
/// `locations::shallow_list`.
///
/// This is the adapter-scoped half of
/// `no-second-traversal-on-report-path`. A `node_modules` tree and a
/// `~/.m2/repository` are the two largest directory trees on a typical
/// developer machine; an adapter that walks either one turns an ordinary
/// refresh into a second full scan of the thing the folded walk just
/// measured.
pub fn build_adapters_do_not_traverse(root: &Path) -> Result<(), String> {
    let mut problems: Vec<String> = Vec::new();
    for rel in adapters_or_err(root)? {
        let f = parse(root, &rel)?;
        for func in ast::functions(&f.ast) {
            if let Some((_, call)) = find_first(&func.body, BUILD_TRAVERSAL_CALLS) {
                problems.push(format!(
                    "{rel}::{} traverses with `{}`: directory structure reaches an adapter through \
                     the folded walk rows in its context, or the capped `locations::shallow_list`",
                    func.name,
                    call.trim()
                ));
            }
        }
        for c in crate::resolve::production_calls(&f.ast) {
            if c.method {
                continue;
            }
            for bad in ["fs::read_dir", "WalkDir::new"] {
                if crate::resolve::path_ends_with(&c.path, bad) {
                    problems.push(format!(
                        "{rel}::{} calls `{}` (resolved: {bad}): listings go through \
                         `locations::shallow_list`, which is capped, counted and refuses symlinks",
                        c.func, c.written
                    ));
                }
            }
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        problems.sort();
        problems.dedup();
        Err(problems.join("\n  "))
    }
}

// ---------------------------------------------------------------------
// build_units_built_through_builder
// ---------------------------------------------------------------------

/// Statement: adapters build nested units with `NestedUnitBuilder`,
/// never a `NestedArtifact { .. }` literal.
///
/// A literal has to spell out `coverage`, `membership`,
/// `physical_bytes`, `physical_total` and the action capability. Every
/// one of those has a *safe* default that a literal can silently get
/// wrong: coverage `supported: true` on a layout nobody tested,
/// `Membership::Exclusive` on a hardlinked store entry (which
/// double-counts), a nonzero `physical_total` on an aggregate (which
/// double-counts again), and an action capability other than
/// `InspectionOnly` on an adapter with no action support at all.
/// The constructor applies the honest defaults; overriding one is a
/// named method call, visible in the diff.
pub fn build_units_built_through_builder(root: &Path) -> Result<(), String> {
    let mut problems: Vec<String> = Vec::new();
    for rel in adapters_or_err(root)? {
        let f = parse(root, &rel)?;
        let res = crate::resolve::resolver(&f.ast);
        for (func, path) in ast::struct_literal_sites(&f.ast) {
            let resolved = res.resolve(&path);
            for what in ["NestedArtifact", "CandidateNestedUnit"] {
                if crate::resolve::path_ends_with(&resolved, what) {
                    problems.push(format!(
                        "{rel}::{func} builds a `{what} {{ .. }}` struct literal (written \
                         `{path}`): units are built with `NestedUnitBuilder::new(adapter, role, \
                         path)`, whose constructor applies the role vocabulary, the accounting \
                         basis, the timestamp provenance and `InspectionOnly` -- every one of \
                         which a literal can silently get wrong in the direction that inflates a \
                         total or promises an action"
                    ));
                }
            }
        }
        // Declaring support is the same reviewed act as lifting a
        // protection: an empty reason is an unreviewed claim.
        for c in crate::resolve::production_calls(&f.ast) {
            if (c.path == "supported_with_reason" || c.path == "acts_with_reason")
                && c.args
                    .iter()
                    .all(|a| a.replace(' ', "").is_empty() || a.replace(' ', "") == "\"\"")
            {
                problems.push(format!(
                    "{rel}::{} lifts an inspection-only/unsupported default with no stated reason",
                    c.func
                ));
            }
        }
    }
    let modrs = parse(root, "crates/core/src/build_adapters/mod.rs")?;
    if !modrs.text.contains("NestedUnitBuilder") {
        problems.push("build_adapters/mod.rs does not define `NestedUnitBuilder`".to_string());
    }
    if problems.is_empty() {
        Ok(())
    } else {
        problems.sort();
        problems.dedup();
        Err(problems.join("\n  "))
    }
}

// ---------------------------------------------------------------------
// build_adapter_test_contract
// ---------------------------------------------------------------------

/// The five things every build adapter proves about *itself*, chosen
/// because each one is a specific way the epic's issues say this work
/// fails: an unsupported layout reported as "nothing here"; a manifest
/// read that grows with the tree; identification that runs the project's
/// build; variants collapsed by basename (`dist` in two workspaces,
/// `debug` for two target triples); and age read as obsolescence.
const REQUIRED_BUILD_TESTS: &[&str] = &[
    "unknown_layout_is_explicit_not_empty",
    "identification_reads_no_more_than_manifest_cap",
    "no_project_or_build_code_is_executed",
    "variants_never_collapse_by_basename",
    "age_is_not_obsolescence",
];

/// Whether `text` defines a test named `name` that is not `#[ignore]`d
/// and contains at least one assertion. Section 17 item 4: a rule that
/// "the test exists" must also assert the test is not ignored and is not
/// an empty stub, or the audit is satisfied by a name.
fn defines_real_test(f: &SourceFile, name: &str) -> Result<(), String> {
    let needle = format!("fn {name}(");
    let Some(at) = f.text.find(&needle) else {
        return Err("missing".into());
    };
    let start = at.saturating_sub(200);
    if f.text[start..at].contains("#[ignore") {
        return Err("`#[ignore]`d".into());
    }
    // The body: from the name to the next test fn, or the end.
    let rest = &f.text[at..];
    let end = rest[needle.len()..]
        .find("\n    fn ")
        .map(|i| i + needle.len())
        .unwrap_or(rest.len());
    let body = &rest[..end];
    let asserts = body.contains("assert!")
        || body.contains("assert_eq!")
        || body.contains("assert_ne!")
        || body.contains("contract::");
    if !asserts {
        return Err("has no assertion".into());
    }
    Ok(())
}

pub fn build_adapter_test_contract(root: &Path) -> Result<(), String> {
    let mut missing: Vec<String> = Vec::new();
    for rel in adapters_or_err(root)? {
        let f = parse(root, &rel)?;
        let absent: Vec<String> = REQUIRED_BUILD_TESTS
            .iter()
            .filter_map(|t| {
                defines_real_test(&f, t)
                    .err()
                    .map(|why| format!("{t} ({why})"))
            })
            .collect();
        if !absent.is_empty() {
            missing.push(format!("{rel}: {}", absent.join(", ")));
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "every build adapter proves the same five things about itself; these do not:\n  {}",
            missing.join("\n  ")
        ))
    }
}

// ---------------------------------------------------------------------
// build_adapter_matrix_matches_docs
// ---------------------------------------------------------------------

/// Statement: the support-matrix table in `docs/build-artifacts.md` has
/// exactly one row per registered adapter family, and every row's
/// "actions" column says inspection-only for as long as no adapter
/// implements an action.
///
/// The agent-side version of this rule caught a published table
/// promising actions the code refused. Here the same failure would be
/// worse: the whole point of this chunk is identification *without*
/// cleanup, so a docs row reading anything but "inspection only" is a
/// promise nothing in the codebase can keep.
pub fn build_adapter_matrix_matches_docs(root: &Path) -> Result<(), String> {
    let adapters = adapters_or_err(root)?;
    let doc_path = root.join("docs/build-artifacts.md");
    let doc = std::fs::read_to_string(&doc_path).map_err(|e| {
        format!(
            "docs/build-artifacts.md: {e}; section 18 requires a published capability matrix the \
             code is checked against"
        )
    })?;
    let matrix = parse(root, "crates/core/src/build_adapters/matrix.rs")?;
    let mut problems = Vec::new();
    for rel in &adapters {
        let id = module_name(rel).replace('_', "-");
        if !doc.contains(&format!("`{id}`")) {
            problems.push(format!(
                "docs/build-artifacts.md has no matrix row naming `{id}`"
            ));
        }
        if !matrix.text.contains(&format!("\"{id}\"")) {
            problems.push(format!("build_adapters/matrix.rs has no entry for `{id}`"));
        }
    }
    // Every table row that names an action capability must say the same
    // thing the code says. Until an adapter implements an action, that
    // is "inspection only" everywhere.
    for line in doc.lines() {
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        if cells.len() < 3 || !line.starts_with('|') {
            continue;
        }
        // The row's *first cell* must name the adapter, backticked and
        // whole. A substring match read `maven-metadata-local.xml` in
        // the origin-evidence table as a Maven support row and demanded
        // an actions column of it.
        let names_adapter = adapters
            .iter()
            .any(|rel| cells[1].starts_with(&format!("`{}`", module_name(rel).replace('_', "-"))));
        if !names_adapter {
            continue;
        }
        let lowered = line.to_ascii_lowercase();
        if !lowered.contains("inspection only") && !lowered.contains("inspection-only") {
            problems.push(format!(
                "docs/build-artifacts.md row `{}` does not state inspection only: no build \
                 adapter implements an action, so any other claim is unkept",
                cells[1]
            ));
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        problems.sort();
        problems.dedup();
        Err(problems.join("\n  "))
    }
}

// ---------------------------------------------------------------------
// build_adapters_reuse_under_event_coverage
// ---------------------------------------------------------------------

/// Statement: an unchanged build container is replayed through the same
/// `EventCoverage`-gated container seam the agent adapters use, never
/// through a directory mtime stamp alone.
///
/// stack/13 landed that gate because a directory's own stamp does not
/// move when a file *inside a subdirectory* changes, so "the stamp is
/// the same" is not "nothing changed". The build side is where that
/// matters most: a `target/` or `node_modules` whose top-level stamp
/// never moves would be replayed forever.
pub fn build_adapters_reuse_under_event_coverage(root: &Path) -> Result<(), String> {
    let modrs = parse(root, "crates/core/src/build_adapters/mod.rs").map_err(|e| {
        format!("{e}; section 18 requires the build adapters' shared model and container seam")
    })?;
    let mut problems = Vec::new();
    if !modrs.text.contains("EventCoverage") && !modrs.text.contains("event_coverage") {
        problems.push(
            "build_adapters/mod.rs does not consult `EventCoverage`: container reuse is gated on \
             trusted event coverage, exactly as the agent containers are, never on a directory \
             stamp alone"
                .to_string(),
        );
    }
    // A stamp-only reuse decision is the specific shortcut. If the
    // module compares modification times to decide reuse, the coverage
    // gate must be a condition of that decision, not merely present in
    // the file.
    for func in ast::functions(&modrs.ast) {
        // Name, signature and body together. The sweep's own shape was a
        // function *called* `reuse_container` whose body said only
        // `previous_mtime == current_mtime`: a body-only rule cannot see
        // that the comparison is a reuse decision, and a name-only rule
        // cannot see that it is a stamp.
        let surface = format!("{} {} {}", func.name, func.sig, func.body);
        let decides_reuse = surface.contains("reuse") || surface.contains("replay");
        let uses_stamp = surface.contains("mtime") || surface.contains("mod_time");
        if decides_reuse
            && uses_stamp
            && !surface.contains("coverage")
            && !surface.contains("Coverage")
        {
            problems.push(format!(
                "build_adapters/mod.rs::{} decides reuse from a modification stamp with no \
                 coverage gate in the same function: a directory's own stamp does not move when a \
                 file inside a subdirectory changes",
                func.name
            ));
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        problems.sort();
        problems.dedup();
        Err(problems.join("\n  "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(files: &[(&str, &str)]) -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        for (rel, body) in files {
            let p = tmp.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        }
        tmp
    }

    const REGISTRY: &str = r#"
        pub struct Registry;
        impl Registry {
            pub fn with_builtins() -> Self { let _ = super::cargo::Adapter; Self }
        }
    "#;
    const MATRIX: &str = r#"pub const IDS: &[&str] = &["cargo"];"#;
    const BOUNDED: &str = r#"pub const MAX_MANIFEST_BYTES: usize = 262144;"#;
    const MODRS: &str = r#"pub struct NestedUnitBuilder; pub fn gate(c: &EventCoverage) {}"#;

    fn base() -> Vec<(&'static str, &'static str)> {
        vec![
            ("crates/core/src/build_adapters/registry.rs", REGISTRY),
            ("crates/core/src/build_adapters/matrix.rs", MATRIX),
            ("crates/core/src/build_adapters/bounded_io.rs", BOUNDED),
            ("crates/core/src/build_adapters/mod.rs", MODRS),
        ]
    }

    #[test]
    fn an_absent_build_adapters_directory_is_the_first_failure() {
        let tmp = tempfile::tempdir().unwrap();
        let err = build_adapters_are_pluggable(tmp.path()).unwrap_err();
        assert!(err.contains("has no adapter modules"), "{err}");
    }

    #[test]
    fn an_adapter_naming_another_adapter_is_rejected() {
        let mut files = base();
        files.push((
            "crates/core/src/build_adapters/cargo.rs",
            "pub fn f() { super::node::helper(); }",
        ));
        files.push((
            "crates/core/src/build_adapters/node.rs",
            "pub fn helper() {}",
        ));
        let tmp = tree(&files);
        let err = build_adapters_are_pluggable(tmp.path()).unwrap_err();
        assert!(err.contains("reaches into adapter `node`"), "{err}");
    }

    #[test]
    fn spawning_a_build_tool_during_identification_is_rejected() {
        let mut files = base();
        files.push((
            "crates/core/src/build_adapters/node.rs",
            "use std::process::Command as Runner;\npub fn f() { let _ = Runner::new(\"npm\"); }",
        ));
        let tmp = tree(&files);
        let err = build_adapters_are_inspection_only(tmp.path()).unwrap_err();
        assert!(err.contains("Command"), "{err}");
    }

    #[test]
    fn an_aliased_whole_file_read_is_rejected() {
        let mut files = base();
        files.push((
            "crates/core/src/build_adapters/node.rs",
            "use std::fs::read_to_string as slurp;\npub fn f(p: &std::path::Path) { let _ = \
             slurp(p); }",
        ));
        let tmp = tree(&files);
        let err = build_adapters_read_bounded_manifests_only(tmp.path()).unwrap_err();
        assert!(err.contains("read_to_string"), "{err}");
    }

    #[test]
    fn an_ignored_required_test_does_not_satisfy_the_contract() {
        let mut files = base();
        let body = REQUIRED_BUILD_TESTS
            .iter()
            .map(|t| {
                if *t == "age_is_not_obsolescence" {
                    format!("#[ignore]\n    fn {t}() {{ assert!(true); }}\n")
                } else {
                    format!("    fn {t}() {{ assert!(true); }}\n")
                }
            })
            .collect::<String>();
        let src = format!("#[cfg(test)]\nmod tests {{\n{body}}}\n");
        let leaked: &'static str = Box::leak(src.into_boxed_str());
        files.push(("crates/core/src/build_adapters/node.rs", leaked));
        let tmp = tree(&files);
        let err = build_adapter_test_contract(tmp.path()).unwrap_err();
        assert!(err.contains("age_is_not_obsolescence"), "{err}");
    }
}
