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

use serde::Serialize;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Every JSON file swamp persists. The file name is decided here.
#[derive(Debug, Clone, Copy)]
pub enum JsonFile<'a> {
    /// `<store>/plans/<id>.json`: one proposed plan.
    Plan { store: &'a Path, id: &'a str },
    /// `<store>/grants.json`: every grant a human minted.
    Grants { store: &'a Path },
    /// `<store>/agent_protect.json`: the human keep list.
    ProtectList { store: &'a Path },
    /// `<store>/scope.json`: the last resolved scope (coverage
    /// bookkeeping only).
    Scope { store: &'a Path },
    /// `<store>/last_run.json`: the last scheduled observation's summary.
    LastRun { store: &'a Path },
    /// `<store>/docker_facts.json`: the Docker daemon answer, cached for
    /// its TTL.
    DockerFacts { store: &'a Path },
    /// `<store>/ui_state.json`: the TUI's remembered filter and view.
    UiState { store: &'a Path },
    /// `<volume>/fsevents.json`: the FSEvents cursor for one root.
    FsEventsCursor { volume: &'a Path },
    /// `<volume>/topology.json`: the worktree topology of one root.
    Topology { volume: &'a Path },
    /// `<volume>/unowned.json`: the unowned rows of one root.
    Unowned { volume: &'a Path },
    /// `<store>/last_report-<key>.json.zst`: the last report, zstd
    /// compressed, so a `--no-observe` or TUI start never re-walks.
    LastReport { store: &'a Path, key: &'a str },
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
            JsonFile::Plan { store, id } => store.join("plans").join(format!("{}.json", plain(id)?)),
            JsonFile::Grants { store } => store.join("grants.json"),
            JsonFile::ProtectList { store } => store.join("agent_protect.json"),
            JsonFile::Scope { store } => store.join("scope.json"),
            JsonFile::LastRun { store } => store.join("last_run.json"),
            JsonFile::DockerFacts { store } => store.join("docker_facts.json"),
            JsonFile::UiState { store } => store.join("ui_state.json"),
            JsonFile::FsEventsCursor { volume } => volume.join("fsevents.json"),
            JsonFile::Topology { volume } => volume.join("topology.json"),
            JsonFile::Unowned { volume } => volume.join("unowned.json"),
            JsonFile::LastReport { store, key } => {
                store.join(format!("last_report-{}.json.zst", plain(key)?))
            }
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
    Config { store: &'a Path },
    /// The scheduled refresh's LaunchAgent plist. Must be
    /// `…/LaunchAgents/<label>.plist`.
    LaunchAgent { plist: &'a Path },
}

impl TextFile<'_> {
    pub fn path(&self) -> io::Result<PathBuf> {
        match *self {
            TextFile::Config { store } => Ok(store.join("config.toml")),
            TextFile::LaunchAgent { plist } => {
                let in_agents = plist
                    .parent()
                    .and_then(Path::file_name)
                    .is_some_and(|n| n == "LaunchAgents");
                let named = plist
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n == format!("{}.plist", crate::schedule::LABEL));
                if in_agents && named {
                    Ok(plist.to_path_buf())
                } else {
                    Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("{} is not swamp's LaunchAgent plist", plist.display()),
                    ))
                }
            }
        }
    }
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
    /// The action ledger (`ledger.jsonl`, or `SWAMP_LEDGER_PATH`).
    Ledger(&'a Path),
    /// The scheduled observation log (`observe.log`).
    Observations(&'a Path),
}

/// Appends one line and syncs it.
pub fn append_line(file: LogFile<'_>, line: &str) -> io::Result<()> {
    let path = match file {
        LogFile::Ledger(p) | LogFile::Observations(p) => p,
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(f, "{line}")?;
    f.sync_data()
}

/// The single-flight observation lock, `<store>/observe.lock`.
#[derive(Debug, Clone, Copy)]
pub struct ObserveLock<'a> {
    pub store: &'a Path,
}

impl ObserveLock<'_> {
    pub fn path(&self) -> PathBuf {
        self.store.join("observe.lock")
    }

    /// Creates the lock only if nothing is there (`O_EXCL`), holding
    /// `contents` (`pid<TAB>started_at`).
    pub fn create(&self, contents: &str) -> io::Result<()> {
        std::fs::create_dir_all(self.store)?;
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

/// Creates a directory swamp owns (a store root, a log directory).
pub fn create_dir_all(dir: impl AsRef<Path>) -> io::Result<()> {
    std::fs::create_dir_all(dir)
}

/// The entries of a directory swamp owns (its `plans/`, a history
/// table's `deltas/`), one level, sorted. Empty when the directory does
/// not exist. Not a walk and never used on a user tree.
pub fn list_owned(dir: impl AsRef<Path>) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    out.sort();
    out
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
