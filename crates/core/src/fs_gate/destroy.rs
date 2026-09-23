//! Every operation that moves, removes or rewrites data swamp does not
//! own. Each one takes a [`RecheckProof`] (by value: spent once) and an
//! [`Authorized`] (by reference), checks that the authorization names the
//! proof's anchor and that the proof is fresh, and only then acts.
//!
//! There is no `rename`, `remove_*` or `write` here that takes a bare
//! path. That is the compile-time form of
//! `.oh/guardrails/execution-sinks-recheck-live-state.md`: the
//! `crates/core/tests/compile_fail/` cases show a sink without a proof,
//! a proof forged outside `recheck`, and a raw `std::fs::rename` in a
//! sink module each fail to build (the last one in the gate audit too).

use crate::authority::Authorized;
use crate::recheck::RecheckProof;
use anyhow::{Context, Result, anyhow, bail};
use std::path::{Path, PathBuf};
use std::time::Duration;

fn licensed(proof: &RecheckProof, auth: &Authorized) -> Result<()> {
    if !auth.covers(proof.anchor()) {
        bail!(
            "refused: the authorization does not name {}",
            proof.anchor().display()
        );
    }
    if !proof.is_fresh() {
        bail!(
            "refused: the live recheck of {} is older than {}s; propose again",
            proof.anchor().display(),
            crate::recheck::MAX_PROOF_AGE.as_secs()
        );
    }
    Ok(())
}

fn plain_name(name: &str) -> Result<()> {
    if name.is_empty() || name.contains('/') || name == "." || name == ".." {
        bail!("refused: `{name}` is not a plain file name inside the Trash");
    }
    Ok(())
}

/// Moves the proof's anchor to `trash_root/dest_name` (one `rename`,
/// same volume). Returns where it went.
pub fn trash_move(
    proof: RecheckProof,
    auth: &Authorized,
    trash_root: &Path,
    dest_name: &str,
) -> Result<PathBuf> {
    licensed(&proof, auth)?;
    plain_name(dest_name)?;
    std::fs::create_dir_all(trash_root).context("could not create the Trash directory")?;
    let dest = trash_root.join(dest_name);
    if std::fs::symlink_metadata(&dest).is_ok() {
        bail!("refused: {} already exists in the Trash", dest.display());
    }
    std::fs::rename(proof.anchor(), &dest)
        .with_context(|| format!("rename to Trash failed for {}", proof.anchor().display()))?;
    Ok(dest)
}

/// A recovery envelope: one directory inside the Trash that receives a
/// multi-member unit (a session and its sidecars, a Cargo group) member
/// by member, next to its own `restore.json`.
///
/// Opened from a proof and an authorization; every member moved must be
/// one the proof covers.
#[derive(Debug)]
pub struct Envelope {
    dir: PathBuf,
    proof: RecheckProof,
    moved: Vec<(PathBuf, PathBuf)>,
}

