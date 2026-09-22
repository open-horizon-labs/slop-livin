use std::{collections::HashMap, fs};
use swamp_core::{actions, agents, external, scope::{ScanConfig, resolve_effective_scope}, locations::{Environment, Platform, Registry}};

fn only_claude() -> ScanConfig {
    ScanConfig { defaults:false, disabled_detectors:Registry::with_builtins().detectors().iter().filter(|d| d.id() != "claude-code").map(|d| d.id().to_string()).collect(), ..Default::default() }
}

fn fixture() -> (tempfile::TempDir, std::path::PathBuf, swamp_core::scope::EffectiveScope) {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("claude");
    fs::create_dir_all(home.join("debug")).unwrap();
    fs::write(home.join("debug/log.txt"), b"original").unwrap();
    let env = Environment::fixture(tmp.path().to_owned(), HashMap::from([("CLAUDE_CONFIG_DIR".into(), home.display().to_string())]), Platform::MacOS);
    let scope = resolve_effective_scope(&env, &only_claude(), &[], &Registry::with_builtins(), 1000);
    (tmp, home, scope)
}

#[test]
fn unchanged_combined_observation_must_not_invent_regrowth() {
    let (_tmp, _home, scope) = fixture();
    let store = tempfile::tempdir().unwrap();
    for t in [1000,2000] {
        let ext = external::discover_and_measure(&scope, Some(store.path()), true, t, 30, 3600).unwrap();
        let units = agents::discover_and_measure(&scope, &[], Some(store.path()), true, t, 30, 3600).unwrap();
        assert!(ext.iter().all(|u| u.regrowth_count == 0), "unchanged external regrowth: {:?}",ext.iter().map(|u| (&u.path,u.regrowth_count)).collect::<Vec<_>>());
        assert!(units.iter().all(|u| u.regrowth_count == 0), "unchanged agent regrowth: {:?}",units.iter().map(|u| (&u.path,u.regrowth_count)).collect::<Vec<_>>());
    }
}

#[test]
fn protection_added_after_approval_must_stop_execution() {
    let (_tmp, home, scope) = fixture();
    let store = tempfile::tempdir().unwrap();
    let trash = tempfile::tempdir().unwrap();
    let units = agents::discover_and_measure(&scope, &[], Some(store.path()), false, 1000, 30, 3600).unwrap();
    let path=home.join("debug");
    let plan=actions::propose_agents(&units,&[path.clone()],"review-fixture").unwrap();
    actions::save_plan(store.path(),&plan).unwrap();
    actions::approve(store.path(),&plan.id,"human:fixture").unwrap();
    agents::protect_add(store.path(),&path).unwrap();
    let result=actions::execute_with_trash(store.path(),&plan.id,"human:fixture",trash.path()).unwrap();
    assert!(path.exists(), "protected data was moved: {:?}",result.outcomes);
}

#[test]
fn replacement_directory_must_not_spend_old_approval() {
    let (_tmp, home, scope) = fixture();
    let store = tempfile::tempdir().unwrap();
    let trash = tempfile::tempdir().unwrap();
    let units = agents::discover_and_measure(&scope, &[], None, false, 1000, 30, 3600).unwrap();
    let path=home.join("debug");
    let plan=actions::propose_agents(&units,&[path.clone()],"review-fixture").unwrap();
    actions::save_plan(store.path(),&plan).unwrap();
    actions::approve(store.path(),&plan.id,"human:fixture").unwrap();
    fs::rename(&path, home.join("old-debug")).unwrap();
    fs::create_dir(&path).unwrap();
    fs::write(path.join("unapproved.txt"),b"new unrelated content").unwrap();
    let result=actions::execute_with_trash(store.path(),&plan.id,"human:fixture",trash.path()).unwrap();
    assert!(path.exists(), "replacement data was moved: {:?}",result.outcomes);
}

#[test]
fn excluded_agent_home_must_not_be_scanned() {
    let (tmp, home, _) = fixture();
    let env = Environment::fixture(tmp.path().to_owned(), HashMap::from([("CLAUDE_CONFIG_DIR".into(), home.display().to_string())]), Platform::MacOS);
    let scope = resolve_effective_scope(&env, &ScanConfig { exclude:vec![home.display().to_string()], ..only_claude() }, &[], &Registry::with_builtins(), 1000);
    let units = agents::discover_and_measure(&scope, &[], None, false, 1000, 30, 3600).unwrap();
    let ext = external::discover_and_measure(&scope, None, false, 1000, 30, 3600).unwrap();
    assert!(units.is_empty() && ext.is_empty(), "excluded home scanned: {} agent units, {} external units",units.len(),ext.len());
}

#[test]
fn protected_descendant_must_prevent_parent_cache_proposal() {
    let (_tmp, home, scope) = fixture();
    let store = tempfile::tempdir().unwrap();
    agents::protect_add(store.path(),&home.join("debug/log.txt")).unwrap();
    let units=agents::discover_and_measure(&scope,&[],Some(store.path()),false,1000,30,3600).unwrap();
    assert!(actions::propose_agents(&units,&[home.join("debug")],"review-fixture").is_err(), "parent of protected file remained actionable");
}

#[test]
fn defaults_false_must_mean_explicit_only() {
    let (tmp, home, _) = fixture();
    let env = Environment::fixture(tmp.path().to_owned(), HashMap::from([("CLAUDE_CONFIG_DIR".into(), home.display().to_string())]), Platform::MacOS);
    let scope=resolve_effective_scope(&env,&ScanConfig {defaults:false,..Default::default()},&[],&Registry::with_builtins(),1000);
    assert!(scope.roots.is_empty(), "defaults=false still inferred {} roots",scope.roots.len());
}

#[test]
fn open_cache_member_must_stop_parent_removal() {
    let (_tmp, home, scope) = fixture();
    let store=tempfile::tempdir().unwrap();
    let trash=tempfile::tempdir().unwrap();
    let path=home.join("debug");
    let units=agents::discover_and_measure(&scope,&[],None,false,1000,30,3600).unwrap();
    let plan=actions::propose_agents(&units,&[path.clone()],"review-fixture").unwrap();
    actions::save_plan(store.path(),&plan).unwrap();
    actions::approve(store.path(),&plan.id,"human:fixture").unwrap();
    let _open=fs::File::open(path.join("log.txt")).unwrap();
    assert!(agents::is_active(&path.join("log.txt")), "fixture must have an observable open file");
    let result=actions::execute_with_trash(store.path(),&plan.id,"human:fixture",trash.path()).unwrap();
    assert!(path.exists(), "actively open member moved: {:?}",result.outcomes);
}
