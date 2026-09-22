//! Independent adversarial review of PR #123 (stack/08-decision-evidence,
//! tip f0181bb). Every test here asserts REQUIRED behavior, not the bug,
//! and every one FAILS on f0181bb.
//!
//! Run (single-threaded: two tests mutate `PATH` for the whole process):
//!
//! ```sh
//! cargo test -p swamp-core --test reviewer_counterexamples_123 \
//!   --target-dir <scratch>/target-audit -- --test-threads=1 --nocapture
//! ```
//!
//! All fixtures are disposable `tempfile` trees. Nothing reads a real
//! home directory, a real tool store, or the user's own swamp state, and
//! nothing is deleted outside the per-test temp directory.

mod fixture;

use std::fs;
use std::path::{Path, PathBuf};
use swamp_core::actions::{self, approve, execute_with_trash, propose};
use swamp_core::evidence::{
    Evidence, EvidenceSource, FactKind, FactStatus, FactSubtype, FactValue,
};
use swamp_core::report::{ArtifactKind, report_full_mode};

// ---------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------

fn report_for(root: &Path, store: &Path) -> swamp_core::Report {
    report_full_mode(
        root,
        None,
        false,
        Some(store),
        Some("1h"),
        true,
        false,
        false,
        true,
    )
    .expect("report")
}

/// Prepends `dir` to `PATH` so a shim executable written there shadows
/// the real one, and returns the previous value for restoration.
/// Process-global: these tests must run with `--test-threads=1`.
fn shadow_path_with(dir: &Path) -> String {
    let previous = std::env::var("PATH").unwrap_or_default();
    let next = format!("{}:{previous}", dir.display());
    unsafe { std::env::set_var("PATH", next) };
    previous
}

fn restore_path(previous: String) {
    unsafe { std::env::set_var("PATH", previous) };
}

fn write_shim(dir: &Path, name: &str, body: &str) {
    let p = dir.join(name);
    fs::write(&p, body).expect("write shim");
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).expect("chmod shim");
}

// ---------------------------------------------------------------------
// 1. Execution's "fresh current-use recheck" fails OPEN.
// ---------------------------------------------------------------------

/// #55/#61 + `activity-and-consumer-evidence-have-limits`: "Unknown
/// remains unknown; it does not become a safety verdict or authorize
/// removal." `occupancy::occupied` -- the gate this new recheck is
/// supposed to supersede -- already fails CLOSED (`unwrap_or(true)`)
/// when `lsof` cannot answer. The new recheck in
/// `actions::execute_with_trash_opts` refuses only on
/// `Known(Bool(true))`, so `Unavailable` (lsof missing, permission
/// denied, query failed) silently authorizes the removal it was added
/// to guard.
#[test]
fn unavailable_current_use_evidence_must_not_authorize_execution() {
    let tmp = tempfile::tempdir().unwrap();
    let fx = fixture::build(tmp.path());
    let store = tempfile::tempdir().unwrap();
    let r = report_for(&fx.root, store.path());

    let plan = propose(&r, None, std::slice::from_ref(&fx.target_dir), "test").expect("plan");
    actions::save_plan(store.path(), &plan).unwrap();
    approve(store.path(), &plan.id, "human:test").unwrap();

    // The source itself cannot answer: `lsof` runs but is denied.
    let shim_dir = tempfile::tempdir().unwrap();
    write_shim(
        shim_dir.path(),
        "lsof",
        "#!/bin/sh\necho 'lsof: permission denied' 1>&2\nexit 1\n",
    );
    let previous = shadow_path_with(shim_dir.path());

    let observed = swamp_core::occupancy::open_file_evidence(&fx.target_dir);
    assert!(
        matches!(observed.status, FactStatus::Unavailable { .. }),
        "precondition: the shim must make current-use Unavailable, got {:?}",
        observed.status
    );

    let trash_dir = tempfile::tempdir().unwrap();
    let result = execute_with_trash(store.path(), &plan.id, "human:test", trash_dir.path());
    restore_path(previous);
    let result = result.unwrap();

    let outcome = &result.outcomes[0];
    assert_ne!(
        outcome.status, "completed",
        "an Unavailable current-use fact must refuse, never authorize: cause {:?}",
        outcome.cause
    );
    assert!(
        fx.target_dir.exists(),
        "the unit must remain in place when current-use could not be established"
    );
}

