//! Exact, reviewed Cargo groups. No age-based eligibility or blanket prune.
//! Supports the tested legacy profile layout only. Locks are advisory: manual
//! writers that ignore Cargo's locks must be stopped by the user.
use crate::artifact::{ArtifactRole, NestedArtifact};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Read,
    os::unix::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Member {
    pub path: PathBuf,
    pub device: u64,
    pub inode: u64,
    pub bytes: u64,
    pub digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CargoGroup {
    pub container: PathBuf,
    pub selected: PathBuf,
    pub profile: PathBuf,
    pub role: ArtifactRole,
    pub members: Vec<Member>,
    pub lock_paths: Vec<PathBuf>,
}

pub fn candidate(unit: &NestedArtifact) -> bool {
    if !unit.coverage.complete || !unit.coverage.supported {
        return false;
    }
    let parent = unit
        .path
        .parent()
        .and_then(Path::file_name)
        .and_then(|n| n.to_str());
    if unit.is_dir {
        matches!(
            (&unit.role, parent),
            (ArtifactRole::Incremental, Some("incremental"))
                | (ArtifactRole::BuildScriptOutput, Some("build"))
        )
    } else {
        matches!(
            unit.role,
            ArtifactRole::TestExecutable | ArtifactRole::Example
        )
    }
}

fn regular(path: &Path) -> Result<fs::File> {
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    if !file.metadata()?.is_file() {
        bail!("not a regular file: {}", path.display());
    }
    Ok(file)
}

fn snapshot(path: &Path) -> Result<Member> {
    snapshot_at(path, 0, &mut 0)
}

fn snapshot_at(path: &Path, depth: usize, visited: &mut usize) -> Result<Member> {
    *visited += 1;
    if depth > 128 || *visited > 100_000 {
        bail!("cleanup review exceeds depth/member limit; inspection-only");
    }
    let canonical = fs::canonicalize(path)?;
    if canonical != path {
        bail!("symlink or noncanonical member: {}", path.display());
    }
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() {
        let mut paths = fs::read_dir(path)?
            .map(|e| e.map(|e| e.path()))
            .collect::<std::io::Result<Vec<_>>>()?;
        paths.sort();
        let mut hash = blake3::Hasher::new();
        let mut bytes = 0;
        for p in paths {
            let m = snapshot_at(&p, depth + 1, visited)?;
            bytes += m.bytes;
            hash.update(&serde_json::to_vec(&m)?);
        }
        return Ok(Member {
            path: path.into(),
            device: metadata.dev(),
            inode: metadata.ino(),
            bytes,
            digest: hash.finalize().to_hex().to_string(),
        });
    }
    let mut file = regular(path)?;
    let before = file.metadata()?;
    if before.nlink() != 1 {
        bail!("shared hardlink is inspection-only: {}", path.display());
    }
    let mut hash = blake3::Hasher::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    let after = file.metadata()?;
    if (
        before.len(),
        before.mtime(),
        before.mtime_nsec(),
        before.ctime(),
        before.ctime_nsec(),
    ) != (
        after.len(),
        after.mtime(),
        after.mtime_nsec(),
        after.ctime(),
        after.ctime_nsec(),
    ) {
        bail!("member changed during review");
    }
    Ok(Member {
        path: path.into(),
        device: before.dev(),
        inode: before.ino(),
        bytes: before.blocks() * 512,
        digest: hash.finalize().to_hex().to_string(),
    })
}

fn local_filesystem(path: &Path) -> Result<()> {
    let c = std::ffi::CString::new(path.as_os_str().as_bytes())?;
    let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
    if unsafe { libc::statfs(c.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let stat = unsafe { stat.assume_init() };
    #[cfg(target_os = "macos")]
    let local = stat.f_flags & libc::MNT_LOCAL as u32 != 0;
    #[cfg(target_os = "linux")]
    let local = matches!(
        stat.f_type as u64,
        0xef53 | 0x9123683e | 0x58465342 | 0x01021994
    );
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let local = false;
    if !local {
        bail!(
            "Cargo cleanup requires a supported local filesystem; network/unknown mounts are inspection-only"
        );
    }
    Ok(())
}

fn locks(profile: &Path) -> Result<Vec<PathBuf>> {
    local_filesystem(profile)?;
    let mut paths = Vec::new();
    for name in [".cargo-lock", ".cargo-build-lock"] {
        let p = profile.join(name);
        match fs::symlink_metadata(&p) {
            Ok(m) if m.is_file() && !m.file_type().is_symlink() => paths.push(p),
            Ok(_) => bail!("invalid Cargo lock: {}", p.display()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    if paths.is_empty() {
        bail!("no established Cargo build lock; inspection-only");
    }
    Ok(paths)
}

fn acquire(paths: &[PathBuf]) -> Result<Vec<fs::File>> {
    let mut held = Vec::new();
    for p in paths {
        let file = regular(p)?;
        file.try_lock().map_err(|e| {
            anyhow::anyhow!(
                "Cargo build busy or lock unavailable at {}: {e}",
                p.display()
            )
        })?;
        let fd = file.metadata()?;
        let path = fs::symlink_metadata(p)?;
        if (fd.dev(), fd.ino()) != (path.dev(), path.ino()) {
            bail!("Cargo lock replaced");
        }
        held.push(file);
    }
    Ok(held)
}

/// Select an evidenced test/example executable with its companions, or one
/// incremental/build-script directory. Shared dependencies, whole profiles, unknown layouts,
/// and files without a trustworthy role are intentionally not action groups.
pub fn propose(units: &[NestedArtifact], selected: &Path, container: &Path) -> Result<CargoGroup> {
    let unit = units
        .iter()
        .find(|u| u.path == selected)
        .context("no nested artifact at selection")?;
    if !unit.coverage.complete || !unit.coverage.supported {
        bail!("incomplete/unsupported Cargo coverage");
    }
    let directory_group = unit.is_dir
        && matches!(
            unit.role,
            ArtifactRole::Incremental | ArtifactRole::BuildScriptOutput
        );
    if !directory_group
        && (!matches!(
            unit.role,
            ArtifactRole::TestExecutable | ArtifactRole::Example
        ) || unit.is_dir)
    {
        bail!("this Cargo role is inspection-only; select a test or example executable");
    }
    let relative = selected.strip_prefix(container)?;
    let canonical_container = fs::canonicalize(container)?;
    let canonical_selected = fs::canonicalize(selected)?;
    if canonical_container.join(relative) != canonical_selected
        || fs::symlink_metadata(selected)?.file_type().is_symlink()
    {
        bail!("selection crosses a symlink; propose again");
    }
    let container = canonical_container;
    let selected = canonical_selected;
    let rel = selected.strip_prefix(&container)?;
    let count = rel.components().count();
    if !(count == 3 || count == 4) {
        bail!("unsupported Cargo executable layout");
    }
    let dir = selected.parent().context("missing executable parent")?;
    let dirname = dir.file_name().and_then(|s| s.to_str()).unwrap_or("");
    if !(matches!(dirname, "deps" | "examples") && !directory_group
        || matches!(dirname, "incremental" | "build") && directory_group)
    {
        bail!("unsupported Cargo executable directory");
    }
    let meta = fs::symlink_metadata(&selected)?;
    if !directory_group && meta.mode() & 0o111 == 0 {
        bail!("not an executable");
    }
    let profile = dir.parent().context("missing profile")?.to_path_buf();
    let lock_paths = locks(&profile)?;
    let _held = acquire(&lock_paths)?;
    let mut paths = vec![selected.clone()];
    let dep = selected.with_extension("d");
    if !directory_group && dep != selected && dep.exists() {
        paths.push(dep);
    }
    let dsym = selected.with_extension("dSYM");
    if !directory_group && dsym.exists() {
        paths.push(dsym);
    }
    let members: Vec<_> = paths.iter().map(|p| snapshot(p)).collect::<Result<_>>()?;
    Ok(CargoGroup {
        container,
        selected,
        profile,
        role: unit.role.clone(),
        members,
        lock_paths,
    })
}

/// Execute only behind the caller's existing explicit authorization. All group
/// members are checked before the first move. Failure rolls back completed
/// moves where possible, with the recovery envelope retained on disk.
pub(crate) fn move_reviewed(group: &CargoGroup, trash: &Path) -> Result<PathBuf> {
    if locks(&group.profile)? != group.lock_paths {
        bail!("Cargo lock set changed; propose again");
    }
    let _held = acquire(&group.lock_paths)?;
    let inspection =
        crate::cargo_artifacts::inspect_target(&group.container, Some(&group.container));
    if !inspection.coverage.complete {
        bail!("Cargo coverage incomplete; refusing cleanup");
    }
    // Recheck role, exact companion membership and file identities under locks.
    // propose() also locks, so perform the structural comparison without it.
    let current = inspection
        .units
        .iter()
        .find(|u| u.path == group.selected)
        .context("selected build vanished")?;
    if current.role != group.role {
        bail!("Cargo role/evidence changed; propose again");
    }
    let mut expected = vec![group.selected.clone()];
    let dep = group.selected.with_extension("d");
    if !current.is_dir && dep.exists() && dep != group.selected {
        expected.push(dep);
    }
    if !current.is_dir && group.selected.with_extension("dSYM").exists() {
        expected.push(group.selected.with_extension("dSYM"));
    }
    if expected
        != group
            .members
            .iter()
            .map(|m| m.path.clone())
            .collect::<Vec<_>>()
    {
        bail!("companion membership changed; propose again");
    }
    for m in &group.members {
        if snapshot(&m.path)? != *m {
            bail!("stale Cargo member {}; propose again", m.path.display());
        }
        if occupied(&m.path) {
            bail!("occupied or occupancy unavailable: {}", m.path.display());
        }
    }
    fs::create_dir_all(trash)?;
    if fs::metadata(trash)?.dev() != group.members[0].device {
        bail!("cross-device Trash unsupported; no permanent fallback");
    }
    let dest = trash.join(format!("swamp-cargo-{}", crate::entities::new_id()));
    fs::create_dir(&dest)?;
    fs::write(dest.join("restore.json"), serde_json::to_vec_pretty(group)?)?;
    fs::File::open(dest.join("restore.json"))?.sync_all()?;
    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
    for (i, member) in group.members.iter().enumerate() {
        let to = dest.join(format!(
            "{i}-{}",
            member.path.file_name().unwrap().to_string_lossy()
        ));
        if let Err(e) = fs::rename(&member.path, &to) {
            let mut failures = Vec::new();
            for (from, to) in moved.iter().rev() {
                if from.exists() {
                    failures.push(format!("{} reappeared", from.display()));
                } else if let Err(e) = fs::rename(to, from) {
                    failures.push(e.to_string());
                }
            }
            bail!(
                "Cargo move failed: {e}; recovery manifest {}; rollback errors: {:?}",
                dest.display(),
                failures
            );
        }
        moved.push((member.path.clone(), to));
    }
    Ok(dest)
}

fn occupied(path: &Path) -> bool {
    let mut cmd = std::process::Command::new("lsof");
    if path.is_dir() {
        cmd.arg("+D").arg(path);
    } else {
        cmd.arg("--").arg(path);
    }
    // Bound the occupancy probe and avoid pipe backpressure. Any diagnostic,
    // timeout, or launch failure refuses cleanup.
    let Ok(output) = tempfile::tempfile() else {
        return true;
    };
    let (Ok(stdout), Ok(stderr)) = (output.try_clone(), output.try_clone()) else {
        return true;
    };
    let Ok(mut child) = cmd.stdout(stdout).stderr(stderr).spawn() else {
        return true;
    };
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return !(status.code() == Some(1)
                    && output.metadata().map(|m| m.len() == 0).unwrap_or(false));
            }
            Ok(None) if started.elapsed() < std::time::Duration::from_secs(10) => {
                std::thread::sleep(std::time::Duration::from_millis(20))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return true;
            }
        }
    }
}
