//! #85 on a real Linux kernel: swamp's freedesktop Trash backend, held
//! to the specification by an independent implementation of it.
//!
//! Every item is moved by `swamp_core::platform::trash` and then *found
//! and restored* through the `trash` crate's `os_limited::{list,
//! restore_all}` -- the same reading a desktop file manager does. A
//! record swamp wrote that another spec implementation cannot parse is
//! a Trash nobody can restore from, which is the failure this file
//! exists to catch.
//!
//! Fixtures are temporary directories; the home trash is redirected by
//! pointing `XDG_DATA_HOME` into the fixture, under one lock, because
//! the `trash` crate reads it from the process environment. Nothing here
//! touches the runner user's own Trash. The cross-device cases use
//! `/dev/shm` (tmpfs) against the fixture disk and skip, saying why,
//! where that is not a second filesystem.
#![cfg(target_os = "linux")]

mod fixture;

use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use swamp_core::platform::Os;
use swamp_core::platform::trash::{Envelope, Kind, Target, move_item};

static ENV: Mutex<()> = Mutex::new(());

fn uid() -> u32 {
    unsafe { libc::getuid() }
}

struct Fixture {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    data: PathBuf,
    target: Target,
}

fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    let data = root.join("xdg-data");
    let home = root.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let target = Target::for_os(Os::Linux, home, Some(data.clone()), uid());
    Fixture {
        _tmp: tmp,
        root,
        data,
        target,
    }
}

fn tree(at: &Path) {
    std::fs::create_dir_all(at.join("debug/deps")).unwrap();
    std::fs::write(at.join("debug/deps/libx.rlib"), vec![7u8; 5000]).unwrap();
    std::fs::write(
        at.join("CACHEDIR.TAG"),
        b"Signature: 8a477f597d28d172789f06886806bc55",
    )
    .unwrap();
}

/// Runs `f` with `XDG_DATA_HOME` pointed into the fixture, so the
/// `trash` crate's home-trash resolution reads the fixture's trash.
fn with_xdg<T>(data: &Path, f: impl FnOnce() -> T) -> T {
    let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let saved = std::env::var_os("XDG_DATA_HOME");
    unsafe { std::env::set_var("XDG_DATA_HOME", data) };
    let out = f();
    unsafe {
        match saved {
            Some(v) => std::env::set_var("XDG_DATA_HOME", v),
            None => std::env::remove_var("XDG_DATA_HOME"),
        }
    }
    out
}

fn listed(data: &Path, original: &Path) -> Vec<trash::TrashItem> {
    with_xdg(data, || {
        trash::os_limited::list()
            .unwrap()
            .into_iter()
            .filter(|i| i.original_path() == original)
            .collect()
    })
}

fn only_file(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.path()).collect())
        .unwrap_or_default()
}

/// The core promise: an item swamp moved is where a file manager looks,
/// with its original path and a deletion time, and restoring it through
/// a different implementation of the spec puts back exactly what left.
#[test]
fn a_trashed_tree_is_listed_and_restored_by_an_independent_spec_reader() {
    let fx = fixture();
    let src = fx.root.join("proj/target");
    tree(&src);
    let before = std::fs::read(src.join("debug/deps/libx.rlib")).unwrap();
    let t0 = swamp_core::entities::now();

    let moved = move_item(&src, &fx.target, "ignored-flat-name").unwrap();
    assert!(!src.exists(), "the tree moved");
    assert_eq!(moved.kind, Kind::FreedesktopHome);
    assert_eq!(moved.location, fx.data.join("Trash/files/target"));
    let info = moved
        .info
        .clone()
        .expect("a freedesktop move writes a .trashinfo");
    assert_eq!(info, fx.data.join("Trash/info/target.trashinfo"));
    let record = std::fs::read_to_string(&info).unwrap();
    assert!(record.starts_with("[Trash Info]\n"), "{record}");
    assert!(
        record.contains(&format!("Path={}\n", src.display())),
        "{record}"
    );

    let items = listed(&fx.data, &src);
    assert_eq!(
        items.len(),
        1,
        "the spec reader finds exactly one item for the original path"
    );
    let deleted = items[0].time_deleted as u64;
    assert!(
        deleted + 2 >= t0 && deleted <= swamp_core::entities::now() + 2,
        "deletion time {deleted} is when the move happened (t0 {t0})"
    );

    with_xdg(&fx.data, || trash::os_limited::restore_all(items).unwrap());
    assert_eq!(
        std::fs::read(src.join("debug/deps/libx.rlib")).unwrap(),
        before
    );
    assert!(!info.exists(), "restoring consumes the record");
}

