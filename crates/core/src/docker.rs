//! Docker facts: images, build cache entries, and volumes, read either
//! from a mocked `docker system df -v --format json` file (tests, or the
//! CLI's `--docker-facts <file>`) or from a live, bounded call to the
//! daemon. The daemon is never allowed to hang or fail the report: an
//! unreachable/slow/erroring daemon becomes `DockerFacts::unavailable`,
//! never an `Err`.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

const DOCKER_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Default)]
pub struct DockerImageFact {
    pub id: String,
    pub repo_tags: Vec<String>,
    pub labels: HashMap<String, String>,
    pub shared_bytes: u64,
    pub unique_bytes: u64,
}

#[derive(Debug, Clone, Default)]
pub struct DockerCacheFact {
    pub id: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Default)]
pub struct DockerVolumeFact {
    pub name: String,
    pub labels: HashMap<String, String>,
    pub bytes: u64,
}

#[derive(Debug, Clone, Default)]
pub struct DockerFacts {
    pub images: Vec<DockerImageFact>,
    pub build_cache: Vec<DockerCacheFact>,
    pub volumes: Vec<DockerVolumeFact>,
    /// Set when the daemon/file could not be read: the reason text is
    /// surfaced as a report-level coverage note, never an error.
    pub unavailable: Option<String>,
}

fn value_str(v: &serde_json::Value) -> String {
    v.as_str().map(|s| s.to_string()).unwrap_or_default()
}

/// Parses a comma-joined `k=v,k=v` label string, the shape both the
/// fixture and `docker ... --format json` use for `Labels`.
fn parse_labels(raw: &str) -> HashMap<String, String> {
    raw.split(',')
        .filter(|s| !s.is_empty())
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// Parses a docker size: a plain byte count (test fixtures) or a human
/// string such as `"153MB"`, `"83.9kB"`, `"0B"` (the live CLI's own
/// `--format json` output uses decimal, not binary, units).
fn parse_size(raw: &str) -> u64 {
    let raw = raw.trim();
    if let Ok(n) = raw.parse::<u64>() {
        return n;
    }
    let Some(idx) = raw.find(|c: char| c.is_alphabetic()) else {
        return 0;
    };
    let (num, unit) = raw.split_at(idx);
    let Ok(num) = num.trim().parse::<f64>() else {
        return 0;
    };
    let mult: f64 = match unit.trim() {
        "B" => 1.0,
        "kB" | "KB" => 1_000.0,
        "MB" => 1_000_000.0,
        "GB" => 1_000_000_000.0,
        "TB" => 1_000_000_000_000.0,
        "KiB" => 1024.0,
        "MiB" => 1024.0 * 1024.0,
        "GiB" => 1024.0 * 1024.0 * 1024.0,
        _ => 1.0,
    };
    (num * mult).round() as u64
}

fn parse_value(value: &serde_json::Value) -> DockerFacts {
    let mut facts = DockerFacts::default();

    if let Some(images) = value.get("Images").and_then(|v| v.as_array()) {
        for image in images {
            let labels = image
                .get("Labels")
                .map(value_str)
                .map(|s| parse_labels(&s))
                .unwrap_or_default();
            let mut repo_tags = Vec::new();
            if let Some(tags) = image.get("RepoTags").and_then(|v| v.as_array()) {
                for t in tags {
                    if let Some(s) = t.as_str() {
                        repo_tags.push(s.to_string());
                    }
                }
            } else {
                let repo = image.get("Repository").map(value_str).unwrap_or_default();
                let tag = image.get("Tag").map(value_str).unwrap_or_default();
                if !repo.is_empty() {
                    repo_tags.push(if tag.is_empty() {
                        repo
                    } else {
                        format!("{repo}:{tag}")
                    });
                }
            }
            let id = image.get("ID").map(value_str).unwrap_or_default();
            let shared_bytes = image
                .get("SharedSize")
                .map(value_str)
                .map(|s| parse_size(&s))
                .unwrap_or(0);
            let unique_bytes = image
                .get("UniqueSize")
                .map(value_str)
                .map(|s| parse_size(&s))
                .unwrap_or_else(|| {
                    image
                        .get("Size")
                        .map(value_str)
                        .map(|s| parse_size(&s))
                        .unwrap_or(0)
                });
            facts.images.push(DockerImageFact {
                id,
                repo_tags,
                labels,
                shared_bytes,
                unique_bytes,
            });
        }
    }

    if let Some(entries) = value.get("BuildCache").and_then(|v| v.as_array()) {
        for entry in entries {
            let id = entry.get("ID").map(value_str).unwrap_or_default();
            let bytes = entry
                .get("Size")
                .map(value_str)
                .map(|s| parse_size(&s))
                .unwrap_or(0);
            facts.build_cache.push(DockerCacheFact { id, bytes });
        }
    }

    if let Some(entries) = value.get("Volumes").and_then(|v| v.as_array()) {
        for entry in entries {
            let name = entry.get("Name").map(value_str).unwrap_or_default();
            let labels = entry
                .get("Labels")
                .map(value_str)
                .map(|s| parse_labels(&s))
                .unwrap_or_default();
            let bytes = entry
                .get("Size")
                .map(value_str)
                .map(|s| parse_size(&s))
                .unwrap_or(0);
            facts.volumes.push(DockerVolumeFact {
                name,
                labels,
                bytes,
            });
        }
    }

    facts
}

/// Loads Docker facts either from a mocked facts file (tests, or the
/// CLI's `--docker-facts <file>`) or, when `facts_path` is `None`, from a
/// live, bounded `docker system df -v --format json` call. Never errors:
/// a missing binary, unreachable daemon, timeout, or bad output all
/// become `DockerFacts::unavailable`.
pub fn load(facts_path: Option<&Path>) -> DockerFacts {
    match facts_path {
        Some(path) => load_from_file(path),
        None => load_live(),
    }
}

fn load_from_file(path: &Path) -> DockerFacts {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            return DockerFacts {
                unavailable: Some(format!("docker: unavailable ({e})")),
                ..Default::default()
            };
        }
    };
    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(v) => parse_value(&v),
        Err(e) => DockerFacts {
            unavailable: Some(format!("docker: unavailable (invalid facts file: {e})")),
            ..Default::default()
        },
    }
}

