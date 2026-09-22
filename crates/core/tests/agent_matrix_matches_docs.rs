//! The support matrix in `docs/agent-storage.md` is parsed back and
//! compared with `agents::matrix::MATRIX` and with the adapter registry
//! (guardrail spec section 14, `agent_matrix_matches_docs`).
//!
//! This is an executable test rather than an AST audit on purpose: what
//! must not drift is a *published claim* against the code that backs it,
//! and the only way to check that is to read the published claim.
//!
//! The 2026-09-21 review's objection to the previous arrangement was
//! that the doc said "Supported" for every tool and the code agreed,
//! while several rows' own footnotes admitted the layout was assumed.
//! A doc and a constant agreeing is worth nothing if nothing checks the
//! agreement, and worth less than nothing if both are wrong. So this
//! test also asserts the two structural properties that make the level
//! mean something: an `unverified` row offers no action, and every
//! documented id is an id the registry actually has an adapter for.

use swamp_core::agents::matrix::{self, SupportLevel};
use swamp_core::agents::registry::Registry;

/// One parsed documentation row.
#[derive(Debug, PartialEq, Eq)]
struct DocRow {
    name: String,
    id: String,
    support: String,
    actions: String,
}

fn doc_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .join("docs/agent-storage.md")
}

/// Every row of the one table whose second column is a backticked id.
/// Deliberately strict: a row that does not parse is a row this test
/// would otherwise silently skip, so an unparseable row fails.
fn parse_doc_rows(text: &str) -> Vec<DocRow> {
    let mut rows = Vec::new();
    let mut in_matrix = false;
    for line in text.lines() {
        if line.starts_with("| Tool | Id | Support | Actions |") {
            in_matrix = true;
            continue;
        }
        if in_matrix {
            if line.starts_with("|---") {
                continue;
            }
            if !line.starts_with('|') {
                break;
            }
            let cells: Vec<&str> = line.trim_matches('|').split('|').collect();
            assert!(
                cells.len() >= 6,
                "matrix row has {} cells, expected at least 6: {line}",
                cells.len()
            );
            let id = cells[1].trim();
            let id = id
                .strip_prefix('`')
                .and_then(|s| s.strip_suffix('`'))
                .unwrap_or_else(|| {
                    panic!("matrix row's Id cell must be a backticked id, got {id:?}")
                });
            rows.push(DocRow {
                name: cells[0].trim().to_string(),
                id: id.to_string(),
                support: cells[2].trim().to_string(),
                actions: cells[3].trim().to_string(),
            });
        }
    }
    rows
}

fn doc_rows() -> Vec<DocRow> {
    let path = doc_path();
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let rows = parse_doc_rows(&text);
    assert!(
        !rows.is_empty(),
        "no support-matrix rows parsed out of {} -- the table's header line must stay \
         `| Tool | Id | Support | Actions | ... |` for this test to have anything to check",
        path.display()
    );
    rows
}

#[test]
fn the_documented_tool_ids_are_exactly_the_matrix_ids() {
    let mut documented: Vec<String> = doc_rows().into_iter().map(|r| r.id).collect();
    documented.sort();
    let mut expected: Vec<String> = matrix::MATRIX
        .iter()
        .map(|e| e.id.slug().to_string())
        .collect();
    expected.sort();
    assert_eq!(
        documented, expected,
        "docs/agent-storage.md's support matrix and agents::matrix::MATRIX list different tools"
    );
}

#[test]
fn the_documented_support_level_matches_the_matrix() {
    for row in doc_rows() {
        let entry = matrix::MATRIX
            .iter()
            .find(|e| e.id.slug() == row.id)
            .unwrap_or_else(|| panic!("docs name a tool the matrix does not: {}", row.id));
        assert_eq!(
            row.support,
            entry.support.label(),
            "{}: the doc says {:?} and the matrix says {:?}",
            row.id,
            row.support,
            entry.support.label()
        );
        assert_eq!(
            row.name, entry.display_name,
            "{}: display name differs between doc and matrix",
            row.id
        );
    }
}

