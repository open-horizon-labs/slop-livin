//! The walk and history guardrails: one byte formatter, canonical roots,
//! FSEvents before a full walk, zstd Parquet everywhere, current plus
//! reverse deltas, folding only for classified artifacts, symlinks never
//! followed, incremental walks only of changed subtrees, the parallel
//! pool, Int32 minutes, and no second traversal on the report path.

use super::{anchors, load, verdict};
use crate::ast;
use crate::program::{Fun, Program, contains_token, split_top_level};
use crate::resolve::{self, Honoured};
use std::collections::HashSet;
use std::path::Path;

// ---------------------------------------------------------------------
// one_byte_formatter
// ---------------------------------------------------------------------

fn binary_label(s: &str) -> bool {
    ["KiB", "MiB", "GiB", "TiB"].iter().any(|u| s.contains(u))
}

pub fn one_byte_formatter(root: &Path) -> Result<(), String> {
    let p = load(root);
    let mut problems = Vec::new();
    let the_one = anchors(&p, &["render::human_bytes"], &mut problems);
    for i in &the_one {
        let f = &p.funs[*i];
        if !contains_token(&f.body, "1000") || contains_token(&f.body, "1024") {
            problems.push(format!("{} must divide by 1000 under SI labels", f.display()));
        }
    }
    // Any other definition of the formatter's name is a second formatter.
    for (i, f) in p.funs.iter().enumerate() {
        if (f.name == "human_bytes" || f.name == "human_signed_bytes") && !the_one.contains(&i) && f.module != "render" {
            problems.push(format!("{} defines a byte formatter; it must re-export core's", f.display()));
        }
    }
    // The TUI re-exports the one formatter.
    if !p.reexports.iter().any(|r| r.module.starts_with("swamp_tui") && resolve::path_ends_with(&r.target, "render::human_bytes_pub")) {
        problems.push("the TUI does not re-export core's byte formatter".into());
    }
    // Anywhere: a function that divides by 1024 and puts a binary unit
    // label into formatted output -- written in the macro, or held in a
    // constant (a unit table) the macro indexes.
    for f in p.funs.iter() {
        let divides = contains_token(&f.body, "1024") || f.body.contains("1024.0") || f.body.replace(' ', "").contains("1<<10");
        if !divides {
            continue;
        }
        let consts: Vec<String> = p
            .consts_reached(f)
            .into_iter()
            .filter(|d| d.literals.iter().any(|l| binary_label(l)))
            .map(|d| d.name.clone())
            .collect();
        let labels = f.macros.iter().any(|m| {
            ["format", "format_args", "write", "writeln", "print", "println", "eprintln"].contains(&m.name.as_str())
                && (m.literals.iter().any(|l| binary_label(l)) || consts.iter().any(|c| contains_token(&m.tokens, c)))
        }) || f.calls.iter().any(|c| c.method && c.path == "push_str" && c.args.iter().any(|a| binary_label(a) || consts.iter().any(|k| contains_token(a, k))));
        if labels {
            problems.push(format!(
                "{} formats bytes with a 1024 divisor and a binary unit label: there is one byte \
                 formatter (render::human_bytes, SI, /1000) and everything else re-exports it",
                f.display()
            ));
        }
    }
    verdict("one byte formatter, SI, everywhere", problems)
}

// ---------------------------------------------------------------------
// legacy_invariants
// ---------------------------------------------------------------------

