//! Root-scoped persistence adversaries.
//!
//! These tests intentionally put all roots on one temporary volume: a
//! device-only store key is the tempting local fix that lets one root clobber
//! another while every single-root test still passes.

use std::fs;
use swamp_core::growth::{root_scoped_volume_id, volume_store_dir};

#[test]
fn canonical_aliases_share_a_store_but_siblings_do_not() {
    let tmp = tempfile::tempdir().expect("temporary volume");
    let first = tmp.path().join("parent");
    let sibling = tmp.path().join("sibling");
    fs::create_dir_all(&first).expect("parent root");
    fs::create_dir_all(&sibling).expect("sibling root");

    #[cfg(unix)]
    std::os::unix::fs::symlink(&first, tmp.path().join("parent-alias")).expect("root alias");

    #[cfg(unix)]
    {
        let alias = tmp.path().join("parent-alias");
        assert_eq!(
            root_scoped_volume_id(&first),
            root_scoped_volume_id(&alias),
            "aliases of one root must replay the same history"
        );
    }
    assert_ne!(
        root_scoped_volume_id(&first),
        root_scoped_volume_id(&sibling),
        "sibling roots on one device must not share current or deltas"
    );

    let store = tmp.path().join("store");
    assert_eq!(
        volume_store_dir(&store, &first),
        volume_store_dir(&store, &first),
        "same-root reads and writes must remain stable"
    );
    assert_ne!(
        volume_store_dir(&store, &first),
        volume_store_dir(&store, &sibling),
        "report/propose must resolve different roots to different stores"
    );
}

#[cfg(unix)]
#[test]
fn non_utf8_root_names_do_not_collapse_to_one_scope() {
    let tmp = tempfile::tempdir().expect("temporary volume");
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let first = tmp.path().join(OsString::from_vec(b"root-\xff".to_vec()));
    let second = tmp.path().join(OsString::from_vec(b"root-\xfe".to_vec()));
    // The macOS filesystem may reject non-UTF-8 names; the scope helper
    // still hashes the raw fallback path when the root is not yet present.
    assert_ne!(
        root_scoped_volume_id(&first),
        root_scoped_volume_id(&second),
        "root scope hashing must use raw path bytes, not lossy UTF-8"
    );
}
