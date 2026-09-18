//! Filter grammar shared by the CLI `--filter` flag, the MCP `report`/
//! `list_worktrees` tools, and (per #35) the TUI. Kept small and pure:
//! `parse` builds a `Filter`, `matches_worktree`/`matches_artifact`
//! evaluate it against a report row. No verdict vocabulary lives here;
//! predicates read facts (`merge-complete`, `idle`, `growth`, `kind`,
//! `project`, `pr`), they don't rename them into "safe"/"stale"/etc.
//!
//! Grammar (whitespace-separated conjunction, every predicate must match):
//!
//! ```text
//! merge-complete
//! idle > <duration>            (e.g. "idle > 48h")
//! growth > <size> in <duration>   (e.g. "growth > 10MB in 24h")
//! growth < <size> in <duration>
//! kind:<artifact-kind>
//! project:<name>
//! pr:open|merged|closed|none
//! ```

use crate::github::{MergedStatus, PrStatus};
use crate::growth::parse_duration_secs;
use crate::report::{ArtifactKind, ArtifactRow, ProjectRow, WorktreeRow};
use anyhow::{Result, anyhow};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Predicate {
    MergeComplete,
    IdleGreaterThan(u64),
    Growth {
        greater: bool,
        bytes: u64,
        within_secs: u64,
    },
    Kind(String),
    Project(String),
    Pr(PrFilter),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrFilter {
    Open,
    Merged,
    Closed,
    None,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filter {
    pub predicates: Vec<Predicate>,
}

/// Extra per-worktree facts a `Filter` needs that aren't already on
/// `WorktreeRow`: the composite `merge_complete` verdict-free fact, the
/// idle duration, and the raw GitHub PR status. Computed once per
/// worktree by `report.rs`/`render.rs` and passed in here rather than
/// recomputed by the filter.
pub struct WorktreeFacts<'a> {
    pub merge_complete: bool,
    pub idle_secs: Option<u64>,
    pub pr: &'a PrStatus,
    #[allow(dead_code)]
    pub merged: &'a MergedStatus,
}

fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim();
    let upper = s.to_uppercase();
    let (num_part, mult): (&str, u64) = if let Some(n) = upper.strip_suffix("GB") {
        (n, 1024 * 1024 * 1024)
    } else if let Some(n) = upper.strip_suffix("MB") {
        (n, 1024 * 1024)
    } else if let Some(n) = upper.strip_suffix("KB") {
        (n, 1024)
    } else if let Some(n) = upper.strip_suffix('B') {
        (n, 1)
    } else {
        (upper.as_str(), 1)
    };
    let n: f64 = num_part.trim().parse().ok()?;
    Some((n * mult as f64) as u64)
}

/// Parses a filter expression into a `Filter`. Tokens are whitespace
/// separated except for the two multi-token forms (`idle > X`,
/// `growth > X in Y`), which are consumed greedily from the token
/// stream. Unknown or malformed predicates produce an `Err` naming the
/// offending token.
pub fn parse(input: &str) -> Result<Filter> {
    let tokens: Vec<&str> = input.split_whitespace().collect();
    let mut predicates = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let tok = tokens[i];
        if tok == "merge-complete" {
            predicates.push(Predicate::MergeComplete);
            i += 1;
        } else if tok == "idle" {
            let op = tokens
                .get(i + 1)
                .ok_or_else(|| anyhow!("filter: 'idle' needs an operator, e.g. 'idle > 48h'"))?;
            if *op != ">" {
                return Err(anyhow!("filter: 'idle' only supports '>', got {op:?}"));
            }
            let dur_tok = tokens
                .get(i + 2)
                .ok_or_else(|| anyhow!("filter: 'idle >' needs a duration, e.g. '48h'"))?;
            let secs = parse_duration_secs(dur_tok)
                .ok_or_else(|| anyhow!("filter: invalid duration {dur_tok:?}"))?;
            predicates.push(Predicate::IdleGreaterThan(secs));
            i += 3;
        } else if tok == "growth" {
            let op = tokens.get(i + 1).ok_or_else(|| {
                anyhow!("filter: 'growth' needs an operator, e.g. 'growth > 10MB in 24h'")
            })?;
            let greater = match *op {
                ">" => true,
                "<" => false,
                other => {
                    return Err(anyhow!(
                        "filter: 'growth' only supports '>'/'<', got {other:?}"
                    ));
                }
            };
            let size_tok = tokens
                .get(i + 2)
                .ok_or_else(|| anyhow!("filter: 'growth {op}' needs a size, e.g. '10MB'"))?;
            let bytes =
                parse_size(size_tok).ok_or_else(|| anyhow!("filter: invalid size {size_tok:?}"))?;
            let in_tok = tokens.get(i + 3);
            if in_tok != Some(&"in") {
                return Err(anyhow!(
                    "filter: 'growth {op} {size_tok}' needs 'in <duration>', e.g. 'in 24h'"
                ));
            }
            let dur_tok = tokens
                .get(i + 4)
                .ok_or_else(|| anyhow!("filter: 'growth {op} {size_tok} in' needs a duration"))?;
            let within_secs = parse_duration_secs(dur_tok)
                .ok_or_else(|| anyhow!("filter: invalid duration {dur_tok:?}"))?;
            predicates.push(Predicate::Growth {
                greater,
                bytes,
                within_secs,
            });
            i += 5;
        } else if let Some(k) = tok.strip_prefix("kind:") {
            if k.is_empty() {
                return Err(anyhow!("filter: 'kind:' needs a value"));
            }
            predicates.push(Predicate::Kind(k.to_string()));
            i += 1;
        } else if let Some(p) = tok.strip_prefix("project:") {
            if p.is_empty() {
                return Err(anyhow!("filter: 'project:' needs a value"));
            }
            predicates.push(Predicate::Project(p.to_string()));
            i += 1;
        } else if let Some(p) = tok.strip_prefix("pr:") {
            let pr = match p {
                "open" => PrFilter::Open,
                "merged" => PrFilter::Merged,
                "closed" => PrFilter::Closed,
                "none" => PrFilter::None,
                other => {
                    return Err(anyhow!(
                        "filter: 'pr:' must be one of open|merged|closed|none, got {other:?}"
                    ));
                }
            };
            predicates.push(Predicate::Pr(pr));
            i += 1;
        } else {
            return Err(anyhow!("filter: unrecognized token {tok:?}"));
        }
    }
    Ok(Filter { predicates })
}

