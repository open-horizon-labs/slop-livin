//! #85 on a real Linux kernel: swamp's freedesktop Trash backend, held
//! to the specification by an independent implementation of it.
//!
//! Every item is moved by `fs_gate::destroy::trash_move`/`Envelope` --
//! the same proof-and-authorization-gated call the macOS backend uses
//! -- and then *found and restored* through the `trash` crate's
//! `os_limited::{list, restore_all}` -- the same reading a desktop file
//! manager does. A record swamp wrote that another spec implementation
//! cannot parse is a Trash nobody can restore from, which is the
//! failure this file exists to catch.
//!
//! Fixtures are temporary directories; the home trash is redirected by
//! pointing `XDG_DATA_HOME` into the fixture, under one lock, because
//! the `trash` crate reads it from the process environment. Nothing here
//! touches the runner user's own Trash.
#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use swamp_core::authority::{Confirmable, HumanConfirmed, SelectedUnit, authorize_confirmed};
use swamp_core::fs_gate::destroy::{self, Trashed};
use swamp_core::fs_gate::store::StoreDir;

static ENV: Mutex<()> = Mutex::new(());

struct Fixture {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    data: PathBuf,
    store_dir: StoreDir,
}

fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    let data = root.join("xdg-data");
    let store = root.join("store");
    std::fs::create_dir_all(&store).unwrap();
    Fixture {
        _tmp: tmp,
        root,
        data,
        store_dir: StoreDir::at(&store).unwrap(),
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

/// A live recheck proof and authorization for `path`, exactly as a
/// human's TUI confirmation would mint one
/// (`.oh/guardrails/human-only-authorization.md`).
fn authorize(store_dir: &StoreDir, path: &Path) -> swamp_core::authority::Authorized {
    let confirmed = HumanConfirmed::tui_dialog(
        "human:test",
        store_dir,
        vec![Confirmable::Unit(SelectedUnit {
            path: path.to_path_buf(),
            reviewed: swamp_core::recheck::capture_anchor(path).ok(),
            docker: None,
            linked_worktree: false,
            preserve_into: None,
        })],
    )
    .remove(0);
    authorize_confirmed(confirmed, path).unwrap()
}

fn trash_move(
    store_dir: &StoreDir,
    path: &Path,
    trash_root: &Path,
    name: &str,
) -> anyhow::Result<Trashed> {
    let auth = authorize(store_dir, path);
    let proof = swamp_core::recheck::run_all(&auth)?;
    destroy::trash_move(proof, &auth, trash_root, name)
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

    let trash_root = fx.data.join("Trash");
    let moved = trash_move(&fx.store_dir, &src, &trash_root, "target").unwrap();
    assert!(!src.exists(), "the tree moved");
    assert_eq!(moved.path(), trash_root.join("files/target"));
    let info = trash_root.join("info/target.trashinfo");
    let record = std::fs::read_to_string(&info).expect("a freedesktop move writes a .trashinfo");
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

/// A destination name already in the Trash is refused outright: this
/// gate's `trash_move` has no freedesktop-style `name.2`/`name.3`
/// disambiguation, so a caller must pick a name nothing already holds
/// (`actions.rs`'s callers append a plan/session id for exactly this
/// reason).
#[test]
fn a_dest_name_already_in_the_trash_is_refused_not_overwritten() {
    let fx = fixture();
    let a = fx.root.join("a/target");
    let b = fx.root.join("b/target");
    tree(&a);
    tree(&b);
    let trash_root = fx.data.join("Trash");

    trash_move(&fx.store_dir, &a, &trash_root, "target").unwrap();
    let err = trash_move(&fx.store_dir, &b, &trash_root, "target")
        .unwrap_err()
        .to_string();
    assert!(err.contains("already exists"), "{err}");
    assert!(b.exists(), "the second unit was never moved");
}

/// A Trash root on another filesystem than the fixture's own is
/// refused, never silently copied: a single-file `trash_move` has no
/// per-mount fallback and fails on the kernel's own `EXDEV` from
/// `rename(2)` -- a refusal, never a copy.
#[test]
fn a_cross_device_trash_root_is_refused_not_copied() {
    use std::os::unix::fs::MetadataExt;
    let fx = fixture();
    let shm = Path::new("/dev/shm");
    let Ok(shm_meta) = std::fs::metadata(shm) else {
        eprintln!("SKIP a_cross_device_trash_root_is_refused_not_copied: /dev/shm unavailable");
        return;
    };
    if shm_meta.dev() == std::fs::metadata(&fx.root).unwrap().dev() {
        eprintln!(
            "SKIP a_cross_device_trash_root_is_refused_not_copied: /dev/shm is on the fixture's own filesystem"
        );
        return;
    }
    let other_trash = tempfile::Builder::new()
        .prefix("swamp-trash-xdev-")
        .tempdir_in(shm)
        .unwrap();

    let src = fx.root.join("p/target");
    tree(&src);
    let auth = authorize(&fx.store_dir, &src);
    let proof = swamp_core::recheck::run_all(&auth).unwrap();
    let err = swamp_core::fs_gate::destroy::trash_move(proof, &auth, other_trash.path(), "target")
        .unwrap_err()
        .to_string();
    // The kernel's own EXDEV, surfaced rather than papered over with a
    // copy; the unit must still be exactly where it was.
    assert!(err.to_lowercase().contains("cross-device link"), "{err}");
    assert!(src.join("debug/deps/libx.rlib").exists());
    assert!(
        std::fs::read_dir(other_trash.path())
            .unwrap()
            .next()
            .is_none(),
        "nothing was copied into the other filesystem's trash"
    );
}

/// A multi-member unit (a Cargo group, an agent session) goes into one
/// envelope, member by member; `Envelope::open`'s own `same_device_as`
/// refuses before anything moves when the trash is on another
/// filesystem, and every member move is a plain rename.
#[test]
fn a_multi_member_envelope_moves_every_member_and_gets_one_sidecar() {
    use std::os::unix::fs::MetadataExt;
    let fx = fixture();
    let session = fx.root.join("session");
    std::fs::create_dir_all(&session).unwrap();
    let members = [session.join("transcript.jsonl"), session.join("meta.json")];
    for m in &members {
        std::fs::write(m, b"fixture content").unwrap();
    }
    let trash_root = fx.data.join("Trash");

    let auth = authorize(&fx.store_dir, &session);
    let proof = swamp_core::recheck::run_all(&auth).unwrap();
    let dev = std::fs::metadata(&session).unwrap().dev();
    let mut envelope =
        destroy::Envelope::open(proof, &auth, &trash_root, "session-1", Some(dev)).unwrap();
    for (i, m) in members.iter().enumerate() {
        envelope.move_member(m, &i.to_string()).unwrap();
    }
    for m in &members {
        assert!(!m.exists());
    }
    assert!(envelope.path().join("0").exists());
    assert!(envelope.path().join("1").exists());
    assert_eq!(envelope.path(), trash_root.join("files/session-1"));
    assert!(
        trash_root.join("info/session-1.trashinfo").exists(),
        "the envelope itself is the trashed item and gets one sidecar"
    );
}