pub fn legacy_invariants(root: &Path) -> Result<(), String> {
    let p = load(root);
    let mut problems = Vec::new();
    let defs = p.named("canonical_roots");
    if defs.is_empty() {
        problems.push("nothing defines `canonical_roots`: roots are not canonicalized at the boundary".into());
    }
    if !defs.iter().any(|d| !p.callers(*d).is_empty()) {
        problems.push("nothing calls `canonical_roots`: the boundary is not on the scan path".into());
    }
    // Every definition canonicalizes, honours the answer, and returns it.
    for d in &defs {
        let f = &p.funs[*d];
        let canon: Vec<&crate::program::PCall> = f
            .calls
            .iter()
            .filter(|c| c.callee() == "canonicalize" && c.honoured != Honoured::Discarded)
            .collect();
        let tail = f.stmts.last().cloned().unwrap_or_default();
        let bound: Vec<String> = f
            .bindings
            .iter()
            .filter(|b| b.from.contains("canonicalize"))
            .map(|b| b.name.clone())
            .collect();
        let flows = tail.contains("canonicalize") || bound.iter().any(|b| contains_token(&tail, b));
        if canon.is_empty() || !flows {
            problems.push(format!(
                "{} is the canonicalization boundary but does not return the canonicalized roots \
                 (a `canonicalize` whose answer is thrown away is not a boundary)",
                f.display()
            ));
        }
    }
    // Grants carry non-index provenance, and it is read.
    let has = p.types.iter().any(|t| t.name == "Grant" && t.fields.iter().any(|(n, _, _, _)| n == "created_outside_index"));
    let read = p.funs.iter().any(|f| f.fields.iter().any(|x| x == "created_outside_index"));
    if !has || !read {
        problems.push("grants need non-index provenance (`Grant::created_outside_index`, read by the grant check)".into());
    }
    verdict("roots are canonicalized at the boundary; grants carry non-index provenance", problems)
}

// ---------------------------------------------------------------------
// fsevents_before_full_walk / incremental_walk_only_changed_subtrees
// ---------------------------------------------------------------------

pub fn fsevents_before_full_walk(root: &Path) -> Result<(), String> {
    let p = load(root);
    let mut problems = Vec::new();
    let stage: HashSet<usize> = p.named("stage_tracked_with_source").into_iter().collect();
    let entry: HashSet<usize> = p.named("observe_tracked_with_source").into_iter().collect();
    if stage.is_empty() || entry.is_empty() {
        problems.push("`stage_tracked_with_source`/`observe_tracked_with_source` are not defined".into());
    }
    let walks = p.traversal(&HashSet::new());
    // The entry point delegates planning to the stage and never walks
    // any other way.
    let walks_except_stage = p.traversal(&stage);
    for e in &entry {
        let f = &p.funs[*e];
        let delegates = f.calls.iter().enumerate().any(|(ci, c)| c.honoured != Honoured::Discarded && p.target(*e, ci).local.iter().any(|g| stage.contains(g)));
        if !delegates || walks_except_stage.contains(e) {
            problems.push(format!(
                "{} must delegate replay planning to `stage_tracked_with_source` and must not \
                 walk any other way ({})",
                f.display(),
                p.chain(*e, &walks_except_stage, |g| p.funs[g].calls.iter().any(crate::program::traversal_call))
            ));
        }
    }
    // Before the FSEvents replay, nothing walks except under the forced
    // path.
    for s in &stage {
        let f = &p.funs[*s];
        let Some(replay_at) = f.calls.iter().filter(|c| c.method && c.path.starts_with("replay")).map(|c| c.stmt).min() else {
            problems.push(format!("{} never calls the FSEvents replay", f.display()));
            continue;
        };
        for (ci, c) in f.calls.iter().enumerate() {
            if c.stmt >= replay_at {
                continue;
            }
            let forced = c.conditions.iter().any(|k| contains_token(k, "force_full"))
                || f.stmts.get(c.stmt).is_some_and(|t| t.trim_start().starts_with("if") && contains_token(t, "force_full"));
            let walks_here = crate::program::traversal_call(c) || p.target(*s, ci).local.iter().any(|g| walks.contains(g));
            if walks_here && !forced {
                problems.push(format!(
                    "{}: statement {} walks (`{}`) before the FSEvents replay without the \
                     force_full guard",
                    f.display(),
                    c.stmt,
                    c.written
                ));
            }
        }
    }
    verdict("FSEvents replay decides before any full walk", problems)
}