#[test]
fn the_documented_actions_column_matches_the_support_level() {
    for row in doc_rows() {
        let entry = matrix::entry(
            matrix::MATRIX
                .iter()
                .find(|e| e.id.slug() == row.id)
                .expect("id checked above")
                .id,
        );
        let expected = if entry.support.actions_available() {
            "yes"
        } else {
            "no"
        };
        assert_eq!(
            row.actions,
            expected,
            "{}: the doc's Actions column says {:?} but support level {:?} means {expected:?}",
            row.id,
            row.actions,
            entry.support.label()
        );
    }
}

#[test]
fn an_unverified_row_shows_no_actions() {
    // The property that makes the level mean something. Stated
    // separately from the column check above so that removing the level
    // from a row cannot quietly remove this assertion with it.
    let rows = doc_rows();
    let unverified: Vec<&DocRow> = rows.iter().filter(|r| r.support == "unverified").collect();
    assert!(
        !unverified.is_empty(),
        "the unverified level must actually be in use in the published matrix, or the docs are \
         back to claiming every tool is supported"
    );
    for row in unverified {
        assert_eq!(
            row.actions, "no",
            "{} is documented unverified yet shows actions available",
            row.id
        );
    }
}

#[test]
fn every_documented_tool_has_a_registered_adapter() {
    let registry = Registry::with_builtins();
    let ids = registry.ids();
    for row in doc_rows() {
        assert!(
            ids.contains(&row.id.as_str()),
            "docs document {} with no adapter registered for it",
            row.id
        );
    }
    let mut registered: Vec<&str> = ids.clone();
    registered.sort_unstable();
    let mut documented: Vec<String> = doc_rows().into_iter().map(|r| r.id).collect();
    documented.sort();
    assert_eq!(
        registered,
        documented.iter().map(String::as_str).collect::<Vec<_>>(),
        "a registered adapter with no documentation row is an undocumented capability"
    );
}

#[test]
fn every_row_records_what_it_was_verified_against() {
    // A `Supported` row must cite something; an `Unverified` row must
    // record what was attempted. Both live in the doc's last column, and
    // the matrix's own `Verification` entries are checked by
    // `matrix::tests::a_supported_row_cites_what_confirmed_it`; this
    // asserts the doc carries it too, so a reader never has to open the
    // source to find out whether a claim was checked.
    let text = std::fs::read_to_string(doc_path()).expect("read doc");
    let mut in_matrix = false;
    let mut checked = 0usize;
    for line in text.lines() {
        if line.starts_with("| Tool | Id | Support | Actions |") {
            in_matrix = true;
            continue;
        }
        if in_matrix {
            if line.starts_with("|---") {
                continue;
            }
            if !line.starts_with('|') {
                break;
            }
            let cells: Vec<&str> = line.trim_matches('|').split('|').collect();
            let verified = cells[5].trim();
            let id = cells[1].trim();
            assert!(
                verified.len() > 30,
                "{id}: the Verified against column says only {verified:?}"
            );
            let support = cells[2].trim();
            if support == "unverified" {
                assert!(
                    verified.contains("NOT CONFIRMED") || verified.contains("PARTIALLY CONFIRMED"),
                    "{id} is unverified but its evidence column does not say what failed to \
                     confirm: {verified}"
                );
            }
            checked += 1;
        }
    }
    assert_eq!(checked, matrix::MATRIX.len());
}