impl Envelope {
    /// Creates `trash_root/name` (or reuses it if a previous attempt in
    /// the same second created it). When
    /// `same_device_as` is given, refuses a Trash on another volume:
    /// a cross-device move would be a copy plus a delete, and there is no
    /// permanent-delete fallback.
    pub fn open(
        proof: RecheckProof,
        auth: &Authorized,
        trash_root: &Path,
        name: &str,
        same_device_as: Option<u64>,
    ) -> Result<Envelope> {
        licensed(&proof, auth)?;
        plain_name(name)?;
        std::fs::create_dir_all(trash_root).context("could not create the Trash directory")?;
        if let Some(dev) = same_device_as {
            use std::os::unix::fs::MetadataExt;
            if std::fs::metadata(trash_root)?.dev() != dev {
                bail!("cross-device Trash unsupported; no permanent fallback");
            }
        }
        let dir = trash_root.join(name);
        std::fs::create_dir_all(&dir).context("could not create Trash envelope")?;
        Ok(Envelope {
            dir,
            proof,
            moved: Vec::new(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.dir
    }

    /// Writes (or rewrites) the envelope's own `restore.json`, synced.
    pub fn write_manifest<T: serde::Serialize + ?Sized>(&self, manifest: &T) -> Result<()> {
        let p = self.dir.join("restore.json");
        let bytes = serde_json::to_vec_pretty(manifest)?;
        std::fs::write(&p, bytes).context("writing restore.json")?;
        std::fs::File::open(&p)?.sync_all()?;
        Ok(())
    }

    /// Moves one covered member into the envelope as `dest_name`.
    pub fn move_member(&mut self, member: &Path, dest_name: &str) -> Result<PathBuf> {
        if !self.proof.covers(member) {
            bail!(
                "refused: {} is not a member the live recheck covered",
                member.display()
            );
        }
        if !self.proof.is_fresh() {
            bail!("refused: the live recheck is too old; propose again");
        }
        plain_name(dest_name)?;
        let to = self.dir.join(dest_name);
        std::fs::rename(member, &to)
            .map_err(|e| anyhow!("rename to Trash failed for {}: {e}", member.display()))?;
        self.moved.push((member.to_path_buf(), to.clone()));
        Ok(to)
    }

    /// Moves every member moved so far back where it came from, newest
    /// first. Returns what could not be restored.
    pub fn roll_back(&mut self) -> Vec<String> {
        let mut failures = Vec::new();
        while let Some((from, to)) = self.moved.pop() {
            if std::fs::symlink_metadata(&from).is_ok() {
                failures.push(format!("{} reappeared", from.display()));
            } else if let Err(e) = std::fs::rename(&to, &from) {
                failures.push(e.to_string());
            }
        }
        failures
    }
}

/// Copies one compiled output out of an authorized unit into
/// `dest_dir` before the unit is trashed (`--keep-executables`). Writes
/// into the user's worktree, so it is a destroy-group operation: the
/// source must lie under an authorized anchor.
pub fn copy_preserved(
    auth: &Authorized,
    anchor: &Path,
    from: &Path,
    dest_dir: &Path,
) -> Result<PathBuf> {
    if !auth.covers(anchor) || !crate::scope::under(from, anchor) {
        bail!(
            "refused: {} is not inside an authorized unit",
            from.display()
        );
    }
    std::fs::create_dir_all(dest_dir)?;
    let name = from.file_name().context("file has a name")?;
    let to = dest_dir.join(name);
    std::fs::copy(from, &to)
        .with_context(|| format!("copy {} to {}", from.display(), to.display()))?;
    Ok(to)
}

/// `docker image rm <id>` / `docker volume rm <name>`: permanent, in the
/// daemon, after `docker::still_removable` re-derived it. Returns the
/// daemon's own refusal text when it declines.
pub fn docker_remove(
    auth: &Authorized,
    anchor: &Path,
    kind: &str,
    id: &str,
) -> std::result::Result<(), String> {
    if !auth.covers(anchor) {
        return Err(format!(
            "refused: the authorization does not name {}",
            anchor.display()
        ));
    }
    if !matches!(kind, "image" | "volume") {
        return Err(format!(
            "refused: docker {kind} rm is not a supported removal"
        ));
    }
    let args: Vec<std::ffi::OsString> = vec![kind.into(), "rm".into(), id.into()];
    let out = super::spawn::run_unchecked(
        super::spawn::Program::Docker,
        &args,
        Duration::from_secs(60),
    )
    .map_err(|e| format!("docker: {e}"))?;
    if out.success() {
        return Ok(());
    }
    let err = out.stderr_lossy().trim().to_string();
    Err(if err.is_empty() {
        "docker refused the removal without saying why".to_string()
    } else {
        err
    })
}

/// `git -C <repo> worktree prune`, after a linked worktree the human
/// confirmed was moved to the Trash. Best-effort; the move already
/// happened.
pub fn git_worktree_prune(auth: &Authorized, anchor: &Path, repo: &Path) -> Result<()> {
    if !auth.covers(anchor) {
        bail!(
            "refused: the authorization does not name {}",
            anchor.display()
        );
    }
    let args: Vec<std::ffi::OsString> = vec![
        "-C".into(),
        repo.as_os_str().to_owned(),
        "worktree".into(),
        "prune".into(),
    ];
    super::spawn::run_unchecked(super::spawn::Program::Git, &args, Duration::from_secs(60))?;
    Ok(())
}
