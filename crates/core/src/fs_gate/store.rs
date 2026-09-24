//! Writes to swamp's **own** files. The history tables are Parquet
//! ([`super::columns`]); everything else swamp persists is one of the
//! variants below, and there is no other way to put bytes on disk.
//!
//! That makes two guardrails types rather than audits:
//!
//! * `.oh/guardrails/json-persistence-is-allowlisted.md`: JSON reaches
//!   disk only through [`write_json`], and [`JsonFile`] *is* the
//!   allow-list -- a new control file is a new variant here, reviewed
//!   where every other one is. `serde_json::to_string(..)` followed by a
//!   raw `std::fs::write` is a gate violation (audit + clippy), and there
//!   is no `write_bytes(path, ..)` to launder it through.
//! * `.oh/guardrails/store-data-is-parquet-not-json-sidecars.md`: there
//!   is no per-unit, per-path or free-named JSON file; every variant's
//!   file name is fixed (or, for plans and cached reports, built from a
//!   validated id under a fixed prefix).
//!
//! Every write is atomic (sibling temp file + `fsync` + rename + parent
//! directory `fsync`) except [`append_line`], the ledger's append-only
//! discipline.
//!
//! **Locations are types too** (re-review 5, finding 4). A file here is a
//! fixed name inside a [`StoreDir`] -- never a caller's path -- and the
//! three files that live outside a store (the ledger override, the
//! observation log, the LaunchAgent plist) are resolved *here*, from the
//! environment, or validated by name. There is no `create_dir_all(path)`,
//! `list(path)` or `remove(path)` a caller can point anywhere.

use serde::Serialize;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// A swamp state directory: every [`JsonFile`], [`TextFile`] and lock
/// lives at a fixed name inside one.
///
/// Built from the resolved swamp dir ([`StoreDir::resolved`]: what the
/// CLI and TUI use), or -- for the core entry points that take a store
/// `&Path`, which tests hand a temp dir -- [`StoreDir::at`], which the
/// gate audit allows only in the store modules. `at` refuses a relative
/// path, a symlink, and anything that exists and is not a directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreDir(PathBuf);

impl StoreDir {
    /// `$SWAMP_DIR`, else `$HOME/.local/share/swamp`: the one resolver.
    pub fn resolved() -> StoreDir {
        if let Some(dir) = std::env::var_os("SWAMP_DIR") {
            return StoreDir(PathBuf::from(dir));
        }
        let home = std::env::var_os("HOME").unwrap_or_else(|| ".".into());
        StoreDir(PathBuf::from(home).join(".local/share/swamp"))
    }

    /// A store at `dir`. Refused unless `dir` is absolute and, when
    /// something is there, a real directory (not a symlink to one).
    pub fn at(dir: &Path) -> io::Result<StoreDir> {
        if !dir.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} is not an absolute store directory", dir.display()),
            ));
        }
        match std::fs::symlink_metadata(dir) {
            Ok(m) if m.is_dir() => Ok(StoreDir(dir.to_path_buf())),
            Ok(_) => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "{} is not a directory swamp can keep state in",
                    dir.display()
                ),
            )),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(StoreDir(dir.to_path_buf())),
            Err(e) => Err(e),
        }
    }

    /// A subdirectory swamp owns (a root's history volume, `external/`):
    /// one plain name, never a path.
    pub fn subdir(&self, name: &str) -> io::Result<StoreDir> {
        Ok(StoreDir(self.0.join(plain(name)?)))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    /// Creates the directory (and its parents) if it is missing.
    pub fn create(&self) -> io::Result<()> {
        std::fs::create_dir_all(&self.0)
    }
}

