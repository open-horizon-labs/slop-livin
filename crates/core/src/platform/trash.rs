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
//! **This module is the one place swamp moves anything into a Trash**
//! (#85). Every recoverable action -- the CLI's `execute`, the TUI's
//! confirm, Cargo's grouped removal, agent-storage cache and session
//! removal -- calls [`move_item`] or [`Envelope`] here after its own
//! live-state rechecks, and none of them renames into a trash directory
//! itself (`trash_backend_owns_every_move` holds that).
//!
//! Three rules, each one a way a Trash implementation loses data:
//!
//! 1. **A rename or nothing.** The backend never copies and never
//!    deletes a source. Where the kernel cannot move an item with
//!    `rename(2)` -- the trash is on another filesystem, a directory is
//!    not writable -- the move fails and the source is where it was.
//!    There is no copy-then-delete fallback and no permanent-deletion
//!    fallback, on either platform.
//! 2. **Unknown recovery refuses.** If the location a removal would go
//!    to cannot be established -- no home directory, a trash directory
//!    that is a symlink or owned by someone else, a mount whose top
//!    directory trash cannot be created -- the item is not moved.
//! 3. **The record is written first.** On Linux the `.trashinfo`
//!    restore record (original path + deletion time) is created with
//!    `O_EXCL` *before* the move, which also reserves the name; a move
//!    that then fails removes only that record, never the item.
//!
//! ## Why not `trash::delete`
//!
//! Part 1 recommended adopting the `trash` crate (5.2.9, MIT) for the
//! move. Reading its 5.2.9 source for this work changed that for the
//! move itself, for four reasons recorded in `docs/platform.md`:
//! `move_items_no_replace` falls back to **copy + `remove_dir_all`** on
//! `EXDEV` (a whole-tree copy that expands sparse files and splits
//! hardlinks, and is not atomic); a per-volume trash that is not writable
//! falls back to the **home trash across devices**, which is that same
//! copy; `delete` returns `()`, so the ledger could not record where an
//! item went; and its home-trash resolution honours a relative
//! `$XDG_DATA_HOME`, which the base-directory spec says to ignore. The
//! crate stays in the build as the independent reader: the Linux tests
//! list and restore swamp's items through `trash::os_limited`, which is
//! what proves a file manager can find and restore them.
//!
//! macOS keeps what it always did, byte for byte: a rename into
//! `~/.Trash` under swamp's own name, where Finder shows it. An explicit
//! directory (`SWAMP_TRASH_DIR`, or a test's temporary directory) is a
//! [`Target::Directory`] on both platforms and behaves the same way --
//! it is not the system Trash, and nothing claims it is.

use crate::platform::Os;
use anyhow::{Context as _, Result, anyhow, bail};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
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
#[derive(Debug, Clone, PartialEq, Eq)]
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
/// subdirectory its `.trashinfo` record goes in.
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

// ---------------------------------------------------------------------
// The move (#85)
// ---------------------------------------------------------------------

/// Where a recoverable removal goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A plain directory: the item is renamed into it under the name the
    /// caller chose. macOS's `~/.Trash` (Finder keeps its own restore
    /// metadata, swamp writes none), and every explicit override --
    /// `SWAMP_TRASH_DIR`, a test's temporary directory -- on both
    /// platforms.
    Directory(PathBuf),
    /// The freedesktop.org Trash specification: the home trash for an
    /// item on the same filesystem as `$XDG_DATA_HOME`, that mount's own
    /// top-directory trash otherwise, with a `.trashinfo` record.
    Freedesktop(Context),
}