pub fn incremental_walk_only_changed_subtrees(root: &Path) -> Result<(), String> {
    let p = load(root);
    let mut problems = Vec::new();
    let defs = p.named("apply_incremental");
    if defs.is_empty() {
        problems.push("`apply_incremental` is not defined".into());
    }
    // The targeted re-walkers the incremental path must use.
    let targeted: HashSet<usize> = p
        .named("attribute_one_worktree")
        .into_iter()
        .chain(p.funs.iter().enumerate().filter(|(_, f)| f.module == "walk" && f.name.starts_with("resize_artifact")).map(|(i, _)| i))
        .collect();
    if targeted.is_empty() {
        problems.push("the targeted re-walkers (`attribute_one_worktree`, `resize_artifact*`) are gone".into());
    }
    let other_walks = p.traversal(&targeted);
    for d in &defs {
        let f = &p.funs[*d];
        let calls_targeted = |pred: &dyn Fn(&Fun) -> bool| {
            f.calls.iter().enumerate().any(|(ci, _)| p.target(*d, ci).local.iter().any(|g| targeted.contains(g) && pred(&p.funs[*g])))
        };
        if !calls_targeted(&|g| g.name == "attribute_one_worktree") || !calls_targeted(&|g| g.name.starts_with("resize_artifact")) {
            problems.push(format!(
                "{} must re-walk only the implicated worktrees and artifacts \
                 (`attribute_one_worktree`, `resize_artifact*`), by calling them",
                f.display()
            ));
        }
        if other_walks.contains(d) {
            problems.push(format!(
                "{} walks by another route ({}): the incremental path re-walks only changed \
                 subtrees",
                f.display(),
                p.chain(*d, &other_walks, |g| p.funs[g].calls.iter().any(crate::program::traversal_call))
            ));
        }
    }
    verdict("the incremental walk re-walks only changed subtrees", problems)
}

// ---------------------------------------------------------------------
// column_store_parquet_zstd / dir_mtime_int32_minutes / reverse deltas
// ---------------------------------------------------------------------

pub fn column_store_parquet_zstd(root: &Path) -> Result<(), String> {
    let p = load(root);
    let mut problems = Vec::new();
    let mut writers = 0usize;
    let zstd_fns: HashSet<usize> = p.funs.iter().enumerate().filter(|(_, f)| f.body.contains("Compression :: ZSTD")).map(|(i, _)| i).collect();
    for (i, f) in p.funs.iter().enumerate() {
        for c in &f.calls {
            if c.method || !(c.is("ArrowWriter::try_new") || c.is("ArrowWriter::new")) {
                continue;
            }
            writers += 1;
            let props = c.args.get(2).map(|a| a.replace(' ', "")).unwrap_or_default();
            let none = props.is_empty() || props == "None" || props.ends_with("::None");
            // The properties must come from a ZSTD declaration: in this
            // body, or from a helper that declares it.
            let zstd_here = f.body.contains("Compression :: ZSTD")
                || p.callees(i).iter().any(|g| zstd_fns.contains(g));
            if none || !zstd_here {
                problems.push(format!(
                    "{} writes Parquet (`{}`) without zstd writer properties",
                    f.display(),
                    c.written
                ));
            }
        }
    }
    if writers == 0 {
        problems.push("nothing writes Parquet any more".into());
    }
    verdict("every Parquet writer declares zstd compression", problems)
}

pub fn dir_mtime_int32_minutes(root: &Path) -> Result<(), String> {
    let p = load(root);
    let mut problems = Vec::new();
    let mut minutes = 0usize;
    for f in p.funs.iter() {
        for c in &f.calls {
            if c.method || !c.is("Field::new") {
                continue;
            }
            let name = c.args.first().and_then(|a| p.eval_literal(a)).unwrap_or_default();
            let ty = c.args.get(1).map(|a| a.replace(' ', "")).unwrap_or_default();
            if !(name.contains("mod_time") || name.contains("mtime")) {
                continue;
            }
            if name == "mod_time_min" && ty.ends_with("DataType::Int32") {
                minutes += 1;
                continue;
            }
            problems.push(format!(
                "{} declares a modification-time column `{name}` as `{ty}`: it is `mod_time_min`, \
                 Int32 minutes",
                f.display()
            ));
        }
    }
    for s in ["dirs_schema", "files_schema"] {
        if p.named(s).is_empty() {
            problems.push(format!("`{s}` is not defined"));
        }
    }
    if minutes < 2 {
        problems.push(format!("only {minutes} schema(s) declare `mod_time_min` Int32; the dirs and files tables both do"));
    }
    verdict("directory and file modification times are Int32 minutes", problems)
}

