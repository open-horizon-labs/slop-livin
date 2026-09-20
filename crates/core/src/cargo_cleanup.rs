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
    #[serde(default)]
    pub nlink: u64,
    #[serde(default)]
    pub hardlink_members: u64,
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
    pub evidence: Vec<Member>,
    /// Bytes allocated by the selected directory entries. This is not a
    /// promise about bytes reclaimed: an inode may still have aliases.
    #[serde(default)]
    pub allocated_bytes: u64,
    /// Reserved for a conservative allocation-based potential estimate.
    /// Currently always `None`: APFS clones, snapshots, and same-device
    /// Trash make any reclaimed-space value unknowable, so this never becomes
    /// an immediate-free-space promise or an execution gate.
    #[serde(default)]
    pub reclaimable_bytes: Option<u64>,
    /// Compact per-plan evidence for the action/UI layers. We deliberately do
    /// not retain or build a filesystem-wide alias index.
    #[serde(default)]
    pub hardlink_members: u64,
    #[serde(default)]
    pub shared_storage: bool,
}

/// Derived from existing facts only. Never performs I/O or implies authorization.
#[derive(Debug, Serialize)]
pub struct Guidance {
    pub scope: &'static str,
    pub check_status: &'static str,
    pub reason_code: &'static str,
    pub message: &'static str,
    pub next_action: &'static str,
}

pub fn guidance(unit: &NestedArtifact) -> Guidance {
    let (scope, status, code, message, next) =
        if !unit.coverage.complete || !unit.coverage.supported {
            (
                "unknown",
                "blocked",
                "coverage_limited",
                "Coverage is incomplete; refresh before reviewing cleanup.",
                "refresh",
            )
        } else if candidate(unit) {
            (
                "group",
                "unchecked",
                "checks_not_run",
                "Cleanup checks not run. Identification does not establish disuse.",
                "review_cleanup",
            )
        } else if unit.is_dir {
            (
                "summary",
                "not_applicable",
                "summary_row",
                "Category total, not a selective cleanup unit. Inspect individual groups.",
                "inspect_groups",
            )
        } else {
            (
                "output",
                "blocked",
                "unsupported_role",
                "This output is inspection-only; selective cleanup is not supported.",
                "inspect",
            )
        };
    Guidance {
        scope,
        check_status: status,
        reason_code: code,
        message,
        next_action: next,
    }
}

/// Add derived guidance to report JSON without persisting a second action model.
pub fn serialize_units<S: serde::Serializer>(
    units: &[NestedArtifact],
    serializer: S,
) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeSeq;
    #[derive(Serialize)]
    struct View<'a> {
        #[serde(flatten)]
        unit: &'a NestedArtifact,
        cleanup: Guidance,
    }
    let mut seq = serializer.serialize_seq(Some(units.len()))?;
    for unit in units {
        seq.serialize_element(&View {
            unit,
            cleanup: guidance(unit),
        })?;
    }
    seq.end()
}

#[derive(Debug, Serialize)]
pub struct CheckResult {
    pub path: PathBuf,
    pub allocated_bytes: u64,
    pub check_status: &'static str,
    pub reason_code: &'static str,
    pub message: String,
    pub next_action: &'static str,
    pub plan_id: Option<String>,
    /// Argument vector, never shell-interpolated. Caller must retain its store.
    pub next_command: Vec<String>,
    pub members: Vec<PathBuf>,
    /// Human-facing plan warnings, including shared-storage uncertainty.
    pub warnings: Vec<String>,
    pub recovery: Option<String>,
    pub checked_at: u64,
    pub elapsed_ms: u128,
}