impl Target {
    /// The Trash a real action uses on this platform.
    ///
    /// `SWAMP_TRASH_DIR` wins on both platforms and is a plain
    /// directory. Otherwise `~/.Trash` on macOS and the freedesktop
    /// system trash on Linux. `Err` -- never a guess -- when there is no
    /// home directory to resolve either from: an action that cannot say
    /// where its item will be is refused, not performed.
    pub fn system() -> Result<Target> {
        if let Some(dir) = std::env::var_os("SWAMP_TRASH_DIR") {
            return Ok(Target::Directory(PathBuf::from(dir)));
        }
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|h| h.is_absolute())
            .ok_or_else(|| {
                anyhow!(
                    "HOME is not set (or is not absolute), so there is no Trash to move into; \
                     nothing was moved. Set HOME, or SWAMP_TRASH_DIR to an explicit directory."
                )
            })?;
        Ok(Self::for_os(
            Os::current(),
            home,
            std::env::var_os("XDG_DATA_HOME").map(PathBuf::from),
            current_uid(),
        ))
    }

    /// [`Target::system`] for a stated machine, so a test on either
    /// platform can ask what the other one does.
    pub fn for_os(os: Os, home: PathBuf, xdg_data_home: Option<PathBuf>, uid: u32) -> Target {
        match Strategy::for_os(os) {
            Strategy::MacOsUserTrash => Target::Directory(home.join(".Trash")),
            Strategy::Freedesktop => Target::Freedesktop(Context {
                os,
                home,
                xdg_data_home,
                uid,
            }),
        }
    }

    /// A path on the filesystem the free-space figures are read from.
    pub fn probe_path(&self) -> PathBuf {
        match self {
            Target::Directory(d) => d.clone(),
            Target::Freedesktop(ctx) => ctx.data_home(),
        }
    }

    /// One line for a confirm banner or an error: where things go.
    pub fn describe(&self) -> String {
        match self {
            Target::Directory(d) => d.display().to_string(),
            Target::Freedesktop(ctx) => format!(
                "the freedesktop Trash ({} for this filesystem, or the mount's own \
                 .Trash-{} on another one)",
                ctx.data_home().join("Trash").display(),
                ctx.uid
            ),
        }
    }
}

/// What a completed move produced: where the item is now and how to get
/// it back. Every field is what the ledger records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trashed {
    /// The item's path inside the trash.
    pub location: PathBuf,
    /// The `.trashinfo` restore record (freedesktop only).
    pub info: Option<PathBuf>,
    pub kind: Kind,
    /// The absolute path the item had before the move.
    pub original: PathBuf,
}

fn current_uid() -> u32 {
    // SAFETY: getuid() cannot fail and reads no caller-owned memory.
    unsafe { libc::getuid() }
}

/// Moves `src` into the trash. `flat_name` is the name a
/// [`Target::Directory`] uses (swamp's historical naming, unchanged);
/// the freedesktop trash uses the item's own basename, as every file
/// manager does, and resolves collisions the spec's way.
///
/// On any `Err` the source has not moved. See the module docs for the
/// three rules this keeps.
pub fn move_item(src: &Path, target: &Target, flat_name: &str) -> Result<Trashed> {
    let meta = std::fs::symlink_metadata(src)
        .with_context(|| format!("{} no longer exists; nothing was moved", src.display()))?;
    match target {
        Target::Directory(dir) => {
            std::fs::create_dir_all(dir)?;
            let dest = dir.join(flat_name);
            rename_no_replace(src, &dest)?;
            Ok(Trashed {
                location: dest,
                info: None,
                kind: Kind::MacOsUser,
                original: src.to_path_buf(),
            })
        }
        Target::Freedesktop(ctx) => {
            let resolved = resolve_live(ctx, src, meta.dev())?;
            let base = src
                .file_name()
                .ok_or_else(|| anyhow!("{} has no file name to trash under", src.display()))?;
            let reserved = reserve(&resolved, &base.to_string_lossy(), src)?;
            if let Err(e) = rename_no_replace(src, &reserved.file) {
                // The record is the only thing this call created; the
                // source never moved.
                let _unused = std::fs::remove_file(&reserved.info);
                return Err(e);
            }
            Ok(Trashed {
                location: reserved.file,
                info: Some(reserved.info),
                kind: resolved.location.kind,
                original: src.to_path_buf(),
            })
        }
    }
}