/// The current-table and reverse-delta path helpers the guardrail names.
const CURRENT_PATHS: &[&str] = &["growth::current_path", "growth::dirs_current_path", "growth::files_current_path"];
const DELTA_PATHS: &[&str] = &["growth::next_delta_path", "growth::next_seq_path"];

pub fn reverse_delta_current_plus_deltas(root: &Path) -> Result<(), String> {
    let p = load(root);
    let mut problems = Vec::new();
    let current = anchors(&p, CURRENT_PATHS, &mut problems);
    let delta = anchors(&p, DELTA_PATHS, &mut problems);
    let writes = p.destructive();
    // A history writer: something that computes a current-table path and
    // writes. It must also compute a delta path, honour it, and pass it
    // on to a write.
    let mut writers = 0usize;
    for (i, f) in p.funs.iter().enumerate() {
        let calls_current = f.calls.iter().enumerate().any(|(ci, _)| p.target(i, ci).local.iter().any(|g| current.contains(g)));
        if !calls_current || !writes.contains(&i) || current.contains(&i) || delta.contains(&i) {
            continue;
        }
        writers += 1;
        let deltas: Vec<&crate::program::PCall> = f
            .calls
            .iter()
            .enumerate()
            .filter(|(ci, _)| p.target(i, *ci).local.iter().any(|g| delta.contains(g)))
            .map(|(_, c)| c)
            .collect();
        let used = deltas.iter().any(|c| c.honoured != Honoured::Discarded);
        if !used {
            problems.push(format!(
                "{} rewrites the current table without appending a reverse delta (a delta path \
                 computed and dropped is not a delta)",
                f.display()
            ));
        }
    }
    for name in ["observe_and_annotate", "observe_and_annotate_dirs", "observe_and_annotate_files"] {
        for d in p.named(name) {
            let f = &p.funs[d];
            let reaches: HashSet<usize> = p.reachable(&[d], &HashSet::new());
            if reaches.is_disjoint(&current) || reaches.is_disjoint(&delta) {
                problems.push(format!("{} must rewrite the current table and append a delta", f.display()));
            }
        }
    }
    if writers == 0 {
        problems.push("nothing writes the current history table".into());
    }
    verdict("history is a current table plus reverse deltas", problems)
}

// ---------------------------------------------------------------------
// scheduled_refresh_launchagent
// ---------------------------------------------------------------------

pub fn scheduled_refresh_launchagent(root: &Path) -> Result<(), String> {
    let p = load(root);
    let mut problems = Vec::new();
    if !p.types.iter().any(|t| t.krate == "swamp" && t.name == "Command" && t.variants.iter().any(|v| v == "Schedule")) {
        problems.push("cli: `Command::Schedule` subcommand missing".into());
    }
    let mains: Vec<usize> = p.funs.iter().enumerate().filter(|(_, f)| f.krate == "swamp" && f.name == "main").map(|(i, _)| i).collect();
    let live = p.reachable(&mains, &HashSet::new());
    // The scheduler is the module that writes the LaunchAgent plist.
    let sched_files: HashSet<String> = p.funs.iter().filter(|f| f.literals.iter().any(|l| l.contains("LaunchAgents"))).map(|f| f.rel.clone()).collect();
    if sched_files.is_empty() {
        problems.push("nothing writes a LaunchAgent plist".into());
    }
    let mut launchctl = false;
    for (i, f) in p.funs.iter().enumerate() {
        if !sched_files.contains(&f.rel) {
            continue;
        }
        for c in &f.calls {
            if !super::execution::spawn_site(c) {
                continue;
            }
            let prog = super::execution::first_literal(&p, c).unwrap_or_else(|| c.args.first().cloned().unwrap_or_default());
            if !live.contains(&i) {
                problems.push(format!(
                    "{} spawns `{prog}` from the scheduler but nothing the CLI runs reaches it: \
                     what schedules is what runs",
                    f.display()
                ));
            } else if prog == "launchctl" {
                launchctl = true;
            }
        }
    }
    if !launchctl {
        problems.push("nothing the CLI runs spawns `launchctl`: the scheduled refresh is not a LaunchAgent".into());
    }
    verdict("the scheduled refresh is a LaunchAgent installed with launchctl", problems)
}

