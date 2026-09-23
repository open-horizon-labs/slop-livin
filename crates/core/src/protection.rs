//! Human keep/protect intent (#100's `swamp protect add/list/remove`):
//! a small JSON control file in the store, deliberately decoupled from
//! the growth store. Survives refresh; blocks actions; never inferred
//! from observation (`.oh/guardrails/protection-fails-closed.md`).
//!
//! [`ProtectList`] is opaque. Its one query is [`ProtectList::conflict`],
//! the both-directions containment test; there is no accessor to the
//! protected paths, no iterator, no `Deref`, no `Clone` of the inner
//! list. So a second protection predicate -- the one-directional
//! `starts_with` that re-review 1 found and re-review 4's sweep wrote
//! again as `strip_prefix(k).is_ok()` -- has nothing to run over: it can
//! be written, but it cannot be written *about protection*
//! (`crates/core/tests/compile_fail/protect_list_*.rs`). That replaces
//! the `protection_fails_closed` call-graph audit.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct ProtectFile {
    /// Canonical absolute paths a human explicitly asked to keep.
    paths: Vec<String>,
}

/// The file the protect list lives in.
pub fn protect_path(swamp_dir: &Path) -> PathBuf {
    swamp_dir.join("agent_protect.json")
}

/// The human keep list, loaded. Opaque: see the module docs.
#[derive(Debug)]
pub struct ProtectList {
    paths: Vec<PathBuf>,
}

impl ProtectList {
    /// Nothing protected: what a store without a protect file means.
    pub fn empty() -> ProtectList {
        ProtectList { paths: Vec::new() }
    }

    /// This list plus `extra` (a caller's own keep entries, e.g. a
    /// proposal's convenience list). The union only ever protects more.
    pub fn including(mut self, extra: &[PathBuf]) -> ProtectList {
        for p in extra {
            if !self.paths.contains(p) {
                self.paths.push(p.clone());
            }
        }
        self
    }

    /// Whether nothing at all is protected (a shortcut, not a query about
    /// any path).
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    /// The reason human keep/protect intent blocks `candidate`, or `None`.
    ///
    /// **The only** protection predicate in the crate. There used to be a
    /// second, `is_human_protected`, which returned a bare `bool` by
    /// delegating here -- and that was how a one-directional mutation
    /// survived: the audit inspected this function, while
    /// `actions::propose_checking_protection` called the boolean wrapper,
    /// and no test proposed an ordinary directory *containing* a protected
    /// descendant. One predicate, everywhere, so there is nothing to
    /// inspect the wrong one of
    /// (`.oh/guardrails/protection-fails-closed.md`).
    ///
    /// It covers `candidate` in **both** directions:
    ///
    /// * `candidate` is the protected path or lies beneath it -- the
    ///   original, obvious direction; and
    /// * a protected path lies beneath `candidate` -- the direction the
    ///   2026-09-21 review's `protected_descendant_must_prevent_parent_cache_proposal`
    ///   counterexample falsified. Protecting `debug/log.txt` and then
    ///   removing `debug/` destroys exactly what the human asked to keep, so
    ///   a unit *containing* a protected path is protected too.
    ///
    /// The returned string carries which direction matched, so a refusal can
    /// say *why*. It is the only query this type has, so there is no
    /// second (one-directional) predicate to reach for.
    pub fn conflict(&self, candidate: &Path) -> Option<String> {
        // One spelling for both sides. `external::discover_and_measure`
        // canonicalizes every candidate and `agents::discover_and_measure`
        // does not, so the same home comes back as `/var/folders/.../claude`
        // from one pass and `/private/var/folders/.../claude` from the
        // other. Compared literally, a single `protect` entry covered one
        // family and not the other (the 2026-09-22 re-review's P2); through
        // `scope::comparable` it covers both.
        let cand = crate::scope::comparable(candidate);
        for p in &self.paths {
            let prot = crate::scope::comparable(p);
            if cand == prot {
                return Some(format!("{} is kept by `swamp protect`", p.display()));
            }
            if cand.starts_with(&prot) {
                return Some(format!(
                    "{} is beneath the human-protected path {}",
                    candidate.display(),
                    p.display()
                ));
            }
            if prot.starts_with(&cand) {
                return Some(format!("contains human-protected path {}", p.display()));
            }
        }
        None
    }
}

/// What `swamp protect list` prints: the entries as text, one per line
/// (`Display`), or as a JSON array (`Serialize`). Nothing else: it is a
/// rendering, not a second copy of the list to test paths against.
#[derive(Debug, Serialize)]
#[serde(transparent)]
pub struct ProtectListing(Vec<String>);

impl ProtectListing {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }
}

impl std::fmt::Display for ProtectListing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, p) in self.0.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "{p}")?;
        }
        Ok(())
    }
}

