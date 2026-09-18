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

    if let Some(entries) = value.get("ImageInspect").and_then(|v| v.as_array()) {
        merge_image_inspect(&mut facts.images, entries);
    }

    facts
}

/// `docker system df -v --format json` image rows carry no `Labels` at
/// all (confirmed against a live daemon: its image objects have exactly
/// `[Containers, CreatedAt, CreatedSince, Digest, ID, Repository,
/// SharedSize, Size, Tag, UniqueSize]`). Labels only exist in `docker
/// image inspect` output, under `Config.Labels`. This merges an
/// `ImageInspect`-shaped array (`[{"Id"|"ID", "RepoTags", "Config":
/// {"Labels": {...}}}, ...]`, the same shape `docker image inspect
/// --format json` returns) into the df-sourced image facts, matching by
/// image ID first and falling back to a shared repo:tag.
fn merge_image_inspect(images: &mut [DockerImageFact], inspect_entries: &[serde_json::Value]) {
    for entry in inspect_entries {
        let id = entry
            .get("Id")
            .or_else(|| entry.get("ID"))
            .map(value_str)
            .unwrap_or_default();
        let labels: HashMap<String, String> = entry
            .get("Config")
            .and_then(|c| c.get("Labels"))
            .and_then(|l| l.as_object())
            .map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        if labels.is_empty() {
            continue;
        }
        let inspect_tags: Vec<String> = entry
            .get("RepoTags")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|t| t.as_str())
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default();

        for image in images.iter_mut() {
            let matches = (!id.is_empty() && image.id == id)
                || image.repo_tags.iter().any(|t| inspect_tags.contains(t));
            if matches {
                image.labels.extend(labels.clone());
            }
        }
    }
}

/// Runs a `docker` subcommand and returns its parsed JSON stdout, bounded
/// by `timeout`. Returns `Err(reason)` on any failure (missing binary,
/// non-zero exit, timeout, bad JSON) -- the caller decides whether that
/// failure is fatal to the whole report or just means "no enrichment".
fn run_docker_json(args: &[&str], timeout: Duration) -> Result<serde_json::Value, String> {
    let mut child = Command::new("docker")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("docker: unavailable ({e})"))?;

    let (tx, rx) = mpsc::channel();
    if let Some(mut stdout) = child.stdout.take() {
        std::thread::spawn(move || {
            let mut buf = String::new();
            let _ = stdout.read_to_string(&mut buf);
            let _ = tx.send(buf);
        });
    }

    let deadline = std::time::Instant::now() + timeout;
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
        return Err("docker: unavailable (timed out)".to_string());
    };
    if !status.success() {
        return Err("docker: unavailable (daemon not responding)".to_string());
    }
    let stdout = rx.recv_timeout(Duration::from_secs(2)).unwrap_or_default();
    serde_json::from_str::<serde_json::Value>(&stdout)
        .map_err(|e| format!("docker: unavailable (bad output: {e})"))
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

/// `docker system df -v` never carries image labels (verified against a
/// live daemon), so a live load is two calls: the df call for object
/// identity/sizes, then a batched `docker image inspect <ids> --format
/// json` for the labels df doesn't have. Volume labels *are* present in
/// `docker system df -v` output already (also verified live), so no
/// separate `docker volume inspect` call is needed for them. Build-cache
/// entries have no label source at all and stay `DockerNoJoin`.
///
/// The inspect call is best-effort: if it fails or times out, images
/// simply keep whatever (empty) labels df gave them -- join falls
/// through to `DockerNoJoin` for those, which is the honest answer, not
/// a fatal error for the whole report.
fn load_live() -> DockerFacts {
    let df_value =
        match run_docker_json(&["system", "df", "-v", "--format", "json"], DOCKER_TIMEOUT) {
            Ok(v) => v,
            Err(reason) => {
                return DockerFacts {
                    unavailable: Some(reason),
                    ..Default::default()
                };
            }
        };
    let mut facts = parse_value(&df_value);

    let ids: Vec<&str> = facts
        .images
        .iter()
        .map(|i| i.id.as_str())
        .filter(|id| !id.is_empty())
        .collect();
    if !ids.is_empty() {
        let mut args: Vec<&str> = vec!["image", "inspect"];
        args.extend(ids);
        args.extend(["--format", "json"]);
        if let Ok(inspect_value) = run_docker_json(&args, DOCKER_TIMEOUT)
            && let Some(entries) = inspect_value.as_array()
        {
            merge_image_inspect(&mut facts.images, entries);
        }
        // A failed/timed-out inspect call is not surfaced as
        // `unavailable`: the df call (object identity, sizes) already
        // succeeded, so the report still reconciles; the affected images
        // just carry no labels and land as `DockerNoJoin`, same as any
        // other image with no join evidence.
    }

    facts
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