/// Every JSON file swamp persists. The file name is decided here.
#[derive(Debug, Clone, Copy)]
pub enum JsonFile<'a> {
    /// `<store>/plans/<id>.json`: one proposed plan.
    Plan { store: &'a StoreDir, id: &'a str },
    /// `<store>/grants.json`: every grant a human minted.
    Grants { store: &'a StoreDir },
    /// `<store>/agent_protect.json`: the human keep list.
    ProtectList { store: &'a StoreDir },
    /// `<store>/scope.json`: the last resolved scope (coverage
    /// bookkeeping only).
    Scope { store: &'a StoreDir },
    /// `<store>/last_run.json`: the last scheduled observation's summary.
    LastRun { store: &'a StoreDir },
    /// `<store>/docker_facts.json`: the Docker daemon answer, cached for
    /// its TTL.
    DockerFacts { store: &'a StoreDir },
    /// `<store>/ui_state.json`: the TUI's remembered filter and view.
    UiState { store: &'a StoreDir },
    /// `<volume>/fsevents.json`: the FSEvents cursor for one root.
    FsEventsCursor { volume: &'a StoreDir },
    /// `<volume>/topology.json`: the worktree topology of one root.
    Topology { volume: &'a StoreDir },
    /// `<volume>/unowned.json`: the unowned rows of one root.
    Unowned { volume: &'a StoreDir },
    /// `<store>/last_report-<key>.json.zst`: the last report, zstd
    /// compressed, so a `--no-observe` or TUI start never re-walks.
    LastReport { store: &'a StoreDir, key: &'a str },
}

/// How a [`JsonFile`] is encoded on disk.
enum Encoding {
    Pretty,
    Compact,
    CompactZstd,
}

fn plain(id: &str) -> io::Result<&str> {
    if id.is_empty() || id.contains(['/', '\\']) || id == "." || id == ".." {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("`{id}` is not a plain store file id"),
        ));
    }
    Ok(id)
}

impl JsonFile<'_> {
    /// Where the file lives.
    pub fn path(&self) -> io::Result<PathBuf> {
        Ok(match *self {
            JsonFile::Plan { store, id } => {
                store.0.join("plans").join(format!("{}.json", plain(id)?))
            }
            JsonFile::Grants { store } => store.0.join("grants.json"),
            JsonFile::ProtectList { store } => store.0.join("agent_protect.json"),
            JsonFile::Scope { store } => store.0.join("scope.json"),
            JsonFile::LastRun { store } => store.0.join("last_run.json"),
            JsonFile::DockerFacts { store } => store.0.join("docker_facts.json"),
            JsonFile::UiState { store } => store.0.join("ui_state.json"),
            JsonFile::FsEventsCursor { volume } => volume.0.join("fsevents.json"),
            JsonFile::Topology { volume } => volume.0.join("topology.json"),
            JsonFile::Unowned { volume } => volume.0.join("unowned.json"),
            JsonFile::LastReport { store, key } => store
                .0
                .join(format!("last_report-{}.json.zst", plain(key)?)),
        })
    }

    fn encoding(&self) -> Encoding {
        match self {
            JsonFile::Plan { .. }
            | JsonFile::Grants { .. }
            | JsonFile::ProtectList { .. }
            | JsonFile::Scope { .. } => Encoding::Pretty,
            JsonFile::LastReport { .. } => Encoding::CompactZstd,
            _ => Encoding::Compact,
        }
    }
}

/// Serializes `value` into `file`, atomically.
pub fn write_json<T: Serialize + ?Sized>(file: JsonFile<'_>, value: &T) -> io::Result<()> {
    let path = file.path()?;
    let bytes = match file.encoding() {
        Encoding::Pretty => serde_json::to_vec_pretty(value)?,
        Encoding::Compact => serde_json::to_vec(value)?,
        Encoding::CompactZstd => zstd::stream::encode_all(&serde_json::to_vec(value)?[..], 3)?,
    };
    write_atomic(&path, &bytes)
}

/// Reads `file` back: `Ok(None)` when it does not exist. The bytes are
/// the JSON text (decompressed for [`JsonFile::LastReport`]).
pub fn read_json_bytes(file: JsonFile<'_>) -> io::Result<Option<Vec<u8>>> {
    let path = file.path()?;
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    Ok(Some(match file.encoding() {
        Encoding::CompactZstd => zstd::stream::decode_all(&bytes[..])?,
        _ => bytes,
    }))
}

/// Every non-JSON text file swamp writes.
#[derive(Debug, Clone, Copy)]
pub enum TextFile<'a> {
    /// `<store>/config.toml`, written only by `swamp config init`.
    Config { store: &'a StoreDir },
    /// The scheduled refresh's LaunchAgent plist, where
    /// [`launch_agent_plist`] resolves it.
    LaunchAgent,
}

impl TextFile<'_> {
    pub fn path(&self) -> io::Result<PathBuf> {
        match *self {
            TextFile::Config { store } => Ok(store.0.join("config.toml")),
            TextFile::LaunchAgent => launch_agent_plist(),
        }
    }
}

/// `<LaunchAgents>/<label>.plist`: `$SWAMP_LAUNCH_AGENTS_DIR` (tests), else
/// `~/Library/LaunchAgents`. The only plist swamp writes, removes or hands
/// to `launchctl`.
pub fn launch_agent_plist() -> io::Result<PathBuf> {
    let dir = match std::env::var_os("SWAMP_LAUNCH_AGENTS_DIR") {
        Some(v) => PathBuf::from(v),
        None => PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| ".".into()))
            .join("Library/LaunchAgents"),
    };
    Ok(dir.join(format!("{}.plist", crate::schedule::LABEL)))
}

/// Writes `text` into `file`, atomically.
pub fn write_text(file: TextFile<'_>, text: &str) -> io::Result<()> {
    write_atomic(&file.path()?, text.as_bytes())
}

/// Removes a file swamp wrote (the LaunchAgent plist on `schedule
/// --off`). Absent is not an error.
pub fn remove_text(file: TextFile<'_>) -> io::Result<()> {
    remove_owned_file(&file.path()?)
}

/// Every append-only log swamp keeps.
#[derive(Debug, Clone, Copy)]
pub enum LogFile<'a> {
    /// The action ledger: `<store>/ledger.jsonl`.
    Ledger(&'a StoreDir),
    /// The action ledger where `$SWAMP_LEDGER_PATH` puts it (read here,
    /// not handed in); `<store>/ledger.jsonl` when unset.
    LedgerResolved(&'a StoreDir),
    /// The scheduled observation log: a file named `observe.log`
    /// (`schedule::log_file`), refused under any other name.
    Observations(&'a Path),
}

impl LogFile<'_> {
    /// Where the log lives.
    pub fn path(&self) -> io::Result<PathBuf> {
        Ok(match *self {
            LogFile::Ledger(store) => store.0.join("ledger.jsonl"),
            LogFile::LedgerResolved(store) => match std::env::var_os("SWAMP_LEDGER_PATH") {
                Some(p) => PathBuf::from(p),
                None => store.0.join("ledger.jsonl"),
            },
            LogFile::Observations(path) => {
                if path.file_name().is_none_or(|n| n != "observe.log") || !path.is_absolute() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("{} is not swamp's observation log", path.display()),
                    ));
                }
                path.to_path_buf()
            }
        })
    }
}