/// Two items with one name get two names in the trash (`target`,
/// `target.2`) and two records; neither overwrites the other and both
/// restore to their own original path.
#[test]
fn name_collisions_get_distinct_items_and_both_restore() {
    let fx = fixture();
    let a = fx.root.join("a/target");
    let b = fx.root.join("b/target");
    tree(&a);
    tree(&b);
    std::fs::write(a.join("which"), b"a").unwrap();
    std::fs::write(b.join("which"), b"b").unwrap();
    let ma = move_item(&a, &fx.target, "x").unwrap();
    let mb = move_item(&b, &fx.target, "x").unwrap();
    assert_ne!(ma.location, mb.location);
    assert_eq!(mb.location, fx.data.join("Trash/files/target.2"));
    assert_eq!(std::fs::read(ma.location.join("which")).unwrap(), b"a");
    assert_eq!(std::fs::read(mb.location.join("which")).unwrap(), b"b");

    let mut items = listed(&fx.data, &a);
    items.extend(listed(&fx.data, &b));
    assert_eq!(items.len(), 2);
    with_xdg(&fx.data, || trash::os_limited::restore_all(items).unwrap());
    assert_eq!(std::fs::read(a.join("which")).unwrap(), b"a");
    assert_eq!(std::fs::read(b.join("which")).unwrap(), b"b");
}

/// A home trash that is a symlink to somewhere else is refused before
/// anything moves -- a `.Trash` pointing into another user's directory,
/// or out of the filesystem, is the attack the spec's rules exist for.
#[test]
fn a_symlinked_trash_is_refused_and_nothing_moves() {
    let fx = fixture();
    let elsewhere = fx.root.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::create_dir_all(&fx.data).unwrap();
    std::os::unix::fs::symlink(&elsewhere, fx.data.join("Trash")).unwrap();
    let src = fx.root.join("p/target");
    tree(&src);
    let err = move_item(&src, &fx.target, "x").unwrap_err().to_string();
    assert!(err.contains("symlink"), "{err}");
    assert!(
        src.join("debug/deps/libx.rlib").exists(),
        "the source did not move"
    );
    assert!(
        only_file(&elsewhere).is_empty(),
        "nothing landed at the symlink's target"
    );

    // The same for a symlinked `files/` inside a real trash.
    let fx = fixture();
    let trash_dir = fx.data.join("Trash");
    std::fs::create_dir_all(&trash_dir).unwrap();
    let sink = fx.root.join("sink");
    std::fs::create_dir_all(&sink).unwrap();
    std::os::unix::fs::symlink(&sink, trash_dir.join("files")).unwrap();
    let src = fx.root.join("q/target");
    tree(&src);
    let err = move_item(&src, &fx.target, "x").unwrap_err().to_string();
    assert!(err.contains("symlink"), "{err}");
    assert!(src.exists());
    assert!(only_file(&sink).is_empty());
}

