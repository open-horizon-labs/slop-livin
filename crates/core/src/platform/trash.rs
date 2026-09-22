//! Where a recoverable removal puts things, per platform.
//!
//! macOS has one answer: `~/.Trash`, which Finder shows and restores
//! from. Linux has a specification instead -- freedesktop.org's Trash
//! spec, which every desktop file manager implements -- and its answer
//! depends on which filesystem the item lives on:
//!
//! * **Same filesystem as `$XDG_DATA_HOME`**: the *home trash*,
//!   `$XDG_DATA_HOME/Trash` (default `~/.local/share/Trash`), with the
//!   item under `files/` and a `info/<name>.trashinfo` record beside it.
//! * **A different mount**: the *top directory trash* for that mount.
//!   The spec gives two methods, in order. An administrator-provided
//!   `$topdir/.Trash` may be used only if it has the **sticky bit** set
//!   and is **not a symlink**; otherwise the implementation uses
//!   `$topdir/.Trash-$uid` and creates it on demand.
//!
//! The cross-device rule is not a detail. Trashing into the home trash
//! from another mount would turn a rename into a whole-tree copy --
//! minutes and a second copy of the bytes for a large build directory --
//! and the spec exists precisely so a file manager can find the item
//! again afterwards.
//!
//! **Scope of this module (#84):** it resolves *where*. Writing the
//! `.trashinfo` record, performing the cross-device move and exposing
//! restore is #85. Until then a Linux build reports
//! [`crate::platform::Support::Planned`] for the `trash` capability and
//! `swamp` does not claim a freedesktop-compliant removal. Resolving the
//! location now is what lets #85 be a move implementation rather than a
//! second path-resolution design.

use crate::platform::Os;
use std::path::{Path, PathBuf};

/// The rule a build follows when picking a trash directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strategy {
    /// `~/.Trash`, what Finder reads.
    MacOsUserTrash,
    /// freedesktop.org Trash spec: home trash, or the mount's own
    /// `.Trash/$uid` / `.Trash-$uid`.
    Freedesktop,
}

impl Strategy {
    pub fn for_os(os: Os) -> Self {
        match os {
            Os::MacOs => Strategy::MacOsUserTrash,
            Os::Linux => Strategy::Freedesktop,
        }
    }
}

/// A resolved trash directory and why that one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub dir: PathBuf,
    pub kind: Kind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// macOS `~/.Trash`.
    MacOsUser,
    /// `$XDG_DATA_HOME/Trash` -- same filesystem, a rename.
    FreedesktopHome,
    /// `$topdir/.Trash/$uid` -- admin-provided, sticky, not a symlink.
    FreedesktopTopDirAdmin,
    /// `$topdir/.Trash-$uid` -- created on demand for this user.
    FreedesktopTopDirUser,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::MacOsUser => "macos_user_trash",
            Kind::FreedesktopHome => "freedesktop_home_trash",
            Kind::FreedesktopTopDirAdmin => "freedesktop_topdir_admin_trash",
            Kind::FreedesktopTopDirUser => "freedesktop_topdir_user_trash",
        }
    }
}

/// Everything the resolution needs, supplied by the caller so a test can
/// state a whole machine without one.
#[derive(Debug, Clone)]
pub struct Context {
    pub os: Os,
    pub home: PathBuf,
    /// `$XDG_DATA_HOME`, if set. Ignored when relative: the base
    /// directory spec requires absolute paths and says an implementation
    /// "should consider the path invalid and ignore it" otherwise.
    pub xdg_data_home: Option<PathBuf>,
    pub uid: u32,
}

impl Context {
    /// The home trash's parent, per the XDG base directory spec.
    fn data_home(&self) -> PathBuf {
        match &self.xdg_data_home {
            Some(p) if p.is_absolute() => p.clone(),
            _ => self.home.join(".local/share"),
        }
    }
}

/// The trash directory for an item at `item`, given what the caller
/// knows about the filesystems involved.
///
/// `item_device` and `home_device` are `st_dev` values; `top_dir` is the
/// mount point `item` lives on. All three are supplied rather than read
/// here so this stays a pure function a test can drive across mount
/// layouts that no CI runner has.
///
/// `admin_top_dir_trash` answers "does `$topdir/.Trash` exist, with the
/// sticky bit, and is not a symlink" -- the spec's method-1 check, which
/// the caller performs because it needs a live `lstat` and this does not.
pub fn resolve(
    ctx: &Context,
    item: &Path,
    item_device: u64,
    home_device: u64,
    top_dir: &Path,
    admin_top_dir_trash: bool,
) -> Location {
    let _ = item;
    match Strategy::for_os(ctx.os) {
        Strategy::MacOsUserTrash => Location {
            dir: ctx.home.join(".Trash"),
            kind: Kind::MacOsUser,
        },
        Strategy::Freedesktop => {
            if item_device == home_device {
                return Location {
                    dir: ctx.data_home().join("Trash"),
                    kind: Kind::FreedesktopHome,
                };
            }
            if admin_top_dir_trash {
                Location {
                    dir: top_dir.join(".Trash").join(ctx.uid.to_string()),
                    kind: Kind::FreedesktopTopDirAdmin,
                }
            } else {
                Location {
                    dir: top_dir.join(format!(".Trash-{}", ctx.uid)),
                    kind: Kind::FreedesktopTopDirUser,
                }
            }
        }
    }
}