// ---------------------------------------------------------------------
// folding_only_for_artifacts
// ---------------------------------------------------------------------

/// The measured folding entry points: they re-size a path the caller
/// already classified (`process_size` recurses inside a folded unit;
/// `resize_artifact_stamped` is the one re-size every `resize_artifact*`
/// delegates to). Exactly these, by resolved path -- not a prefix, which
/// let any `resize_artifact_*` fold anything.
pub const FOLDING_ENTRY_POINTS: &[&str] = &["walk::process_size", "walk::resize_artifact_stamped"];

pub fn folding_only_for_artifacts(root: &Path) -> Result<(), String> {
    let p = load(root);
    let mut problems = Vec::new();
    let entries = anchors(&p, FOLDING_ENTRY_POINTS, &mut problems);
    // Each is the walker's own: defined beside the job type it builds.
    let job_modules: HashSet<String> = p.types.iter().filter(|t| t.name == "AttrJob").map(|t| format!("{}::{}", t.krate, t.module)).collect();
    for e in &entries {
        if !job_modules.contains(&Program::module_of(&p.funs[*e])) {
            problems.push(format!("{} is exempt as a folding entry point but is not the walker's own", p.funs[*e].display()));
        }
    }
    let mut folds = 0usize;
    let mut records = 0usize;
    for file in &p.files {
        for s in ast::guarded_sites(&file.ast, "AttrJob::Size") {
            folds += 1;
            let guarded = s.enclosing_if_let_inits.iter().any(|i| i.contains("classify_at"));
            let exempt = entries.iter().any(|e| p.funs[*e].rel == file.rel && p.funs[*e].name == s.func);
            if !guarded && !exempt {
                problems.push(format!(
                    "{}::{} folds (`AttrJob::Size`) outside an `if let Some(kind) = classify_at(..)`",
                    file.rel, s.func
                ));
            }
        }
        for s in ast::guarded_sites(&file.ast, "record_artifact") {
            if s.func == "record_artifact" {
                continue;
            }
            records += 1;
            if !s.enclosing_if_let_inits.iter().any(|i| i.contains("classify_at")) {
                problems.push(format!("{}::{}: record_artifact outside a classify_at guard", file.rel, s.func));
            }
        }
    }
    if folds == 0 || records == 0 {
        problems.push("the walks no longer fold or record artifacts at all".into());
    }
    // classify_at is table-driven and refuses by default.
    for d in p.named("classify_at") {
        let f = &p.funs[d];
        let tail = f.stmts.last().cloned().unwrap_or_default();
        let table = tail.trim() == "None"
            || ["ARTIFACT_KINDS", "MARKED_ARTIFACT_KINDS", "classify_gated", "classify ("].iter().any(|t| tail.contains(t));
        if !table || tail.contains("Some (ArtifactKind") || tail.contains("or (") || f.body.contains("Some (ArtifactKind") {
            problems.push(format!("{} must end in a table lookup or `None`; tail is `{tail}`", f.display()));
        }
    }
    for name in ["classify", "classify_marked"] {
        for d in p.named(name) {
            if p.funs[d].module == "attribution" && p.funs[d].body.contains("Some (ArtifactKind") {
                problems.push(format!("{} invents a kind instead of consulting the tables", p.funs[d].display()));
            }
        }
    }
    for d in p.named("classify_gated") {
        let f = &p.funs[d];
        if !f.body.contains("Some (ArtifactKind") {
            continue;
        }
        let guard = f.body.split("Some (ArtifactKind").next().unwrap_or("");
        let ok = p.funs.iter().any(|g| {
            g.rel == f.rel && g.name != f.name && guard.contains(&format!("{} (", g.name)) && (g.body.contains("marker_present") || g.body.contains("manifest"))
        });
        if !ok {
            problems.push(format!("{} invents a kind without a marker-file convention predicate", f.display()));
        }
    }
    verdict("only classified artifacts are folded", problems)
}