/// Explicit bounded review; no approval, execution, or automatic scope expansion.
pub fn check(
    report: &crate::report::Report,
    store: &Path,
    paths: &[PathBuf],
    role: Option<&str>,
    limit: usize,
) -> Result<Vec<CheckResult>> {
    anyhow::ensure!((1..=20).contains(&limit), "limit must be between 1 and 20");
    let mut selected: Vec<_> = report
        .nested_artifacts
        .iter()
        .filter(|u| {
            (if paths.is_empty() {
                candidate(u)
            } else {
                paths.contains(&u.path)
            }) && role.is_none_or(|r| u.role.label() == r)
        })
        .collect();
    if !paths.is_empty() {
        for path in paths {
            anyhow::ensure!(
                selected.iter().any(|u| &u.path == path),
                "No matching Cargo row for {}. Use an exact path from report JSON; selection was not widened.",
                path.display()
            );
        }
        anyhow::ensure!(
            selected.len() <= limit,
            "Selection exceeds limit; increase --limit (maximum 20) or select fewer paths"
        );
    }
    selected.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.path.cmp(&b.path)));
    selected.truncate(limit);
    let mut results = Vec::new();
    for unit in selected {
        let start = std::time::Instant::now();
        let checked_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs();
        let g = guidance(unit);
        let mut result = CheckResult {
            path: unit.path.clone(),
            allocated_bytes: unit.bytes,
            check_status: g.check_status,
            reason_code: g.reason_code,
            message: g.message.into(),
            next_action: g.next_action,
            plan_id: None,
            next_command: vec![
                "swamp".into(),
                "report".into(),
                report.root.display().to_string(),
                "--view".into(),
                "rust".into(),
                "--all".into(),
            ],
            members: Vec::new(),
            warnings: Vec::new(),
            recovery: None,
            checked_at,
            elapsed_ms: 0,
        };
        if g.check_status == "unchecked" {
            match crate::actions::propose(
                report,
                None,
                std::slice::from_ref(&unit.path),
                "cleanup-check",
            ) {
                Ok(plan) => {
                    crate::actions::save_plan(store, &plan)?;
                    result.members = plan
                        .units
                        .iter()
                        .filter_map(|u| u.cargo_group.as_ref())
                        .flat_map(|g| g.members.iter().map(|m| m.path.clone()))
                        .collect();
                    result.recovery = plan.units.first().map(|u| u.recovery.clone());
                    result.warnings = plan
                        .units
                        .iter()
                        .flat_map(|u| u.warnings.iter().cloned())
                        .collect();
                    result.check_status = "ready_for_review";
                    result.reason_code = "checks_passed";
                    result.message = "Unapproved plan created. Checked layout, Cargo lock, member contents and fingerprint evidence. Not confirmed unused. Review exact members and rebuilding consequences; execution rechecks the selection and occupancy. Trash does not promise immediate free space.".into();
                    result.next_action = "review_plan";
                    result.next_command = vec!["swamp".into(), "plans".into(), "--json".into()];
                    result.plan_id = Some(plan.id);
                    result.allocated_bytes = plan.units.iter().map(|u| u.bytes).sum();
                }
                Err(error) => {
                    result.check_status = "blocked";
                    let message = error.to_string();
                    result.reason_code = if message.contains("Cargo build busy or lock unavailable")
                    {
                        "lock_unavailable"
                    } else if message.contains("no established Cargo build lock") {
                        "missing_lock"
                    } else {
                        "review_refused"
                    };
                    result.message = message;
                    if result.reason_code == "lock_unavailable" {
                        result.message.push_str(" A build may hold the lock, or locking may be unavailable. Wait for builds to finish, then retry this exact selection. Nothing changed.");
                        result.next_action = "retry_after_builds";
                        result.next_command = vec![
                            "swamp".into(),
                            "cleanup-check".into(),
                            report.root.display().to_string(),
                            "--path".into(),
                            unit.path.display().to_string(),
                        ];
                    } else {
                        result.next_action = "inspect";
                    }
                }
            }
        }
        result.elapsed_ms = start.elapsed().as_millis();
        results.push(result);
    }
    Ok(results)
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