// ---------------------------------------------------------------------
// 2. Current-use evidence for a directory ignores its own contents.
// ---------------------------------------------------------------------

/// #55: "Associate active-use observations with stable units." A unit
/// IS the directory and everything under it (that is what `execute`
/// renames away). `occupancy::open_file_evidence` runs `lsof -- <dir>`,
/// which reports only handles on the directory itself, then emits a
/// positive `Known(false)` fact noting "no open-file match this pass"
/// for a unit whose contents are demonstrably open. A negative fact
/// about a question that was never asked is worse than `Unknown`: #60
/// renders it, and `render::evidence_warnings` stays silent on it.
#[test]
fn current_use_evidence_for_a_unit_must_cover_the_contents_it_would_remove() {
    let tmp = tempfile::tempdir().unwrap();
    let unit_dir = tmp.path().join("build-output");
    fs::create_dir_all(&unit_dir).unwrap();
    let member = unit_dir.join("log.txt");
    fs::write(&member, b"held open by a live writer").unwrap();

    let _held = fs::File::open(&member).expect("hold the member open");

    // Precondition: lsof works here and does see the open member.
    let member_fact = swamp_core::occupancy::open_file_evidence(&member);
    assert!(
        matches!(
            member_fact.status,
            FactStatus::Known(FactValue::Bool(true))
        ),
        "precondition: lsof must see the open member itself, got {:?}",
        member_fact.status
    );

    let unit_fact = swamp_core::occupancy::open_file_evidence(&unit_dir);
    assert!(
        !matches!(unit_fact.status, FactStatus::Known(FactValue::Bool(false))),
        "a unit whose member is open must not report a known-negative current-use fact \
         (Known(true), or at minimum Unknown with a stated coverage limit): got {:?}",
        unit_fact.status
    );
}

// ---------------------------------------------------------------------
// 3. A corrupt human-protection file silently lifts every protection.
// ---------------------------------------------------------------------

/// #60: "do not allow scanned project files or agent observations to
/// authorize or remove protections." `agents::load_protect` parses with
/// `serde_json::from_str(..).unwrap_or_default()`, so an unparseable
/// `agent_protect.json` reads as "nothing is protected". #123 makes the
/// blast radius larger, not smaller: `propose_checking_protection` (new
/// here) is fed by the CLI as `protect_list(..).unwrap_or_default()`
/// (`crates/cli/src/main.rs:1043`), extending a fail-open protection
/// lookup from agent storage to every ordinary artifact row. Worse,
/// `protect_add` then rewrites the file from that empty list, destroying
/// the protections it could not read.
#[test]
fn corrupt_protection_file_must_fail_closed_not_read_as_no_protections() {
    let tmp = tempfile::tempdir().unwrap();
    let fx = fixture::build(tmp.path());
    let store = tempfile::tempdir().unwrap();
    let r = report_for(&fx.root, store.path());

    swamp_core::agents::protect_add(store.path(), &fx.target_dir).expect("protect the unit");
    let protect_file = store.path().join("agent_protect.json");
    assert!(protect_file.exists(), "precondition: protect file written");

    // Truncated/corrupted on disk (an interrupted non-atomic write, a
    // half-synced file): the protections are unreadable, not absent.
    fs::write(&protect_file, b"{\"paths\": [\"").unwrap();

    let listed = swamp_core::agents::protect_list(store.path());
    assert!(
        listed.is_err(),
        "an unreadable protection file must be an error, never an empty protect list: got {listed:?}"
    );

    // And the consequence the CLI actually reaches: a protected unit
    // must not become plannable because its protection file broke.
    let protected = listed.unwrap_or_default();
    let plan = actions::propose_checking_protection(
        &r,
        None,
        std::slice::from_ref(&fx.target_dir),
        "test",
        &protected,
    );
    assert!(
        plan.is_err(),
        "a protected unit must stay refused when its protection file cannot be parsed"
    );
}

// ---------------------------------------------------------------------
// 4. Protection is never rechecked at the sink.
// ---------------------------------------------------------------------