/// The `files/` subdirectory an item is moved into, and the `info/`
/// subdirectory its `.trashinfo` record goes in. Present so #85 has one
/// spelling to use and this layout is asserted now.
pub fn files_dir(location: &Location) -> PathBuf {
    match location.kind {
        Kind::MacOsUser => location.dir.clone(),
        _ => location.dir.join("files"),
    }
}

pub fn info_dir(location: &Location) -> Option<PathBuf> {
    match location.kind {
        // macOS keeps its own restore metadata; swamp writes none.
        Kind::MacOsUser => None,
        _ => Some(location.dir.join("info")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linux_ctx() -> Context {
        Context {
            os: Os::Linux,
            home: PathBuf::from("/home/dev"),
            xdg_data_home: None,
            uid: 1000,
        }
    }

    #[test]
    fn macos_always_uses_the_user_trash() {
        let ctx = Context {
            os: Os::MacOs,
            home: PathBuf::from("/Users/dev"),
            xdg_data_home: Some(PathBuf::from("/Users/dev/.local/share")),
            uid: 501,
        };
        // Even across a device boundary: macOS's Trash handling is
        // Finder's, and swamp does not invent a second convention for it.
        let got = resolve(
            &ctx,
            Path::new("/Volumes/ext/x"),
            9,
            1,
            Path::new("/Volumes/ext"),
            true,
        );
        assert_eq!(got.dir, PathBuf::from("/Users/dev/.Trash"));
        assert_eq!(got.kind, Kind::MacOsUser);
        assert_eq!(info_dir(&got), None, "swamp writes no .trashinfo on macOS");
    }

    #[test]
    fn linux_same_filesystem_uses_the_home_trash() {
        let got = resolve(
            &linux_ctx(),
            Path::new("/home/dev/p/target"),
            1,
            1,
            Path::new("/"),
            false,
        );
        assert_eq!(got.dir, PathBuf::from("/home/dev/.local/share/Trash"));
        assert_eq!(got.kind, Kind::FreedesktopHome);
        assert_eq!(
            files_dir(&got),
            PathBuf::from("/home/dev/.local/share/Trash/files")
        );
        assert_eq!(
            info_dir(&got),
            Some(PathBuf::from("/home/dev/.local/share/Trash/info"))
        );
    }

    #[test]
    fn linux_honours_an_absolute_xdg_data_home() {
        let ctx = Context {
            xdg_data_home: Some(PathBuf::from("/data/dev")),
            ..linux_ctx()
        };
        let got = resolve(&ctx, Path::new("/home/dev/x"), 1, 1, Path::new("/"), false);
        assert_eq!(got.dir, PathBuf::from("/data/dev/Trash"));
    }

    /// The base directory spec: a relative value "should be considered
    /// invalid and ignored". Honouring it would put the trash somewhere
    /// relative to whatever the process's cwd happened to be.
    #[test]
    fn a_relative_xdg_data_home_is_ignored_not_joined_to_the_cwd() {
        let ctx = Context {
            xdg_data_home: Some(PathBuf::from("relative/share")),
            ..linux_ctx()
        };
        let got = resolve(&ctx, Path::new("/home/dev/x"), 1, 1, Path::new("/"), false);
        assert_eq!(got.dir, PathBuf::from("/home/dev/.local/share/Trash"));
    }

    /// The rule that keeps a cross-mount removal a rename instead of a
    /// whole-tree copy, and keeps the item where a file manager looks.
    #[test]
    fn linux_other_mount_uses_that_mounts_own_trash_never_the_home_trash() {
        let ctx = linux_ctx();
        let got = resolve(
            &ctx,
            Path::new("/mnt/build/target"),
            42,
            1,
            Path::new("/mnt/build"),
            false,
        );
        assert_eq!(got.dir, PathBuf::from("/mnt/build/.Trash-1000"));
        assert_eq!(got.kind, Kind::FreedesktopTopDirUser);
        assert!(
            !got.dir.starts_with("/home/dev"),
            "an item on another mount must not be trashed into the home trash"
        );
    }

    /// Method 1 only when the caller confirmed sticky-and-not-a-symlink.
    /// A writable, non-sticky `$topdir/.Trash` is exactly the shape the
    /// spec warns about, and falls through to method 2.
    #[test]
    fn an_admin_top_dir_trash_is_used_only_when_the_sticky_check_passed() {
        let ctx = linux_ctx();
        let sticky = resolve(
            &ctx,
            Path::new("/mnt/b/x"),
            42,
            1,
            Path::new("/mnt/b"),
            true,
        );
        assert_eq!(sticky.dir, PathBuf::from("/mnt/b/.Trash/1000"));
        assert_eq!(sticky.kind, Kind::FreedesktopTopDirAdmin);

        let not_sticky = resolve(
            &ctx,
            Path::new("/mnt/b/x"),
            42,
            1,
            Path::new("/mnt/b"),
            false,
        );
        assert_eq!(not_sticky.dir, PathBuf::from("/mnt/b/.Trash-1000"));
    }

    #[test]
    fn each_uid_gets_its_own_top_dir_trash() {
        let a = resolve(
            &linux_ctx(),
            Path::new("/mnt/b/x"),
            42,
            1,
            Path::new("/mnt/b"),
            false,
        );
        let b = resolve(
            &Context {
                uid: 1001,
                ..linux_ctx()
            },
            Path::new("/mnt/b/x"),
            42,
            1,
            Path::new("/mnt/b"),
            false,
        );
        assert_ne!(a.dir, b.dir);
    }
}