fn load_live() -> DockerFacts {
    let mut child = match Command::new("docker")
        .args(["system", "df", "-v", "--format", "json"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return DockerFacts {
                unavailable: Some(format!("docker: unavailable ({e})")),
                ..Default::default()
            };
        }
    };

    let (tx, rx) = mpsc::channel();
    if let Some(mut stdout) = child.stdout.take() {
        std::thread::spawn(move || {
            let mut buf = String::new();
            let _ = stdout.read_to_string(&mut buf);
            let _ = tx.send(buf);
        });
    }

    let deadline = std::time::Instant::now() + DOCKER_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(_) => break None,
        }
    };

    let Some(status) = status else {
        return DockerFacts {
            unavailable: Some("docker: unavailable (timed out)".to_string()),
            ..Default::default()
        };
    };
    if !status.success() {
        return DockerFacts {
            unavailable: Some("docker: unavailable (daemon not responding)".to_string()),
            ..Default::default()
        };
    }
    let stdout = rx.recv_timeout(Duration::from_secs(2)).unwrap_or_default();
    match serde_json::from_str::<serde_json::Value>(&stdout) {
        Ok(v) => parse_value(&v),
        Err(e) => DockerFacts {
            unavailable: Some(format!("docker: unavailable (bad output: {e})")),
            ..Default::default()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_human_and_raw_sizes() {
        assert_eq!(parse_size("153MB"), 153_000_000);
        assert_eq!(parse_size("0B"), 0);
        assert_eq!(parse_size("83.9kB"), 83_900);
        assert_eq!(parse_size("8388608"), 8_388_608);
    }

    #[test]
    fn missing_facts_file_is_unavailable_not_an_error() {
        let facts = load_from_file(Path::new("/nonexistent/does-not-exist.json"));
        assert!(facts.unavailable.is_some());
        assert!(facts.images.is_empty());
    }
}