/// Appends one line and syncs it.
pub fn append_line(file: LogFile<'_>, line: &str) -> io::Result<()> {
    let path = file.path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    writeln!(f, "{line}")?;
    f.sync_data()
}

/// The single-flight observation lock, `<store>/observe.lock`.
#[derive(Debug, Clone, Copy)]
pub struct ObserveLock<'a> {
    pub store: &'a StoreDir,
}

impl ObserveLock<'_> {
    pub fn path(&self) -> PathBuf {
        self.store.0.join("observe.lock")
    }

    /// Creates the lock only if nothing is there (`O_EXCL`), holding
    /// `contents` (`pid<TAB>started_at`).
    pub fn create(&self, contents: &str) -> io::Result<()> {
        self.store.create()?;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.path())?;
        f.write_all(contents.as_bytes())?;
        f.sync_all()
    }

    /// Releases (or reclaims a stale) lock.
    pub fn remove(&self) -> io::Result<()> {
        remove_owned_file(&self.path())
    }
}

/// Atomic write: sibling temp file + `fsync` + rename + parent `fsync`.
/// Private: callers name a [`JsonFile`] or [`TextFile`].
///
/// Durability of the *rename*, not only of the bytes: without an fsync
/// on the parent directory a crash can lose the directory entry the
/// rename created, leaving the old contents (or nothing) where
/// protection state should be (the 2026-09-22 re-review's P3).
/// Best-effort: a filesystem that refuses to sync a directory handle
/// must not fail the write that already succeeded.
fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(
        ".{}.tmp-{}-{}",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("control"),
        std::process::id(),
        crate::entities::new_id()
    ));
    std::fs::write(&tmp, bytes)?;
    if let Ok(f) = std::fs::File::open(&tmp) {
        let _ = f.sync_all();
    }
    match std::fs::rename(&tmp, path) {
        Ok(()) => {
            if let Ok(d) = std::fs::File::open(dir) {
                let _ = d.sync_all();
            }
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Removes a regular file swamp wrote; absent is fine.
pub(super) fn remove_owned_file(path: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.is_file() => std::fs::remove_file(path),
        Ok(_) => Err(io::Error::other(format!(
            "{} is not a regular file swamp wrote",
            path.display()
        ))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}