/// CHANGELOG (#123): "`swamp protect` is extended from
/// agent-storage-only to ordinary filesystem artifact rows." It is
/// extended to `propose` only. `execute_with_trash_opts` re-stats the
/// path, re-reads its newest mtime and re-takes occupancy, but never
/// consults the protect list -- so protecting a unit after its plan was
/// approved does not stop the removal. Inspection is not authorization,
/// and an approval taken before a protection cannot spend it.
#[test]
fn protection_added_after_approval_must_stop_ordinary_execution() {
    let tmp = tempfile::tempdir().unwrap();
    let fx = fixture::build(tmp.path());
    let store = tempfile::tempdir().unwrap();
    let r = report_for(&fx.root, store.path());

    let protected_before = swamp_core::agents::protect_list(store.path()).unwrap_or_default();
    let plan = actions::propose_checking_protection(
        &r,
        None,
        std::slice::from_ref(&fx.target_dir),
        "test",
        &protected_before,
    )
    .expect("unprotected at proposal time");
    actions::save_plan(store.path(), &plan).unwrap();
    approve(store.path(), &plan.id, "human:test").unwrap();

    // The human changes their mind between approval and execution.
    swamp_core::agents::protect_add(store.path(), &fx.target_dir).expect("protect");

    let trash_dir = tempfile::tempdir().unwrap();
    let result =
        execute_with_trash(store.path(), &plan.id, "human:test", trash_dir.path()).unwrap();
    let outcome = &result.outcomes[0];
    assert_ne!(
        outcome.status, "completed",
        "execution must re-read the protect list at the sink: cause {:?}",
        outcome.cause
    );
    assert!(
        fx.target_dir.exists(),
        "a path protected before execution must still be there afterwards"
    );
}

// ---------------------------------------------------------------------
// 5. Unchanged work re-spawns one `plutil` per DerivedData folder.
// ---------------------------------------------------------------------

fn external_unit(
    detector_id: &str,
    category: swamp_core::locations::StorageCategory,
    path: &Path,
) -> swamp_core::external::ExternalUnit {
    swamp_core::external::ExternalUnit {
        detector_id: detector_id.to_string(),
        detector_name: detector_id.to_string(),
        category,
        provenance: swamp_core::locations::Provenance::BuiltinConvention,
        path: path.to_path_buf(),
        bytes: 0,
        hardlinked: false,
        growth_bytes: None,
        regrowth_count: 0,
        observed_at: 0,
        consumers: Vec::new(),
        note: None,
        evidence: Vec::new(),
    }
}

/// Handoff: "Ordinary unchanged work should scale with roots/changed
/// containers, not all files." `consumer_wiring`'s own module doc claims
/// the cost shape is "a full re-parse only when that fingerprint
/// changes". That mtime-keyed cache covers declaration and lockfile
/// *parsing* only. The Xcode DerivedData join
/// (`consumer_wiring::attach_xcode_associations`) has no cache at all:
/// it spawns one `plutil` subprocess per DerivedData subfolder on every
/// single call -- every `report --view external`, every unified
/// `propose`, and every TUI startup -- for containers that did not
/// change.
#[test]
fn a_second_unchanged_pass_must_not_respawn_plutil_for_every_derived_data_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let empty_root = tmp.path().join("root");
    fs::create_dir_all(&empty_root).unwrap();
    let mut report = report_for(&empty_root, store.path());

    let derived = tmp.path().join("Xcode/DerivedData");
    const FOLDERS: usize = 6;
    for i in 0..FOLDERS {
        let sub = derived.join(format!("App{i}-hash{i}"));
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join("info.plist"), b"bplist00").unwrap();
    }
    let mut units = vec![external_unit(
        swamp_core::locations::xcode::XCODE_DETECTOR_ID,
        swamp_core::locations::StorageCategory::BuildOutput,
        &derived,
    )];

    let shim_dir = tempfile::tempdir().unwrap();
    let counter = shim_dir.path().join("plutil-calls");
    write_shim(
        shim_dir.path(),
        "plutil",
        &format!(
            "#!/bin/sh\necho call >> '{}'\ncat <<'PLIST'\n<?xml version=\"1.0\"?>\n<plist version=\"1.0\"><dict><key>WorkspacePath</key><string>/nowhere/App.xcodeproj</string></dict></plist>\nPLIST\n",
            counter.display()
        ),
    );
    let previous = shadow_path_with(shim_dir.path());

    swamp_core::consumer_wiring::attach_associations(
        &mut report,
        &mut units,
        Some(store.path()),
    );
    let after_first = fs::read_to_string(&counter).map(|t| t.lines().count()).unwrap_or(0);

    // Nothing changed: same tree, same units, same store (so any
    // persisted cache is in place).
    let mut units2 = vec![external_unit(
        swamp_core::locations::xcode::XCODE_DETECTOR_ID,
        swamp_core::locations::StorageCategory::BuildOutput,
        &derived,
    )];
    swamp_core::consumer_wiring::attach_associations(
        &mut report,
        &mut units2,
        Some(store.path()),
    );
    let after_second = fs::read_to_string(&counter).map(|t| t.lines().count()).unwrap_or(0);
    restore_path(previous);

    assert!(
        after_first >= FOLDERS,
        "precondition: the first pass must actually read every folder's info.plist, got {after_first}"
    );
    assert_eq!(
        after_second, after_first,
        "an unchanged second pass must not re-spawn a subprocess per unchanged container \
         (first pass {after_first} spawns, second pass total {after_second})"
    );
}