/// A move that the kernel refuses part-way leaves the source where it
/// was and removes only the record swamp created -- no orphan
/// `.trashinfo` pointing at an item that is not in the trash.
#[test]
fn an_interrupted_move_keeps_the_source_and_leaves_no_orphan_record() {
    if uid() == 0 {
        eprintln!("SKIP an_interrupted_move_keeps_the_source: running as root ignores the mode");
        return;
    }
    let fx = fixture();
    let files = fx.data.join("Trash/files");
    std::fs::create_dir_all(&files).unwrap();
    std::fs::create_dir_all(fx.data.join("Trash/info")).unwrap();
    std::fs::set_permissions(&files, std::fs::Permissions::from_mode(0o500)).unwrap();
    let src = fx.root.join("p/target");
    tree(&src);
    let err = move_item(&src, &fx.target, "x").unwrap_err().to_string();
    std::fs::set_permissions(&files, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(err.contains("was not moved"), "{err}");
    assert!(src.join("debug/deps/libx.rlib").exists());
    assert!(
        only_file(&fx.data.join("Trash/info")).is_empty(),
        "the record for a move that did not happen was removed"
    );
}

/// The second filesystem the cross-device cases use, or why there is
/// none on this runner.
fn other_device(than: &Path) -> Result<PathBuf, String> {
    let shm = Path::new("/dev/shm");
    let m = std::fs::metadata(shm).map_err(|e| format!("/dev/shm unavailable: {e}"))?;
    if m.dev() == std::fs::metadata(than).unwrap().dev() {
        return Err("/dev/shm is on the same filesystem as the fixture".into());
    }
    let d = tempfile::Builder::new()
        .prefix("swamp-trash-xdev-")
        .tempdir_in(shm)
        .map_err(|e| format!("cannot create a directory in /dev/shm: {e}"))?;
    Ok(d.keep())
}

/// An item on another mount goes to *that mount's* trash, never copied
/// into the home trash; and when that mount's trash cannot be used the
/// move is refused -- no copy, no permanent fallback.
#[test]
fn another_mount_uses_its_own_trash_and_a_blocked_one_refuses_without_copying() {
    let fx = fixture();
    let other = match other_device(&fx.root) {
        Ok(p) => p,
        Err(why) => {
            eprintln!("SKIP another_mount_uses_its_own_trash: {why}");
            return;
        }
    };
    let src = other.join("target");
    tree(&src);
    // /dev/shm is a mount point: its top directory trash is
    // /dev/shm/.Trash-$uid. Only proceed if that is ours to use.
    let top_trash = PathBuf::from(format!("/dev/shm/.Trash-{}", uid()));
    let pre_existing = top_trash.exists();
    let moved = move_item(&src, &fx.target, "x");
    match moved {
        Ok(m) => {
            assert_eq!(m.kind, Kind::FreedesktopTopDirUser);
            assert!(
                m.location.starts_with(&top_trash),
                "{}",
                m.location.display()
            );
            assert_eq!(
                std::fs::metadata(&m.location).unwrap().dev(),
                std::fs::metadata(&other).unwrap().dev(),
                "the item stayed on its own filesystem: a rename, not a copy"
            );
            assert!(
                !fx.data.join("Trash/files/target").exists(),
                "nothing was copied into the home trash"
            );
            let _ = std::fs::remove_dir_all(&m.location);
            if let Some(i) = m.info {
                let _ = std::fs::remove_file(i);
            }
        }
        Err(e) => panic!("a writable other-mount trash must be used: {e:#}"),
    }
    if !pre_existing {
        let _ = std::fs::remove_dir_all(&top_trash);
    }

    // A Directory target on another filesystem than the item: EXDEV is
    // an error, not a copy.
    let src2 = other.join("target2");
    tree(&src2);
    let err = move_item(&src2, &Target::Directory(fx.root.join("T")), "t2")
        .unwrap_err()
        .to_string();
    assert!(err.contains("different filesystems"), "{err}");
    assert!(
        src2.join("debug/deps/libx.rlib").exists(),
        "the source stayed"
    );
    assert!(
        !fx.root.join("T/t2").exists(),
        "no partial copy was left behind"
    );

    // An envelope refuses before the first member moves.
    let err = Envelope::open(&src2, &Target::Directory(fx.root.join("T")), "env")
        .unwrap_err()
        .to_string();
    assert!(err.contains("cross-device"), "{err}");
    assert!(src2.exists());
    let _ = std::fs::remove_dir_all(&other);
}

/// A top-directory trash that exists but is not a directory we own (a
/// file, or a symlink planted by someone else) is refused; swamp does
/// not fall back to copying the item into the home trash.
#[test]
fn a_hostile_top_directory_trash_refuses_rather_than_copying_home() {
    let fx = fixture();
    let other = match other_device(&fx.root) {
        Ok(p) => p,
        Err(why) => {
            eprintln!("SKIP a_hostile_top_directory_trash: {why}");
            return;
        }
    };
    let top_trash = PathBuf::from(format!("/dev/shm/.Trash-{}", uid()));
    if top_trash.exists() || std::fs::symlink_metadata(&top_trash).is_ok() {
        eprintln!(
            "SKIP a_hostile_top_directory_trash: {} already exists",
            top_trash.display()
        );
        let _ = std::fs::remove_dir_all(&other);
        return;
    }
    let decoy = other.join("decoy");
    std::fs::create_dir_all(&decoy).unwrap();
    std::os::unix::fs::symlink(&decoy, &top_trash).unwrap();
    let src = other.join("target");
    tree(&src);
    let result = move_item(&src, &fx.target, "x");
    let _ = std::fs::remove_file(&top_trash);
    let err = result
        .expect_err("a symlinked .Trash-$uid must refuse")
        .to_string();
    assert!(err.contains("symlink"), "{err}");
    assert!(src.join("debug/deps/libx.rlib").exists());
    assert!(
        only_file(&decoy).is_empty(),
        "nothing went through the symlink"
    );
    assert!(
        !fx.data.join("Trash/files/target").exists(),
        "no copy into the home trash"
    );
    let _ = std::fs::remove_dir_all(&other);
}

/// The whole action path on the freedesktop target: propose -> approve
/// -> execute moves the unit into the fixture's system trash with a
/// record, and the ledger names both.
#[test]
fn execute_records_the_trash_location_and_its_restore_record() {
    let fx = fixture();
    let tmp = tempfile::tempdir().unwrap();
    let f = fixture::build(tmp.path());
    let store = tempfile::tempdir().unwrap();
    let r = swamp_core::report::report_full_mode(
        &f.root,
        None,
        false,
        Some(store.path()),
        Some("1h"),
        true,
        false,
        false,
        true,
    )
    .expect("report");
    let plan = swamp_core::actions::propose(&r, None, std::slice::from_ref(&f.target_dir), "test")
        .expect("plan");
    swamp_core::actions::save_plan(store.path(), &plan).unwrap();
    swamp_core::actions::approve(store.path(), &plan.id, "human:test").unwrap();
    let res = swamp_core::actions::execute_with_target(
        store.path(),
        &plan.id,
        "human:test",
        fx.target.clone(),
    )
    .unwrap();
    let outcome = &res.outcomes[0];
    assert_eq!(outcome.status, "completed", "{:?}", outcome.cause);
    let loc = outcome.recovery_location.clone().unwrap();
    assert!(
        loc.starts_with(fx.data.join("Trash/files")),
        "{}",
        loc.display()
    );
    assert!(!f.target_dir.exists());
    let ledger = std::fs::read_to_string(store.path().join("ledger.jsonl")).unwrap();
    assert!(ledger.contains("\"trash_info\""), "{ledger}");
    assert!(ledger.contains(".trashinfo"), "{ledger}");
    assert_eq!(listed(&fx.data, &f.target_dir).len(), 1);
}