// ---------------------------------------------------------------------
// symlinks_never_followed
// ---------------------------------------------------------------------

pub fn symlinks_never_followed(root: &Path) -> Result<(), String> {
    let p = load(root);
    let mut problems = Vec::new();
    for (i, f) in p.funs.iter().enumerate() {
        let walks_here = f.calls.iter().any(crate::program::traversal_call);
        for (ci, c) in f.calls.iter().enumerate() {
            let following_stat = !c.method && (c.is("fs::metadata") || p.target(i, ci).abs.ends_with("fs::metadata"));
            // A measurement never follows a link: a following stat whose
            // answer is a size, anywhere.
            if following_stat {
                let sized = f.calls.iter().any(|m| m.method && (m.path == "len" || m.path == "blocks") && (m.receiver.contains("metadata") || m.receiver.len() < 3))
                    || f.body.contains(". len ()") && f.body.contains("metadata (");
                if sized || walks_here {
                    problems.push(format!(
                        "{} stats through `{}`, which follows symlinks: a measurement uses \
                         symlink_metadata or the entry's own file type",
                        f.display(),
                        c.written
                    ));
                }
            }
            // Canonicalizing a child dereferences whatever it points at.
            if c.callee() == "canonicalize" && c.args.first().is_some_and(|a| a.contains(". join (")) {
                problems.push(format!("{} canonicalizes a child path, following its link", f.display()));
            }
        }
        if !walks_here {
            continue;
        }
        // In a function that lists a directory: no type question on a
        // *path*, and the symlink guard comes before the descent.
        for c in &f.calls {
            if c.method && ["is_dir", "is_file", "exists"].contains(&c.path.as_str()) {
                let r = c.receiver.replace(' ', "");
                if r.ends_with(".path()") || r.contains(".join(") {
                    problems.push(format!("{}: `{}.{}()` asks about a path, which follows symlinks", f.display(), r, c.path));
                }
            }
        }
    }
    for file in &p.files {
        for lo in ast::descent_guard_order(&file.ast) {
            if let Some(d) = lo.descent
                && lo.guard.is_none_or(|g| g > d)
                && p.funs.iter().any(|f| f.rel == file.rel && f.name == lo.func && f.calls.iter().any(crate::program::traversal_call))
            {
                problems.push(format!("{}::{}: a directory loop descends before discarding symlinks", file.rel, lo.func));
            }
        }
    }
    verdict("symlinks are never followed by a walk or a measurement", problems)
}

// ---------------------------------------------------------------------
// walk_optimized_parallel_pool
// ---------------------------------------------------------------------

pub fn walk_optimized_parallel_pool(root: &Path) -> Result<(), String> {
    let p = load(root);
    let mut problems = Vec::new();
    let entry = anchors(&p, &["walk::discover_and_attribute"], &mut problems);
    let parallel = anchors(&p, &["walk::attribute_parallel"], &mut problems);
    for e in &entry {
        if p.reachable(&[*e], &HashSet::new()).is_disjoint(&parallel) {
            problems.push(format!("{} does not use the parallel walk", p.funs[*e].display()));
        }
    }
    // The serial walk is test-only: no production caller at all.
    let serial = anchors(&p, &["attribution::attribute"], &mut problems);
    for s in &serial {
        for c in p.callers(*s) {
            if !serial.contains(&c) {
                problems.push(format!(
                    "{} calls the serial, test-only walk (`attribution::attribute`)",
                    p.funs[c].display()
                ));
            }
        }
    }
    verdict("production walks use the parallel pool; the serial walk is test-only", problems)
}

