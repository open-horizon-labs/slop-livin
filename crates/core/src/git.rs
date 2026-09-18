use crate::entities::{ActivitySignal, Confidence, FactMeta, Project, Worktree};
use anyhow::Result;
use std::{path::Path, process::Command};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GitFacts {
    pub project: Project,
    pub worktrees: Vec<Worktree>,
    pub signals: Vec<ActivitySignal>,
}
pub fn extract(root: &Path) -> Result<Option<GitFacts>> {
    let out = Command::new("git")
        .args([
            "-C",
            root.to_str().unwrap_or("."),
            "rev-parse",
            "--git-common-dir",
        ])
        .output()?;
    if !out.status.success() {
        return Ok(None);
    }
    let repo_id = crate::scan::repo_identity(root)
        .unwrap_or_else(|| crate::entities::id_for(&root.display().to_string()));
    let meta = FactMeta::now("git.rev-parse", Confidence::High);
    let project = Project {
        id: repo_id.clone(),
        path: root.to_path_buf(),
        confidence: Confidence::High,
        meta: meta.clone(),
    };
    let wt = Worktree {
        path: root.to_path_buf(),
        repo_id: repo_id.clone(),
        linked: false,
        locked: false,
        meta: meta.clone(),
    };
    let mut signals = Vec::new();
    for (name, args) in [
        ("last_commit", &["log", "-1", "--format=%ct"][..]),
        ("dirty", &["status", "--porcelain"][..]),
        ("unpushed", &["rev-list", "--left-right", "@{u}...HEAD"][..]),
    ] {
        let mut command_args = vec!["-C", root.to_str().unwrap_or(".")];
        command_args.extend_from_slice(args);
        let value = Command::new("git")
            .args(command_args)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        signals.push(ActivitySignal {
            project_id: repo_id.clone(),
            name: name.into(),
            value,
            meta: FactMeta::now(format!("git.{name}"), Confidence::High),
        });
    }
    Ok(Some(GitFacts {
        project,
        worktrees: vec![wt],
        signals,
    }))
}