/// An `Unverified` tool is identified and measured, and offers nothing.
///
/// The doc/constant comparisons above check that the *claim* is
/// consistent. This checks the behaviour the claim is about, through the
/// real `agents::discover_and_measure`, because "no action is offered"
/// is enforced in one shared place and a test of the adapter alone
/// would not see it: Cursor's adapter still *produces* a cache unit with
/// `CacheOrTrash`, and the shared layer is what withholds it.
#[test]
fn an_unverified_tools_units_are_measured_and_offer_nothing() {
    use swamp_core::agents::{AgentActionCapability, ProjectLinkState};
    use swamp_core::locations::{Environment, Platform, Registry as Detectors};
    use swamp_core::scope::{ScanConfig, resolve_effective_scope};

    let unverified: Vec<&str> = matrix::MATRIX
        .iter()
        .filter(|e| e.support == SupportLevel::Unverified)
        .map(|e| e.id.slug())
        .collect();
    assert!(
        unverified.contains(&"cursor"),
        "this test is written against Cursor; if its level changed, change the fixture too"
    );

    let tmp = tempfile::tempdir().unwrap();
    let home = std::fs::canonicalize(tmp.path()).unwrap();
    // A Cursor editor profile, plus a real checkout its workspace
    // metadata declares -- so linkage *would* resolve if the tool were
    // verified, which is what makes the assertion below meaningful
    // rather than vacuous.
    let profile = home.join("Library/Application Support/Cursor");
    let repo = home.join("declared-repo");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    for (rel, body) in [
        ("User/globalStorage/state.vscdb", b"sqlite".to_vec()),
        ("User/History/e1/snapshot", b"bytes".to_vec()),
        ("Cache/blob", vec![b'x'; 4096]),
    ] {
        let p = profile.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }
    let ws = profile.join("User/workspaceStorage/w1");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(
        ws.join("workspace.json"),
        format!("{{\"folder\":\"file://{}\"}}", repo.display()),
    )
    .unwrap();
    std::fs::write(ws.join("state.vscdb"), b"sqlite").unwrap();

    let detectors = Detectors::with_builtins();
    let scope = resolve_effective_scope(
        &Environment::fixture(
            home.clone(),
            std::collections::HashMap::new(),
            Platform::MacOS,
        ),
        &ScanConfig {
            defaults: false,
            include: Vec::new(),
            exclude: Vec::new(),
            disabled_detectors: Vec::new(),
            enabled_detectors: vec!["cursor".into()],
        },
        &[],
        &detectors,
        1_000,
    );
    let store = tempfile::tempdir().unwrap();
    let units = swamp_core::agents::discover_and_measure(
        &scope,
        &[],
        Some(store.path()),
        true,
        1_000,
        30,
        3600,
    )
    .expect("discovery");

    assert!(
        !units.is_empty(),
        "an unverified tool is still identified and measured -- withholding actions is not the \
         same as pretending the bytes are not there"
    );
    assert!(
        units.iter().any(|u| u.bytes > 0),
        "and the bytes are real: {:?}",
        units
            .iter()
            .map(|u| (u.relative_path.clone(), u.bytes))
            .collect::<Vec<_>>()
    );
    for u in &units {
        assert_eq!(
            u.action,
            AgentActionCapability::None,
            "{} offers {:?} for an unverified tool",
            u.relative_path,
            u.action
        );
        assert!(
            !matches!(u.project_link, ProjectLinkState::Linked { .. }),
            "{} claims a project link on an unverified layout: {:?}",
            u.relative_path,
            u.project_link
        );
        assert!(
            u.note.as_deref().is_some_and(|n| n.contains("unverified")),
            "{} does not say why it offers nothing: {:?}",
            u.relative_path,
            u.note
        );
    }

    // The control: the same fixture shape under a *verified* tool does
    // link and does offer an action, so the assertions above are about
    // the support level and not about this fixture being unactionable.
    let cline_home =
        home.join("Library/Application Support/Code/User/globalStorage/saoudrizwan.claude-dev");
    std::fs::create_dir_all(cline_home.join("tasks/t1")).unwrap();
    std::fs::write(cline_home.join("tasks/t1/ui_messages.json"), b"[]").unwrap();
    let cline_scope = resolve_effective_scope(
        &Environment::fixture(home, std::collections::HashMap::new(), Platform::MacOS),
        &ScanConfig {
            defaults: false,
            include: Vec::new(),
            exclude: Vec::new(),
            disabled_detectors: Vec::new(),
            enabled_detectors: vec!["cline".into()],
        },
        &[],
        &detectors,
        1_000,
    );
    let store2 = tempfile::tempdir().unwrap();
    let cline_units = swamp_core::agents::discover_and_measure(
        &cline_scope,
        &[],
        Some(store2.path()),
        true,
        1_000,
        30,
        3600,
    )
    .expect("discovery");
    assert!(
        cline_units
            .iter()
            .any(|u| u.action == AgentActionCapability::SessionRemoval),
        "a Supported tool must still offer its action: {:?}",
        cline_units
            .iter()
            .map(|u| (u.relative_path.clone(), u.action))
            .collect::<Vec<_>>()
    );
}