// ---------------------------------------------------------------------
// 6. Rendered reclaimability facts are indistinguishable.
// ---------------------------------------------------------------------

/// #59: "Separate logical size, allocated measurement, estimated
/// reclaimable bytes and observed post-action free-space change with
/// explicit units and provenance." #60: "Show timestamp meaning, source
/// and freshness rather than one unexplained value."
/// `render::render_evidence_lines` prints `kind`, value, source,
/// coverage and note -- but never `subtype`, which is the only thing
/// that distinguishes these facts. `reclaimability::accounting_evidence`
/// gives them the same `EvidenceSource`, so for the ordinary
/// (non-hardlinked) row that `report::attach_decision_evidence` builds,
/// "allocated" and "estimated reclaimable" render as two byte-identical
/// lines. This is what the TUI detail area and `report --view external`
/// actually show.
#[test]
fn rendered_reclaimability_must_distinguish_allocated_from_estimated_reclaimable() {
    let acc = swamp_core::reclaimability::exclusive_allocation(3 * 1024 * 1024);
    let facts = swamp_core::reclaimability::accounting_evidence(
        &acc,
        EvidenceSource::FilesystemMetadata {
            detail: "folded directory allocation".into(),
        },
    );
    let lines = swamp_core::render::render_evidence_lines(&facts);
    assert_eq!(lines.len(), 2, "allocated + estimated reclaimable: {lines:?}");
    assert_ne!(
        lines[0], lines[1],
        "a reader must be able to tell allocated bytes from estimated reclaimable bytes; \
         both render identically: {lines:?}"
    );
}

// ---------------------------------------------------------------------
// 7. A failed current-use query is invisible at the confirmation.
// ---------------------------------------------------------------------

/// `activity-and-consumer-evidence-have-limits`: "missing evidence must
/// not be presented as unused or safe to remove." #60: "Make
/// consequential unknowns actionable." `render::evidence_warnings` --
/// what the TUI's delete-confirmation row shows -- matches only
/// `CurrentUse` + `Known(Bool(true))`. An `Unavailable` current-use
/// fact (lsof missing, permission denied, timed out) produces no line at
/// all, so the confirmation looks exactly like a unit confirmed not to
/// be in use.
#[test]
fn unavailable_current_use_must_be_surfaced_at_the_confirmation() {
    let unavailable = Evidence::unavailable(
        FactKind::CurrentUse,
        FactSubtype::OpenFile,
        EvidenceSource::ProcessQuery {
            tool: "lsof".into(),
        },
        1_700_000_000,
        "permission denied querying open files for this path",
    );
    let warnings = swamp_core::render::evidence_warnings(std::slice::from_ref(&unavailable));
    assert!(
        !warnings.is_empty(),
        "a current-use query that could not run must be visible before authorizing removal, \
         not rendered identically to 'not in use'"
    );

    let unknown = Evidence::unknown(
        FactKind::CurrentUse,
        FactSubtype::Lock,
        EvidenceSource::ManagerLock {
            tool: "cargo".into(),
            path: "/nowhere/.package-cache".into(),
        },
        1_700_000_000,
        "no lock file present",
    );
    assert!(
        !swamp_core::render::evidence_warnings(std::slice::from_ref(&unknown)).is_empty(),
        "an unresolved current-use question must not be silently absent from the confirmation"
    );
}