fn load_paths(swamp_dir: &Path) -> Result<Vec<PathBuf>> {
    let path = protect_path(swamp_dir);
    match crate::fs_gate::read::read_owned_string(&path) {
        Ok(text) => {
            let f: ProtectFile = serde_json::from_str(&text).map_err(|e| {
                anyhow::anyhow!(
                    "protection state unknown: {} is malformed ({e}). Every action is refused \
                     until it is repaired or removed; `swamp protect list` shows this same error.",
                    path.display()
                )
            })?;
            for p in &f.paths {
                if p.trim().is_empty() {
                    anyhow::bail!(
                        "protection state unknown: {} contains an empty path entry. Every action \
                         is refused until it is repaired or removed.",
                        path.display()
                    );
                }
            }
            Ok(f.paths.into_iter().map(PathBuf::from).collect())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(anyhow::anyhow!(
            "protection state unknown: {} could not be read ({e}). Every action is refused \
             until it can be read again.",
            path.display()
        )),
    }
}

/// The single entry point for protection state
/// (`.oh/guardrails/protection-fails-closed.md`). An absent file is an
/// empty keep list -- the ordinary "nothing protected yet" case. A file
/// that exists but cannot be read or parsed is **not**: protection state
/// is then *unknown*, and every caller must fail closed rather than
/// proceed as if nothing were protected. Returning `Result` -- with no
/// `Default` for `ProtectList`, so `.unwrap_or_default()` does not
/// compile -- is what makes that structural instead of a convention.
pub fn load_protect(swamp_dir: &Path) -> Result<ProtectList> {
    Ok(ProtectList {
        paths: load_paths(swamp_dir)?,
    })
}

fn save_protect(swamp_dir: &Path, paths: &[PathBuf]) -> Result<()> {
    let f = ProtectFile {
        paths: paths.iter().map(|p| p.display().to_string()).collect(),
    };
    crate::fs_gate::store::write_json(
        crate::fs_gate::store::JsonFile::ProtectList { store: swamp_dir },
        &f,
    )?;
    Ok(())
}

/// Adds `path` to the human keep list. Stored verbatim, and **refused
/// unless it is absolute**.
///
/// Verbatim, because `AgentUnit.path`/`AgentMember.path` are built as
/// `home.join(relative)` and both sides of every comparison are brought
/// into one spelling by `scope::comparable` at comparison time, not by
/// rewriting what the human typed.
///
/// Absolute, because a relative entry protects nothing. The 2026-09-22
/// re-review's CE5: `swamp protect add debug` returned `Ok`, `swamp
/// protect list` showed `debug`, and the very next
/// propose/approve/execute moved `<home>/debug`.
/// `protection_conflict` compares against absolute unit paths in both
/// directions and a relative entry matches neither, so the protection
/// layer -- which refuses every action on a corrupt protect file --
/// accepted, confirmed, and then did not protect. The existing "used
/// verbatim, never canonicalized" rationale is about *symlink*
/// mismatch; it never justified accepting a path that cannot be
/// enforced.
///
/// Idempotent; a not-yet-observed path can still be protected in
/// advance.
pub fn protect_add(swamp_dir: &Path, path: &Path) -> Result<()> {
    if path.as_os_str().is_empty() {
        anyhow::bail!("refused: an empty path protects nothing");
    }
    if !path.is_absolute() {
        anyhow::bail!(
            "refused: `{}` is not an absolute path, and a relative entry protects nothing \
             (protection is compared against absolute unit paths in both directions). Pass the \
             full path, e.g. `$PWD/{}`.",
            path.display(),
            path.display()
        );
    }
    let mut paths = load_paths(swamp_dir)?;
    if !paths.iter().any(|p| p == path) {
        paths.push(path.to_path_buf());
        save_protect(swamp_dir, &paths)?;
    }
    Ok(())
}

pub fn protect_remove(swamp_dir: &Path, path: &Path) -> Result<()> {
    let mut paths = load_paths(swamp_dir)?;
    let before = paths.len();
    paths.retain(|p| p != path);
    if paths.len() != before {
        save_protect(swamp_dir, &paths)?;
    }
    Ok(())
}

/// The keep list's raw entries, for integration tests only (`testing`
/// feature; no production build has it). The reviewers' counterexample
/// files read the list through this name.
#[cfg(feature = "testing")]
pub fn protect_list(swamp_dir: &Path) -> Result<Vec<PathBuf>> {
    load_paths(swamp_dir)
}

/// The keep list as `swamp protect list` shows it; errors exactly as
/// [`load_protect`] does.
pub fn protect_listing(swamp_dir: &Path) -> Result<ProtectListing> {
    Ok(ProtectListing(
        load_paths(swamp_dir)?
            .iter()
            .map(|p| p.display().to_string())
            .collect(),
    ))
}