fn kind_matches(kind: &ArtifactKind, name: &str) -> bool {
    format!("{kind:?}").eq_ignore_ascii_case(name)
}

impl Filter {
    /// True when every predicate matches. An empty filter (no
    /// predicates) matches everything.
    pub fn matches_worktree(
        &self,
        project: &ProjectRow,
        worktree: &WorktreeRow,
        facts: &WorktreeFacts,
    ) -> bool {
        self.predicates.iter().all(|p| match p {
            Predicate::MergeComplete => facts.merge_complete,
            Predicate::IdleGreaterThan(secs) => facts.idle_secs.is_some_and(|i| i > *secs),
            Predicate::Growth {
                greater,
                bytes,
                within_secs: _,
            } => worktree.artifacts.iter().any(|a| match a.growth_bytes {
                Some(g) => {
                    let g_abs = g.unsigned_abs();
                    if *greater {
                        g > 0 && g_abs > *bytes
                    } else {
                        g < 0 && g_abs > *bytes
                    }
                }
                None => false,
            }),
            Predicate::Kind(name) => worktree
                .artifacts
                .iter()
                .any(|a| kind_matches(&a.kind, name)),
            Predicate::Project(name) => project.name.eq_ignore_ascii_case(name),
            Predicate::Pr(want) => match (want, facts.pr) {
                (PrFilter::None, PrStatus::None) => true,
                (PrFilter::Open, PrStatus::Some(pr)) => {
                    matches!(pr.state, crate::github::PrState::Open)
                }
                (PrFilter::Merged, PrStatus::Some(pr)) => {
                    matches!(pr.state, crate::github::PrState::Merged)
                }
                (PrFilter::Closed, PrStatus::Some(pr)) => {
                    matches!(pr.state, crate::github::PrState::Closed)
                }
                _ => false,
            },
        })
    }

    /// True when every predicate that can apply to a single artifact row
    /// matches (`kind:`/`project:`/`growth`); worktree-level predicates
    /// (`merge-complete`, `idle`, `pr:`) are ignored here since an
    /// artifact row has no branch of its own -- callers filtering
    /// artifacts should pair this with `matches_worktree` on the
    /// containing worktree first.
    pub fn matches_artifact(&self, project: &ProjectRow, artifact: &ArtifactRow) -> bool {
        self.predicates.iter().all(|p| match p {
            Predicate::MergeComplete | Predicate::IdleGreaterThan(_) | Predicate::Pr(_) => true,
            Predicate::Growth {
                greater,
                bytes,
                within_secs: _,
            } => match artifact.growth_bytes {
                Some(g) => {
                    let g_abs = g.unsigned_abs();
                    if *greater {
                        g > 0 && g_abs > *bytes
                    } else {
                        g < 0 && g_abs > *bytes
                    }
                }
                None => false,
            },
            Predicate::Kind(name) => kind_matches(&artifact.kind, name),
            Predicate::Project(name) => project.name.eq_ignore_ascii_case(name),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_merge_complete() {
        let f = parse("merge-complete").unwrap();
        assert_eq!(f.predicates, vec![Predicate::MergeComplete]);
    }

    #[test]
    fn round_trips_idle() {
        let f = parse("idle > 48h").unwrap();
        assert_eq!(f.predicates, vec![Predicate::IdleGreaterThan(48 * 3600)]);
    }

    #[test]
    fn round_trips_growth() {
        let f = parse("growth > 10MB in 24h").unwrap();
        assert_eq!(
            f.predicates,
            vec![Predicate::Growth {
                greater: true,
                bytes: 10 * 1024 * 1024,
                within_secs: 24 * 3600
            }]
        );
    }

    #[test]
    fn round_trips_conjunction() {
        let f = parse("merge-complete idle > 48h project:widgets pr:open").unwrap();
        assert_eq!(f.predicates.len(), 4);
    }

    #[test]
    fn rejects_garbage_with_a_message() {
        let err = parse("bogus-token").unwrap_err();
        assert!(err.to_string().contains("bogus-token"));

        let err = parse("idle >").unwrap_err();
        assert!(err.to_string().contains("duration"));

        let err = parse("pr:nope").unwrap_err();
        assert!(err.to_string().contains("pr:"));
    }
}