/// A directory in the trash that several members are moved into as one
/// recoverable unit (a Cargo group, an agent session), with swamp's own
/// `restore.json` manifest beside them.
///
/// Relation to the system Trash: on macOS the envelope is one item in
/// `~/.Trash`. On Linux it is one freedesktop trash item whose
/// `.trashinfo` `Path` is `<the first member's directory>/<envelope
/// name>`, so a file manager's "Restore" puts the envelope back *beside*
/// the members' original location, and `restore.json` inside it says
/// where each member goes. The spec has one original path per item; a
/// group of members has several, so this is the honest mapping.
#[derive(Debug)]
pub struct Envelope {
    dir: PathBuf,
    info: Option<PathBuf>,
    kind: Kind,
    device: u64,
    moved: Vec<(PathBuf, PathBuf)>,
}

impl Envelope {
    /// Creates the envelope for members living beside `near`. Refuses
    /// before anything moves when the envelope would not be on the
    /// members' filesystem: every later move is a rename or nothing.
    pub fn open(near: &Path, target: &Target, name: &str) -> Result<Envelope> {
        let dev = std::fs::symlink_metadata(near)
            .with_context(|| format!("{} no longer exists; nothing was moved", near.display()))?
            .dev();
        match target {
            Target::Directory(dir) => {
                std::fs::create_dir_all(dir)?;
                if std::fs::metadata(dir)?.dev() != dev {
                    bail!(
                        "cross-device Trash unsupported: {} is on another filesystem than {}; \
                         no copy and no permanent fallback, nothing was moved",
                        dir.display(),
                        near.display()
                    );
                }
                let env = dir.join(name);
                // `create_dir_all`, as the session removal always did: an
                // envelope name is unique per plan and timestamp.
                std::fs::create_dir_all(&env).with_context(|| {
                    format!("could not create Trash envelope {}", env.display())
                })?;
                Ok(Envelope {
                    dir: env,
                    info: None,
                    kind: Kind::MacOsUser,
                    device: dev,
                    moved: Vec::new(),
                })
            }
            Target::Freedesktop(ctx) => {
                let resolved = resolve_live(ctx, near, dev)?;
                let original = near.parent().unwrap_or(near).join(name);
                let reserved = reserve(&resolved, name, &original)?;
                if let Err(e) = std::fs::create_dir(&reserved.file) {
                    let _unused = std::fs::remove_file(&reserved.info);
                    return Err(anyhow!(
                        "could not create Trash envelope {}: {e}",
                        reserved.file.display()
                    ));
                }
                Ok(Envelope {
                    dir: reserved.file,
                    info: Some(reserved.info),
                    kind: resolved.location.kind,
                    device: dev,
                    moved: Vec::new(),
                })
            }
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn info(&self) -> Option<&Path> {
        self.info.as_deref()
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// Moves one member into the envelope under `name_in_envelope`.
    /// A member on another filesystem is refused, not copied.
    pub fn move_member(&mut self, src: &Path, name_in_envelope: &str) -> Result<PathBuf> {
        let dev = std::fs::symlink_metadata(src)
            .with_context(|| format!("member no longer exists: {}", src.display()))?
            .dev();
        if dev != self.device {
            bail!(
                "{} is on another filesystem than its Trash envelope; no copy and no permanent \
                 fallback",
                src.display()
            );
        }
        let to = self.dir.join(name_in_envelope);
        rename_no_replace(src, &to)?;
        self.moved.push((src.to_path_buf(), to.clone()));
        Ok(to)
    }

    /// Puts every member moved so far back where it came from, newest
    /// first, and says what could not be put back. A member whose
    /// original path has been re-created is left in the envelope rather
    /// than overwriting what is there now.
    pub fn rollback(&mut self) -> Vec<String> {
        let mut failures = Vec::new();
        while let Some((from, to)) = self.moved.pop() {
            if std::fs::symlink_metadata(&from).is_ok() {
                failures.push(format!("{} reappeared", from.display()));
                continue;
            }
            if let Err(e) = rename_no_replace(&to, &from) {
                failures.push(e.to_string());
            }
        }
        failures
    }
}

/// `rename(2)` that refuses to replace an existing destination: on
/// Linux `renameat2(RENAME_NOREPLACE)` (atomic); elsewhere, and on a
/// Linux filesystem without that flag, an existence check first. Never
/// a copy: `EXDEV` is returned as the error it is.
fn rename_no_replace(from: &Path, to: &Path) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::ffi::OsStrExt;
        let (Ok(f), Ok(t)) = (
            std::ffi::CString::new(from.as_os_str().as_bytes()),
            std::ffi::CString::new(to.as_os_str().as_bytes()),
        ) else {
            bail!("path contains a NUL byte");
        };
        // SAFETY: both are valid NUL-terminated strings for the call.
        let rc = unsafe {
            libc::renameat2(
                libc::AT_FDCWD,
                f.as_ptr(),
                libc::AT_FDCWD,
                t.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if rc == 0 {
            return Ok(());
        }
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::EEXIST) {
            bail!(
                "{} already exists in the Trash; nothing was moved",
                to.display()
            );
        }
        if err.raw_os_error() != Some(libc::EINVAL) {
            return Err(move_error(from, to, err));
        }
    }
    if std::fs::symlink_metadata(to).is_ok() {
        bail!(
            "{} already exists in the Trash; nothing was moved",
            to.display()
        );
    }
    std::fs::rename(from, to).map_err(|e| move_error(from, to, e))
}

fn move_error(from: &Path, to: &Path, e: std::io::Error) -> anyhow::Error {
    if e.raw_os_error() == Some(libc::EXDEV) {
        anyhow!(
            "rename to Trash failed: {} and {} are on different filesystems. swamp does not \
             copy into a Trash and has no permanent-deletion fallback; nothing was moved",
            from.display(),
            to.display()
        )
    } else {
        anyhow!(
            "rename to Trash failed: {e}; {} was not moved",
            from.display()
        )
    }
}

/// A trash directory resolved against the live filesystem, with its
/// `files/` and `info/` directories checked.
#[derive(Debug)]
struct Resolved {
    location: Location,
    files: PathBuf,
    info: PathBuf,
}

/// [`resolve`] with the `lstat`s it leaves to its caller, and every
/// directory it will write into checked for the shapes a hostile or
/// broken trash takes: a symlink (a `.Trash` pointing somewhere else),
/// another user's directory, a directory on a different filesystem than
/// the item.
fn resolve_live(ctx: &Context, item: &Path, item_dev: u64) -> Result<Resolved> {
    let home_trash = ctx.data_home().join("Trash");
    let home_dev = nearest_existing_dev(&home_trash)
        .ok_or_else(|| anyhow!("cannot stat any ancestor of {}", home_trash.display()))?;
    let top = mount_top(item, item_dev);
    let admin = top.join(".Trash");
    let admin_ok = std::fs::symlink_metadata(&admin).is_ok_and(|m| {
        m.file_type().is_dir()
            && !m.file_type().is_symlink()
            && m.permissions().mode() & 0o1000 != 0
    });
    let location = resolve(ctx, item, item_dev, home_dev, &top, admin_ok);
    let dir = &location.dir;
    match location.kind {
        Kind::FreedesktopHome => {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("could not create the home trash {}", dir.display()))?;
        }
        Kind::FreedesktopTopDirAdmin | Kind::FreedesktopTopDirUser => {
            match std::fs::create_dir(dir) {
                Ok(()) => {
                    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => bail!(
                    "cannot create {} ({e}), and the home trash is on another filesystem: moving \
                     there would copy the tree. Nothing was moved.",
                    dir.display()
                ),
            }
        }
        Kind::MacOsUser => {}
    }
    own_real_dir(dir, ctx.uid)?;
    let files = files_dir(&location);
    let info = info_dir(&location).ok_or_else(|| anyhow!("no info directory for this trash"))?;
    for sub in [&files, &info] {
        match std::fs::create_dir(sub) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => bail!("could not create {}: {e}; nothing was moved", sub.display()),
        }
        own_real_dir(sub, ctx.uid)?;
    }
    let files_dev = std::fs::metadata(&files)?.dev();
    if files_dev != item_dev {
        bail!(
            "cross-device Trash unsupported: {} is on another filesystem than {}; no copy and no \
             permanent fallback, nothing was moved",
            files.display(),
            item.display()
        );
    }
    Ok(Resolved {
        location,
        files,
        info,
    })
}

/// A directory swamp will write a restore record or an item into must
/// be a real directory, not a symlink, and owned by this user.
fn own_real_dir(dir: &Path, uid: u32) -> Result<()> {
    let m = std::fs::symlink_metadata(dir)
        .with_context(|| format!("cannot stat {}; nothing was moved", dir.display()))?;
    if m.file_type().is_symlink() {
        bail!(
            "{} is a symlink; a Trash directory that points somewhere else is refused, nothing \
             was moved",
            dir.display()
        );
    }
    if !m.is_dir() {
        bail!("{} is not a directory; nothing was moved", dir.display());
    }
    if m.uid() != uid {
        bail!(
            "{} belongs to uid {}, not {uid}; nothing was moved",
            dir.display(),
            m.uid()
        );
    }
    Ok(())
}

fn nearest_existing_dev(path: &Path) -> Option<u64> {
    let mut p = Some(path);
    while let Some(cur) = p {
        if let Ok(m) = std::fs::metadata(cur) {
            return Some(m.dev());
        }
        p = cur.parent();
    }
    None
}

/// The mount point `item` lives on: the highest ancestor still on the
/// item's device. No mount table is read (`getmntent` is not
/// thread-safe, the caveat the `trash` crate documents); the device
/// boundary is the same one the walker uses.
fn mount_top(item: &Path, dev: u64) -> PathBuf {
    let mut top = item.parent().unwrap_or(item).to_path_buf();
    while let Some(parent) = top.parent() {
        match std::fs::metadata(parent) {
            Ok(m) if m.dev() == dev => top = parent.to_path_buf(),
            _ => break,
        }
    }
    top
}

struct Reserved {
    file: PathBuf,
    info: PathBuf,
}

/// Creates `info/<name>.trashinfo` with `O_EXCL` -- the spec's way of
/// reserving a name -- trying `name`, `name.2`, `name.3`, ... until one
/// is free in both `info/` and `files/`, and writes the record into it.
fn reserve(resolved: &Resolved, name: &str, original: &Path) -> Result<Reserved> {
    use std::io::Write as _;
    let record = trashinfo(original, &deletion_date(crate::entities::now()));
    for n in 1..=10_000u32 {
        let candidate = if n == 1 {
            name.to_string()
        } else {
            format!("{name}.{n}")
        };
        let info = resolved.info.join(format!("{candidate}.trashinfo"));
        let file = resolved.files.join(&candidate);
        let mut f = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&info)
        {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => bail!(
                "could not write the restore record {} ({e}); nothing was moved",
                info.display()
            ),
        };
        if std::fs::symlink_metadata(&file).is_ok() {
            drop(f);
            let _unused = std::fs::remove_file(&info);
            continue;
        }
        if let Err(e) = f.write_all(record.as_bytes()).and_then(|()| f.sync_all()) {
            drop(f);
            let _unused = std::fs::remove_file(&info);
            bail!(
                "could not write the restore record {} ({e}); nothing was moved",
                info.display()
            );
        }
        return Ok(Reserved { file, info });
    }
    bail!("no free name for {name} in {}", resolved.files.display())
}