// ---------------------------------------------------------------------
// no_second_traversal_on_report_path
// ---------------------------------------------------------------------

pub fn no_second_traversal_on_report_path(root: &Path) -> Result<(), String> {
    let p = load(root);
    let mut problems = Vec::new();
    // The sanctioned seams: the measurement seam (only while it consults
    // the persisted rows first), the bounded primitives, and the
    // declared-project handoff (see `rules::adapters`).
    let mut stop: HashSet<usize> = HashSet::new();
    for (path, cap) in [("locations::shallow_list", "SHALLOW_LIST_CAP"), ("folded_measurement::folded_bytes_bounded_stamped", "max_entries")] {
        for i in anchors(&p, &[path], &mut problems) {
            if super::is_bounded_by(&p.funs[i], cap) {
                stop.insert(i);
            } else {
                problems.push(format!("{} lost its bound `{cap}`", p.funs[i].display()));
            }
        }
    }
    let walks_raw = p.traversal(&HashSet::new());
    for m in anchors(&p, &["folded_measurement::measure"], &mut problems) {
        let f = &p.funs[m];
        let first_walk = f
            .calls
            .iter()
            .enumerate()
            .filter(|(ci, c)| crate::program::traversal_call(c) || p.target(m, *ci).local.iter().any(|g| walks_raw.contains(g)))
            .map(|(_, c)| c.stmt)
            .min();
        let gate = first_walk.is_some_and(|w| {
            f.stmts.iter().take(w).any(|s| {
                s.trim_start().starts_with("if") && contains_token(s, "return") && f.calls.iter().enumerate().any(|(ci, c)| {
                    c.stmt < w && !c.method && s.contains(&c.written.replace("::", " :: ")) && p.target(m, ci).local.iter().any(|g| !walks_raw.contains(g))
                })
            })
        });
        if first_walk.is_some() && !gate {
            problems.push(format!(
                "{} re-walks without first consulting the persisted folded rows (an early return \
                 on a reuse lookup): the allow-list proved where a re-walk is written, not whether \
                 it happens",
                f.display()
            ));
        } else {
            stop.insert(m);
        }
    }
    stop.extend(p.funs.iter().enumerate().filter(|(_, f)| contains_token(&f.ret, "ProjectLinkState")).map(|(i, _)| i));
    // The bus dispatches to consumers by trait object; what they run is
    // the folded walk and its bookkeeping, sanctioned by construction.
    let consumer_methods: HashSet<usize> = p
        .funs
        .iter()
        .enumerate()
        .filter(|(_, f)| f.trait_.as_deref() == Some("Consumer"))
        .map(|(i, _)| i)
        .collect();
    let sanctioned = p.reachable(&consumer_methods.iter().copied().collect::<Vec<_>>(), &stop);
    let mut stop_all = stop.clone();
    stop_all.extend(consumer_methods.iter().copied());
    let walks = p.traversal(&stop_all);
    // The report path's own modules.
    let entries: Vec<usize> = p.defs("report::observe_scope").into_iter().chain(p.defs("bus::run_report")).collect();
    if entries.is_empty() {
        problems.push("neither `report::observe_scope` nor `bus::run_report` is defined".into());
    }
    let on_path = p.reachable(&entries, &stop_all);
    let path_files: HashSet<String> = on_path.iter().filter(|i| !sanctioned.contains(i)).map(|i| p.funs[*i].rel.clone()).collect();
    for (i, f) in p.funs.iter().enumerate() {
        if !path_files.contains(&f.rel) || sanctioned.contains(&i) || stop.contains(&i) {
            continue;
        }
        if walks.contains(&i) {
            problems.push(format!(
                "{} traverses ({}): the ordinary report path traverses only in the folded walk; \
                 use folded rows, cached identification or a bounded primitive",
                f.display(),
                p.chain(i, &walks, |g| p.funs[g].calls.iter().any(crate::program::traversal_call))
            ));
        }
    }
    let _ = split_top_level;
    verdict("no second traversal on the report path", problems)
}