// ---------------------------------------------------------------------
// 8. Unresolvable Maven coordinates vanish instead of being named.
// ---------------------------------------------------------------------

/// #56/#57: "unresolved aliases/ranges and conflicting declarations
/// remain explicit rather than guessed exact matches"; #123's own
/// session note: "an unparseable lockfile is a named evidence gap, never
/// a silent empty list." `external_associations::parse_pom_xml` `continue`s
/// past every dependency whose `<version>` is an unresolved
/// `${property}` (and every dependency that inherits its version from a
/// parent POM or `dependencyManagement`), returning `Ok(vec![])`. A
/// real multi-module project's POM therefore produces no consumer fact
/// AND no stated gap: the Maven local repository reads as having no
/// declared consumer in this project.
#[test]
fn a_pom_whose_versions_cannot_be_resolved_must_state_the_gap() {
    let tmp = tempfile::tempdir().unwrap();
    let fx = fixture::build(tmp.path());
    let store = tempfile::tempdir().unwrap();

    fs::write(
        fx.checkout.join("pom.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://maven.apache.org/POM/4.0.0">
  <parent>
    <groupId>com.example</groupId><artifactId>parent</artifactId><version>1.0.0</version>
  </parent>
  <properties><spring.version>6.1.4</spring.version></properties>
  <dependencies>
    <dependency>
      <groupId>org.springframework</groupId>
      <artifactId>spring-core</artifactId>
      <version>${spring.version}</version>
    </dependency>
    <dependency>
      <groupId>org.slf4j</groupId>
      <artifactId>slf4j-api</artifactId>
    </dependency>
  </dependencies>
</project>
"#,
    )
    .unwrap();

    let mut report = report_for(&fx.root, store.path());
    let maven_repo = tmp.path().join("m2/repository");
    fs::create_dir_all(&maven_repo).unwrap();
    let mut units = vec![external_unit(
        swamp_core::locations::maven::MAVEN_DETECTOR_ID,
        swamp_core::locations::StorageCategory::Cache,
        &maven_repo,
    )];

    swamp_core::consumer_wiring::attach_associations(&mut report, &mut units, Some(store.path()));

    let source_row_facts: Vec<&Evidence> = report
        .projects
        .iter()
        .flat_map(|p| p.worktrees.iter())
        .filter(|wt| wt.path == fx.checkout)
        .flat_map(|wt| wt.artifacts.iter())
        .filter(|a| a.kind == ArtifactKind::Source)
        .flat_map(|a| a.evidence.iter())
        .collect();

    let names_the_gap = source_row_facts.iter().any(|e| {
        e.kind == FactKind::Consumer
            && !e.is_known()
            && format!("{:?}{:?}", e.status, e.source).contains("spring")
    });
    assert!(
        names_the_gap,
        "a dependency whose version is an unresolved Maven property (or inherited from a parent \
         POM) must appear as an explicit unresolved consumer fact, never be dropped silently. \
         Facts on this worktree's Source row: {source_row_facts:#?}"
    );
}

// ---------------------------------------------------------------------
// 9. Consumer evidence does not survive a report refresh.
// ---------------------------------------------------------------------

/// #53: "Invalidate or refresh evidence on relevant source/identity/
/// config changes"; #60: the same semantic facts across interfaces.
/// `consumer_wiring::attach_associations` is called exactly once, on the
/// already-MERGED report (`crates/tui/src/lib.rs:377`,
/// `crates/cli/src/main.rs:1386`). The per-root `Report`s that the TUI
/// caches in `reports_by_root` never receive those facts, so
/// `App::replace_report_for_root` -> `report::merge_reports`
/// (`crates/tui/src/app.rs:499`) rebuilds the aggregate report without
/// them. Every #56/#57 consumer fact disappears on the first background
/// refresh of any root and never comes back for the rest of the session.
/// This test exercises the same core merge the TUI rebuild uses.
#[test]
fn consumer_evidence_must_survive_a_per_root_report_refresh() {
    let tmp = tempfile::tempdir().unwrap();
    let fx = fixture::build(tmp.path());
    let store = tempfile::tempdir().unwrap();

    fs::write(fx.checkout.join(".tool-versions"), "nodejs 20.11.1\n").unwrap();
    let nvm_versions = tmp.path().join("nvm/versions/node");
    fs::create_dir_all(nvm_versions.join("20.11.1")).unwrap();

    // Startup: one per-root report, cached by root, folded into the
    // aggregate the app renders (`tui::run_scope` ->
    // `App::new_multi_root` + `reports_by_root`).
    let per_root = report_for(&fx.root, store.path());
    let roots = vec![per_root.root.clone()];
    let mut by_root: std::collections::HashMap<PathBuf, swamp_core::Report> =
        std::collections::HashMap::new();
    by_root.insert(per_root.root.clone(), per_root);
    let mut merged = swamp_core::report::merge_reports(&roots, &by_root);

    // ...then the one and only consumer-wiring call, on the MERGED
    // report (`crates/tui/src/lib.rs:377`).
    let mut units = vec![external_unit(
        swamp_core::locations::nvm::NVM_DETECTOR_ID,
        swamp_core::locations::StorageCategory::Installation,
        &nvm_versions,
    )];
    swamp_core::consumer_wiring::attach_associations(&mut merged, &mut units, Some(store.path()));

    let consumer_facts = |r: &swamp_core::Report| -> usize {
        r.projects
            .iter()
            .flat_map(|p| p.worktrees.iter())
            .flat_map(|wt| wt.artifacts.iter())
            .flat_map(|a| a.evidence.iter())
            .filter(|e| e.kind == FactKind::Consumer)
            .count()
    };
    assert!(
        consumer_facts(&merged) > 0,
        "precondition: the declaring worktree must carry a consumer fact after wiring"
    );

    // A background refresh of that root: `App::replace_report_for_root`
    // re-folds `reports_by_root` into a new aggregate.
    let refreshed = swamp_core::report::merge_reports(&roots, &by_root);

    assert!(
        consumer_facts(&refreshed) > 0,
        "a per-root report refresh must not erase every #56/#57 consumer fact the session had"
    );
}

// ---------------------------------------------------------------------
// 10. Docker rows get a filesystem-provenance reclaimability fact.
// ---------------------------------------------------------------------

/// `evidence.rs`'s own first hard rule: "A fact always carries its
/// `EvidenceSource` (what produced it) ... never a bare value with no
/// provenance." #59: "Keep Docker logical objects/layers and host
/// backing-file accounting separate; avoid equating sparse apparent
/// size with reclaimable space."
/// `report::attach_decision_evidence` applies one generic
/// reclaimability block to every row in `wt.artifacts`, Docker rows
/// included, stamping them
/// `FilesystemMetadata { detail: "folded directory allocation" }` and
/// `EstimatedReclaimable = allocated` exactly. A Docker image's bytes
/// came from `docker system df -v` (a daemon-reported, layer-shared,
/// logical number), never from a folded directory walk, and removing
/// the image frees an unknown amount of host backing store.
#[test]
fn docker_reclaimability_must_not_claim_filesystem_provenance() {
    let tmp = tempfile::tempdir().unwrap();
    let fx = fixture::build(tmp.path());
    let store = tempfile::tempdir().unwrap();
    let r = report_full_mode(
        &fx.root,
        Some(&fx.docker_facts),
        false,
        Some(store.path()),
        Some("1h"),
        true,
        false,
        false,
        true,
    )
    .expect("report with docker facts");

    let docker_rows: Vec<_> = r
        .projects
        .iter()
        .flat_map(|p| p.worktrees.iter())
        .flat_map(|wt| wt.artifacts.iter())
        .filter(|a| {
            matches!(
                a.kind,
                ArtifactKind::DockerImage
                    | ArtifactKind::DockerVolume
                    | ArtifactKind::DockerBuildCache
            )
        })
        .collect();
    assert!(
        !docker_rows.is_empty(),
        "precondition: the fixture must produce at least one joined Docker row"
    );

    for row in docker_rows {
        for fact in row
            .evidence
            .iter()
            .filter(|e| e.kind == FactKind::Reclaimability)
        {
            assert!(
                !matches!(fact.source, EvidenceSource::FilesystemMetadata { .. }),
                "a {:?} row's reclaimability fact must name its real source (the Docker daemon), \
                 not a folded directory walk that never measured it: {fact:?}",
                row.kind
            );
        }
    }
}