/// The `.trashinfo` body the spec defines: the original path, URL-encoded
/// the way file managers decode it, and the deletion time in local time.
pub fn trashinfo(original: &Path, deletion_date: &str) -> String {
    format!(
        "[Trash Info]\nPath={}\nDeletionDate={deletion_date}\n",
        encode_path(original)
    )
}

/// RFC 2396 escaping of every byte outside the unreserved set, `/` kept.
pub fn encode_path(p: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut out = String::new();
    for &b in p.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `YYYY-MM-DDThh:mm:ss` in the local timezone, as the spec requires.
pub fn deletion_date(secs: u64) -> String {
    let t = secs as libc::time_t;
    // SAFETY: `tm` is written by localtime_r before it is read; both
    // pointers are to live stack values.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut tm).is_null() {
            return "1970-01-01T00:00:00".to_string();
        }
        tm
    };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
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

    #[test]
    fn a_trashinfo_record_names_the_original_path_and_time() {
        let body = trashinfo(Path::new("/home/dev/p q/target"), "2026-09-22T10:11:12");
        assert_eq!(
            body,
            "[Trash Info]\nPath=/home/dev/p%20q/target\nDeletionDate=2026-09-22T10:11:12\n"
        );
        let d = deletion_date(0);
        assert_eq!(d.len(), 19, "{d}");
        assert_eq!(&d[4..5], "-");
        assert_eq!(&d[10..11], "T");
    }

    #[test]
    fn macos_is_a_plain_directory_target_and_linux_is_freedesktop() {
        assert_eq!(
            Target::for_os(Os::MacOs, PathBuf::from("/Users/dev"), None, 501),
            Target::Directory(PathBuf::from("/Users/dev/.Trash"))
        );
        assert!(matches!(
            Target::for_os(Os::Linux, PathBuf::from("/home/dev"), None, 1000),
            Target::Freedesktop(_)
        ));
    }

    /// The explicit-directory target keeps swamp's historical behaviour
    /// on both platforms: a rename under the caller's name, nothing else
    /// written, and an existing destination refused rather than replaced.
    #[test]
    fn a_directory_target_renames_under_the_given_name_and_never_replaces() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("target");
        std::fs::create_dir_all(src.join("debug")).unwrap();
        std::fs::write(src.join("debug/x"), b"x").unwrap();
        let trash = tmp.path().join("Trash");
        let t = Target::Directory(trash.clone());
        let got = move_item(&src, &t, "target-p-1").unwrap();
        assert_eq!(got.location, trash.join("target-p-1"));
        assert_eq!(got.info, None);
        assert!(!src.exists());
        assert_eq!(
            std::fs::read(trash.join("target-p-1/debug/x")).unwrap(),
            b"x"
        );
        assert_eq!(std::fs::read_dir(&trash).unwrap().count(), 1);

        std::fs::create_dir_all(&src).unwrap();
        let err = move_item(&src, &t, "target-p-1").unwrap_err().to_string();
        assert!(err.contains("already exists"), "{err}");
        assert!(
            src.exists(),
            "a refused move leaves the source where it was"
        );
    }

    #[test]
    fn an_envelope_rolls_back_what_it_moved() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a");
        let b = tmp.path().join("b");
        std::fs::write(&a, b"a").unwrap();
        std::fs::write(&b, b"b").unwrap();
        let mut env =
            Envelope::open(&a, &Target::Directory(tmp.path().join("T")), "env-1").unwrap();
        env.move_member(&a, "0-a").unwrap();
        env.move_member(&b, "1-b").unwrap();
        assert!(!a.exists() && !b.exists());
        assert!(env.rollback().is_empty());
        assert_eq!(std::fs::read(&a).unwrap(), b"a");
        assert_eq!(std::fs::read(&b).unwrap(), b"b");
    }
}