/// Authorization identity for a selected path. Link counts and allocation
/// accounting are observations, not safety facts: aliases outside the
/// selection may change without changing the selected content or membership.
fn same_safety(a: &Member, b: &Member) -> bool {
    a.path == b.path && a.device == b.device && a.inode == b.inode && a.digest == b.digest
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
        let mut hardlink_members = 0;
        for p in paths {
            let m = snapshot_at(&p, depth + 1, visited)?;
            bytes += m.bytes;
            hardlink_members += m.hardlink_members;
            // The safety identity deliberately excludes allocation/link
            // accounting. External aliases may appear or disappear without
            // changing this selected group's membership or content.
            hash.update(&serde_json::to_vec(&(
                &m.path, m.device, m.inode, &m.digest,
            ))?);
        }
        return Ok(Member {
            path: path.into(),
            device: metadata.dev(),
            inode: metadata.ino(),
            // Directory link counts describe `.`/`..`, not aliases of the
            // directory's contents. Child file counts are aggregated above.
            nlink: 1,
            hardlink_members,
            bytes,
            digest: hash.finalize().to_hex().to_string(),
        });
    }
    let mut file = regular(path)?;
    let before = file.metadata()?;
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
        nlink: before.nlink(),
        hardlink_members: u64::from(before.nlink() > 1),
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

struct HeldLocks(Vec<fs::File>);

impl Drop for HeldLocks {
    fn drop(&mut self) {
        // Closing alone leaves flock held when another thread's fork temporarily
        // inherits the open file description. End our critical section explicitly.
        for file in &self.0 {
            let _ = file.unlock();
        }
    }
}

fn acquire(paths: &[PathBuf]) -> Result<HeldLocks> {
    let mut held = HeldLocks(Vec::new());
    for p in paths {
        let file = regular(p)?;
        file.try_lock().map_err(|e| {
            anyhow::anyhow!(
                "Cargo build busy or lock unavailable at {}: {e}",
                p.display()
            )
        })?;
        held.0.push(file);
        let file = held.0.last().unwrap();
        let fd = file.metadata()?;
        let path = fs::symlink_metadata(p)?;
        if (fd.dev(), fd.ino()) != (path.dev(), path.ino()) {
            bail!("Cargo lock replaced");
        }
    }
    Ok(held)
}

#[test]
fn explicit_unlock_releases_even_with_a_duplicated_description() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join(".cargo-lock");
    fs::write(&path, b"").unwrap();
    let held = acquire(std::slice::from_ref(&path)).unwrap();
    let duplicate = held.0[0].try_clone().unwrap();
    assert!(
        acquire(std::slice::from_ref(&path)).is_err(),
        "live guard must exclude another holder"
    );
    drop(held);
    let reacquired = acquire(&[path]).expect("duplicate must not extend the critical section");
    drop(reacquired);
    drop(duplicate);
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
    if !candidate(unit) {
        bail!(
            "{} Nothing changed. Use report --view rust to inspect individual groups, then cleanup-check --path with an exact group path.",
            guidance(unit).message
        );
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
    let evidence: Vec<_> = unit
        .producer_evidence
        .iter()
        .filter(|e| e.source == "cargo-fingerprint")
        .map(|e| snapshot(Path::new(&e.detail)))
        .collect::<Result<_>>()?;
    let evidence_paths: Vec<_> = evidence.iter().map(|e| e.path.clone()).collect();
    if crate::cargo_artifacts::reviewed_role(&container, &selected, &evidence_paths)?.0 != unit.role
    {
        bail!("Cargo role/evidence changed; refresh before proposing");
    }
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
    let allocated_bytes = members.iter().map(|m| m.bytes).sum();
    let hardlink_members = members.iter().map(|m| m.hardlink_members).sum();
    Ok(CargoGroup {
        container,
        selected,
        profile,
        role: unit.role.clone(),
        members,
        lock_paths,
        evidence,
        allocated_bytes,
        reclaimable_bytes: None,
        hardlink_members,
        shared_storage: hardlink_members > 0,
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
    for evidence in &group.evidence {
        if !same_safety(&snapshot(&evidence.path)?, evidence) {
            bail!("Cargo role/evidence changed; propose again");
        }
    }
    let paths: Vec<_> = group.evidence.iter().map(|e| e.path.clone()).collect();
    let (role, is_dir) =
        crate::cargo_artifacts::reviewed_role(&group.container, &group.selected, &paths)?;
    if role != group.role {
        bail!("Cargo role/evidence changed; propose again");
    }
    let mut expected = vec![group.selected.clone()];
    let dep = group.selected.with_extension("d");
    if !is_dir && dep.exists() && dep != group.selected {
        expected.push(dep);
    }
    if !is_dir && group.selected.with_extension("dSYM").exists() {
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
        if !same_safety(&snapshot(&m.path)?, m) {
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
