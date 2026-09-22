//! Filesystem space, from the kernel rather than from `df`'s output.
//!
//! `df -k` was parsed by field index (`fields[3]` = "Available"), which
//! is a macOS layout assumption in two ways. GNU coreutils `df` prints a
//! different header and, on a long device name, *wraps the row onto a
//! second line* -- so `fields[3]` on Linux can be the capacity percentage
//! or nothing at all. It also honours `POSIXLY_CORRECT`, `BLOCK_SIZE` and
//! `DF_BLOCK_SIZE` from the environment, which a measurement must not
//! depend on. And every call was a subprocess spawn on a path that
//! already counts its spawns.
//!
//! `statvfs(3)` is POSIX, present on both targets with the same struct in
//! the `libc` crate, and answers directly. The one real difference is the
//! multiplier: POSIX defines `f_blocks`/`f_bfree`/`f_bavail` in units of
//! `f_frsize`, and Darwin's `statvfs` shim reports `f_frsize` too, so
//! `f_frsize` is correct on both -- with a fallback to `f_bsize` for the
//! filesystem that reports `f_frsize` as zero.
//!
//! Semantics, kept explicit because the two are not the same number:
//!
//! * **available** (`f_bavail`) is what an unprivileged process may
//!   actually use -- what `df`'s "Avail" column shows and what a "you
//!   would get this much back" statement must be measured against.
//! * **free** (`f_bfree`) includes blocks reserved for root. Reporting it
//!   as "free space" would overstate what a cleanup can recover.
//!
//! Only `available` is exposed: nothing in swamp has a use for the
//! reserved blocks, and offering both invites picking the flattering one.

use std::path::Path;

/// Bytes an unprivileged process can still write to the filesystem
/// holding `path`.
///
/// `None` when the filesystem cannot answer (the path does not exist,
/// permission is denied, the path contains a NUL byte, or the filesystem
/// reports a zero block size). A missing measurement stays missing: no
/// caller may substitute zero, and none may treat `None` as "plenty".
pub fn available_bytes(path: &Path) -> Option<u64> {
    let stat = statvfs(path)?;
    // POSIX: block counts are in f_frsize units. A filesystem that
    // reports f_frsize = 0 is answering with f_bsize instead.
    let unit = if stat.f_frsize > 0 {
        stat.f_frsize
    } else {
        stat.f_bsize
    };
    if unit == 0 {
        return None;
    }
    (stat.f_bavail as u64).checked_mul(unit as u64)
}

/// Total bytes the filesystem holding `path` can hold, for the "x of y"
/// shape a report uses when it has both numbers. Same `None` discipline
/// as [`available_bytes`].
pub fn total_bytes(path: &Path) -> Option<u64> {
    let stat = statvfs(path)?;
    let unit = if stat.f_frsize > 0 {
        stat.f_frsize
    } else {
        stat.f_bsize
    };
    if unit == 0 {
        return None;
    }
    (stat.f_blocks as u64).checked_mul(unit as u64)
}

fn statvfs(path: &Path) -> Option<libc::statvfs> {
    use std::os::unix::ffi::OsStrExt;
    let cpath = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut buf: std::mem::MaybeUninit<libc::statvfs> = std::mem::MaybeUninit::uninit();
    // SAFETY: `cpath` is a NUL-terminated C string that outlives the
    // call, and `buf` is a correctly sized, writable statvfs. The return
    // value is checked before `assume_init`.
    unsafe {
        if libc::statvfs(cpath.as_ptr(), buf.as_mut_ptr()) != 0 {
            return None;
        }
        Some(buf.assume_init())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_real_directory_answers_with_a_plausible_figure() {
        let tmp = tempfile::tempdir().unwrap();
        let available = available_bytes(tmp.path()).expect("statvfs on a temp dir");
        let total = total_bytes(tmp.path()).expect("statvfs on a temp dir");
        assert!(available > 0, "a writable temp dir has some space");
        assert!(
            available <= total,
            "available {available} cannot exceed total {total}"
        );
        // Sanity against an absurd unit mistake: no developer machine has
        // a filesystem smaller than a megabyte or larger than an exabyte.
        assert!(
            (1 << 20..1 << 60).contains(&total),
            "implausible total {total}"
        );
    }

    #[test]
    fn a_missing_path_is_none_not_zero() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(available_bytes(&tmp.path().join("does-not-exist")), None);
    }

    #[test]
    fn a_path_with_an_interior_nul_is_none_not_a_panic() {
        use std::os::unix::ffi::OsStrExt;
        let path = std::path::PathBuf::from(std::ffi::OsStr::from_bytes(b"/tmp/a\0b"));
        assert_eq!(available_bytes(&path), None);
    }

    /// The reason this module exists: the old implementation read
    /// `df -k`'s fourth whitespace-separated field, which is the
    /// available figure only on macOS's layout and only when the row does
    /// not wrap. The kernel answer must not depend on any of that.
    #[test]
    fn the_answer_does_not_depend_on_df_environment_variables() {
        let tmp = tempfile::tempdir().unwrap();
        let before = available_bytes(tmp.path()).unwrap();
        // SAFETY: single-threaded test-local mutation, restored below.
        unsafe {
            std::env::set_var("DF_BLOCK_SIZE", "1");
            std::env::set_var("POSIXLY_CORRECT", "1");
        }
        let after = available_bytes(tmp.path()).unwrap();
        unsafe {
            std::env::remove_var("DF_BLOCK_SIZE");
            std::env::remove_var("POSIXLY_CORRECT");
        }
        // Space genuinely moves between the two calls on a busy machine;
        // the unit must not. A block-size mistake changes the answer by
        // a factor of at least 512.
        let ratio = before.max(after) as f64 / before.min(after).max(1) as f64;
        assert!(ratio < 2.0, "before {before}, after {after}");
    }
}
