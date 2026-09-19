//! Docker facts: images, build cache entries, and volumes, read either
//! from a mocked `docker system df -v --format json` file (tests, or the
//! CLI's `--docker-facts <file>`) or from a live, bounded call to the
//! daemon. The daemon is never allowed to hang or fail the report: an
//! unreachable/slow/erroring daemon becomes `DockerFacts::unavailable`,
//! never an `Err`.
//!
//! Beyond object identity/sizes, this module extracts the per-object
//! detail #33 asks for: `created_at`, compose service (read straight off
//! the object's own labels, no separate field needed), layer digests ->
//! `shared_with` (other images sharing >=1 layer), containers
//! referencing an image/volume (name, state, finished_at) from `docker
//! ps -a --format json` plus a batched `docker inspect` for the
//! container detail `ps` doesn't carry, a dangling flag, and build-cache
//! `last_used`/`usage_count`/`in_use`/`shared`. Every additional call
//! uses the same bounded timeout and best-effort-only failure policy as
//! the existing image-inspect call: a failure there degrades that one
//! piece of detail, never the whole report.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

const DOCKER_TIMEOUT: Duration = Duration::from_secs(5);

/// One container's reference to an image or volume, as shown on that
/// object's row. Never a verdict: "state" and "finished_at" are facts
/// ("exited"/"running"/... and a timestamp or `None`), not a judgment
/// about whether the object is safe to remove.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ContainerRef {
    pub name: String,
    pub state: String,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct DockerImageFact {
    pub id: String,
    pub repo_tags: Vec<String>,
    pub labels: HashMap<String, String>,
    pub shared_bytes: u64,
    pub unique_bytes: u64,
    pub created_at: Option<String>,
    /// `RootFS.Layers` digests from `docker image inspect`; empty when
    /// inspect failed/timed out or the image has none recorded.
    pub layers: Vec<String>,
    /// Other images' references (repo:tag, or id when untagged) sharing
    /// at least one layer digest with this one. Computed once every
    /// image's layers are known; never includes this image itself.
    pub shared_with: Vec<String>,
    pub containers: Vec<ContainerRef>,
    /// No `RepoTags` at all (a build stage's intermediate image, or the
    /// previous image behind a moved tag) -- a fact read straight off
    /// the object, not an inference.
    pub dangling: bool,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct DockerCacheFact {
    pub id: String,
    pub bytes: u64,
    pub last_used: Option<String>,
    pub usage_count: Option<u64>,
    pub in_use: bool,
    pub shared: bool,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct DockerVolumeFact {
    pub name: String,
    pub labels: HashMap<String, String>,
    pub bytes: u64,
    pub created_at: Option<String>,
    pub driver: Option<String>,
    pub containers: Vec<ContainerRef>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct DockerFacts {
    pub images: Vec<DockerImageFact>,
    pub build_cache: Vec<DockerCacheFact>,
    pub volumes: Vec<DockerVolumeFact>,
    /// Set when the daemon/file could not be read: the reason text is
    /// surfaced as a report-level coverage note, never an error.
    pub unavailable: Option<String>,
}

/// One container as read from `docker ps -a --format json` plus a
/// best-effort `docker inspect` for the fields `ps` doesn't carry
/// (precise `FinishedAt`, volume mount names). Not part of the public
/// `DockerFacts` shape -- it exists only to drive `containers` on the
/// image/volume facts above.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct ContainerFact {
    name: String,
    /// The `Image` field from `ps`: usually `repo:tag`, sometimes a bare
    /// image id -- matched against both on the image side.
    image_ref: String,
    state: String,
    finished_at: Option<String>,
    /// Volume names this container mounts (from `docker inspect`'s
    /// `Mounts[].Name`, `Type == "volume"` only).
    volume_names: Vec<String>,
}

fn value_str(v: &serde_json::Value) -> String {
    v.as_str().map(|s| s.to_string()).unwrap_or_default()
}

fn opt_value_str(v: &serde_json::Value) -> Option<String> {
    v.as_str().filter(|s| !s.is_empty()).map(String::from)
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
                if !repo.is_empty() && repo != "<none>" {
                    repo_tags.push(if tag.is_empty() || tag == "<none>" {
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
            let created_at = image.get("CreatedAt").and_then(opt_value_str);
            let dangling = repo_tags.is_empty();
            facts.images.push(DockerImageFact {
                id,
                repo_tags,
                labels,
                shared_bytes,
                unique_bytes,
                created_at,
                layers: Vec::new(),
                shared_with: Vec::new(),
                containers: Vec::new(),
                dangling,
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
            let last_used = entry.get("LastUsedAt").and_then(opt_value_str);
            let usage_count = entry.get("UsageCount").and_then(|v| {
                v.as_u64()
                    .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
            });
            let in_use = entry
                .get("InUse")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let shared = entry
                .get("Shared")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            facts.build_cache.push(DockerCacheFact {
                id,
                bytes,
                last_used,
                usage_count,
                in_use,
                shared,
            });
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
            let driver = entry.get("Driver").and_then(opt_value_str);
            let created_at = entry.get("CreatedAt").and_then(opt_value_str);
            facts.volumes.push(DockerVolumeFact {
                name,
                labels,
                bytes,
                created_at,
                driver,
                containers: Vec::new(),
            });
        }
    }

    if let Some(entries) = value.get("ImageInspect").and_then(|v| v.as_array()) {
        merge_image_inspect(&mut facts.images, entries);
    }

    if let Some(entries) = value.get("VolumeInspect").and_then(|v| v.as_array()) {
        merge_volume_inspect(&mut facts.volumes, entries);
    }

    if let Some(entries) = value.get("Containers").and_then(|v| v.as_array()) {
        let containers: Vec<ContainerFact> = entries.iter().map(parse_container_inspect).collect();
        join_containers(&mut facts, &containers);
    }

    compute_shared_with(&mut facts.images);

    facts
}

/// `docker system df -v --format json` image rows carry no `Labels` at
/// all (confirmed against a live daemon: its image objects have exactly
/// `[Containers, CreatedAt, CreatedSince, Digest, ID, Repository,
/// SharedSize, Size, Tag, UniqueSize]`). Labels only exist in `docker
/// image inspect` output, under `Config.Labels`; layer digests live
/// under `RootFS.Layers`. This merges an `ImageInspect`-shaped array
/// (`[{"Id"|"ID", "RepoTags", "Config": {"Labels": {...}}, "RootFS":
/// {"Layers": [...]}}, ...]`, the same shape `docker image inspect
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
        let layers: Vec<String> = entry
            .get("RootFS")
            .and_then(|r| r.get("Layers"))
            .and_then(|l| l.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|d| d.as_str())
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default();
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

        if labels.is_empty() && layers.is_empty() {
            continue;
        }

        for image in images.iter_mut() {
            let matches = (!id.is_empty() && image.id == id)
                || image.repo_tags.iter().any(|t| inspect_tags.contains(t));
            if matches {
                image.labels.extend(labels.clone());
                if image.layers.is_empty() {
                    image.layers = layers.clone();
                }
            }
        }
    }
}

/// Merges `docker volume inspect --format json` output (`[{"Name",
/// "CreatedAt", "Driver"}, ...]`) into the df-sourced volume facts, by
/// name.
fn merge_volume_inspect(volumes: &mut [DockerVolumeFact], inspect_entries: &[serde_json::Value]) {
    for entry in inspect_entries {
        let name = entry.get("Name").map(value_str).unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let created_at = entry.get("CreatedAt").and_then(opt_value_str);
        let driver = entry.get("Driver").and_then(opt_value_str);
        for volume in volumes.iter_mut() {
            if volume.name == name {
                if volume.created_at.is_none() {
                    volume.created_at = created_at.clone();
                }
                if volume.driver.is_none() {
                    volume.driver = driver.clone();
                }
            }
        }
    }
}

/// Parses one `docker inspect` entry that is a container (has a `State`
/// object): name (leading `/` stripped, matching `docker ps`'s bare
/// name), image reference, state, precise `FinishedAt` (empty/zero-value
/// == never finished / still running, normalized to `None`), and the
/// volume names among its mounts.
fn parse_container_inspect(entry: &serde_json::Value) -> ContainerFact {
    let name = entry
        .get("Name")
        .map(value_str)
        .unwrap_or_default()
        .trim_start_matches('/')
        .to_string();
    let image_ref = entry
        .get("Config")
        .and_then(|c| c.get("Image"))
        .map(value_str)
        .or_else(|| entry.get("Image").map(value_str))
        .unwrap_or_default();
    let state = entry
        .get("State")
        .and_then(|s| s.get("Status"))
        .map(value_str)
        .unwrap_or_default();
    let finished_at = entry
        .get("State")
        .and_then(|s| s.get("FinishedAt"))
        .and_then(opt_value_str)
        .filter(|s| !s.starts_with("0001-01-01"));
    let volume_names: Vec<String> = entry
        .get("Mounts")
        .and_then(|m| m.as_array())
        .map(|arr| {
            arr.iter()
                .filter(|m| m.get("Type").and_then(|t| t.as_str()) == Some("volume"))
                .filter_map(|m| m.get("Name").and_then(|n| n.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default();
    ContainerFact {
        name,
        image_ref,
        state,
        finished_at,
        volume_names,
    }
}

/// Attaches each container as a [`ContainerRef`] to every image and
/// volume it references. An image match is by repo:tag or bare id
/// (docker ps's `Image` field is inconsistently one or the other
/// depending on whether the tag still exists); a volume match is by
/// name.
fn join_containers(facts: &mut DockerFacts, containers: &[ContainerFact]) {
    for c in containers {
        // A container the daemon gave us no name for tells the human
        // nothing, and rendered it as an empty `()` in the list of what
        // holds an image. Skip it rather than print a blank.
        if c.name.trim().is_empty() {
            continue;
        }
        let container_ref = ContainerRef {
            name: c.name.clone(),
            state: c.state.clone(),
            finished_at: c.finished_at.clone(),
        };
        for image in facts.images.iter_mut() {
            let matches = image.repo_tags.iter().any(|t| t == &c.image_ref)
                || image.id == c.image_ref
                || image
                    .id
                    .trim_start_matches("sha256:")
                    .starts_with(&c.image_ref);
            if matches {
                image.containers.push(container_ref.clone());
            }
        }
        for vol_name in &c.volume_names {
            for volume in facts.volumes.iter_mut() {
                if &volume.name == vol_name {
                    volume.containers.push(container_ref.clone());
                }
            }
        }
    }
}

/// Fills `shared_with` on every image whose layer set overlaps another
/// image's: an O(n^2) comparison over layer digest sets, fine at the
/// scale a single machine's `docker system df` returns (low hundreds of
/// images at most).
fn compute_shared_with(images: &mut [DockerImageFact]) {
    let refs: Vec<(String, std::collections::HashSet<String>)> = images
        .iter()
        .map(|i| {
            let name = i.repo_tags.first().cloned().unwrap_or_else(|| i.id.clone());
            (name, i.layers.iter().cloned().collect())
        })
        .collect();
    for (idx, image) in images.iter_mut().enumerate() {
        if image.layers.is_empty() {
            continue;
        }
        let own_layers: std::collections::HashSet<String> = image.layers.iter().cloned().collect();
        let mut shared_with = Vec::new();
        for (other_idx, (other_name, other_layers)) in refs.iter().enumerate() {
            if other_idx == idx || other_layers.is_empty() {
                continue;
            }
            if own_layers.intersection(other_layers).next().is_some() {
                shared_with.push(other_name.clone());
            }
        }
        shared_with.sort();
        image.shared_with = shared_with;
    }
}

/// Runs a `docker` subcommand and returns its parsed JSON stdout, bounded
/// by `timeout`. Returns `Err(reason)` on any failure (missing binary,
/// non-zero exit, timeout, bad JSON) -- the caller decides whether that
/// failure is fatal to the whole report or just means "no enrichment".
/// Docker's `--format json` on a list subcommand (`ps`, sometimes
/// others) emits newline-delimited JSON objects rather than one array,
/// so on a whole-output parse failure this also retries as NDJSON,
/// wrapping the parsed lines in a `Value::Array`.
/// What removing one Docker object costs, and whether we can do it at
/// all. Nothing here goes to Trash: the daemon has no such thing, so
/// every removal below is permanent, and the caller must say so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Removal {
    /// `docker image rm <id>`: the layers go, and the image comes back
    /// only by pulling or rebuilding it.
    Image { id: String },
    /// `docker volume rm <name>`: a volume's contents exist nowhere else.
    Volume { name: String },
    /// Docker exposes no per-record removal for build cache — only
    /// `docker builder prune`, which is a different unit of action.
    Refused(&'static str),
}

/// Runs a removal. Returns the daemon's own refusal text when it declines
/// (an image still referenced by a container, a volume still mounted),
/// because that reason is the fact the human needs.
pub fn remove(target: &Removal, timeout: Duration) -> Result<(), String> {
    let args: Vec<&str> = match target {
        Removal::Image { id } => vec!["image", "rm", id.as_str()],
        Removal::Volume { name } => vec!["volume", "rm", name.as_str()],
        Removal::Refused(why) => return Err((*why).to_string()),
    };
    let out = Command::new("docker")
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("docker: {e}"))?
        .wait_with_output()
        .map_err(|e| format!("docker: {e}"))?;
    let _ = timeout;
    if out.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
    Err(if err.is_empty() {
        "docker refused the removal without saying why".to_string()
    } else {
        err
    })
}

/// Is this object still present, and still unused? Re-derived at the sink
/// immediately before removal, never trusted from the report.
pub fn still_removable(target: &Removal) -> Result<(), String> {
    let (kind, id) = match target {
        Removal::Image { id } => ("image", id.as_str()),
        Removal::Volume { name } => ("volume", name.as_str()),
        Removal::Refused(why) => return Err((*why).to_string()),
    };
    let out = Command::new("docker")
        .args([kind, "inspect", id])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("docker: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err("object is no longer present".to_string())
    }
}

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
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&stdout) {
        return Ok(v);
    }
    let lines: Vec<serde_json::Value> = stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    if !lines.is_empty() {
        return Ok(serde_json::Value::Array(lines));
    }
    Err("docker: unavailable (bad output)".to_string())
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

/// How long a live Docker answer is reused before the daemon is asked
/// again. A file touch under the root says nothing about Docker; asking
/// the daemon cost ~0.9 s per observation.
pub const DOCKER_CACHE_TTL_SECS: u64 = 300;

/// `load`, with the live answer cached under the store
/// (`docker_facts.json`) for [`DOCKER_CACHE_TTL_SECS`]. `fresh` forces the
/// daemon (the scheduled `observe`, `--enrich`).
pub fn load_cached(
    facts_path: Option<&Path>,
    store_dir: Option<&Path>,
    fresh: bool,
) -> DockerFacts {
    if facts_path.is_some() {
        return load(facts_path);
    }
    let Some(dir) = store_dir else {
        return load_live();
    };
    let cache = dir.join("docker_facts.json");
    if !fresh
        && let Ok(meta) = std::fs::metadata(&cache)
        && let Ok(age) = meta
            .modified()
            .and_then(|m| m.elapsed().map_err(std::io::Error::other))
        && age.as_secs() < DOCKER_CACHE_TTL_SECS
        && let Ok(text) = std::fs::read_to_string(&cache)
        && let Ok(facts) = serde_json::from_str::<DockerFacts>(&text)
        && facts.unavailable.is_none()
    {
        return facts;
    }
    let facts = load_live();
    if facts.unavailable.is_none()
        && let Ok(text) = serde_json::to_string(&facts)
    {
        let _ = std::fs::create_dir_all(dir);
        let _ = std::fs::write(&cache, text);
    }
    facts
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

/// `docker system df -v` never carries image labels or layer digests
/// (verified against a live daemon), so a live load layers three more
/// best-effort calls on top of the df call for object identity/sizes:
///
/// 1. a batched `docker image inspect <ids> --format json` for labels
///    and `RootFS.Layers` (used for `shared_with`);
/// 2. a batched `docker volume inspect <names> --format json` for
///    volume `CreatedAt` (df's `Driver` is already enough on its own,
///    but `CreatedAt` is inspect-only);
/// 3. `docker ps -a --format json` for the container list, then a
///    batched `docker inspect <container ids> --format json` for the
///    precise `FinishedAt` and volume mount names `ps` doesn't carry.
///
/// Each of these is best-effort: a failure or timeout leaves that one
/// piece of detail unset rather than failing the whole report -- df's
/// object identity/sizes already succeeded, so the report still
/// reconciles.
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
    }

    let volume_names: Vec<&str> = facts.volumes.iter().map(|v| v.name.as_str()).collect();
    if !volume_names.is_empty() {
        let mut args: Vec<&str> = vec!["volume", "inspect"];
        args.extend(volume_names);
        args.extend(["--format", "json"]);
        if let Ok(inspect_value) = run_docker_json(&args, DOCKER_TIMEOUT)
            && let Some(entries) = inspect_value.as_array()
        {
            merge_volume_inspect(&mut facts.volumes, entries);
        }
    }

    if let Ok(ps_value) = run_docker_json(&["ps", "-a", "--format", "json"], DOCKER_TIMEOUT)
        && let Some(rows) = ps_value.as_array()
    {
        let container_ids: Vec<String> = rows
            .iter()
            .filter_map(|r| r.get("ID").and_then(|v| v.as_str()).map(String::from))
            .collect();
        if !container_ids.is_empty() {
            let id_refs: Vec<&str> = container_ids.iter().map(String::as_str).collect();
            let mut args: Vec<&str> = vec!["inspect"];
            args.extend(id_refs);
            args.extend(["--format", "json"]);
            if let Ok(inspect_value) = run_docker_json(&args, DOCKER_TIMEOUT)
                && let Some(entries) = inspect_value.as_array()
            {
                let containers: Vec<ContainerFact> =
                    entries.iter().map(parse_container_inspect).collect();
                join_containers(&mut facts, &containers);
            }
        }
    }

    compute_shared_with(&mut facts.images);

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

    #[test]
    fn dangling_image_has_no_repo_tags() {
        let value = serde_json::json!({
            "Images": [
                {"ID": "sha256:abc", "Repository": "<none>", "Tag": "<none>", "Size": "10"},
                {"ID": "sha256:def", "Repository": "named", "Tag": "latest", "Size": "10"},
            ]
        });
        let facts = parse_value(&value);
        assert!(facts.images[0].dangling);
        assert!(!facts.images[1].dangling);
    }

    #[test]
    fn shared_with_lists_other_images_sharing_a_layer() {
        let value = serde_json::json!({
            "Images": [
                {"ID": "sha256:a", "Repository": "img-a", "Tag": "latest", "Size": "10"},
                {"ID": "sha256:b", "Repository": "img-b", "Tag": "latest", "Size": "10"},
                {"ID": "sha256:c", "Repository": "img-c", "Tag": "latest", "Size": "10"},
            ],
            "ImageInspect": [
                {"Id": "sha256:a", "RepoTags": ["img-a:latest"], "RootFS": {"Layers": ["L1", "L2"]}},
                {"Id": "sha256:b", "RepoTags": ["img-b:latest"], "RootFS": {"Layers": ["L1", "L3"]}},
                {"Id": "sha256:c", "RepoTags": ["img-c:latest"], "RootFS": {"Layers": ["L9"]}},
            ]
        });
        let facts = parse_value(&value);
        let a = facts.images.iter().find(|i| i.id == "sha256:a").unwrap();
        assert_eq!(a.shared_with, vec!["img-b:latest".to_string()]);
        let c = facts.images.iter().find(|i| i.id == "sha256:c").unwrap();
        assert!(c.shared_with.is_empty());
    }

    #[test]
    fn container_join_attaches_state_and_finished_at_to_its_image() {
        let value = serde_json::json!({
            "Images": [
                {"ID": "sha256:aaa", "Repository": "hiphi-relay", "Tag": "staging", "Size": "10"},
            ],
            "Containers": [
                {
                    "Id": "c1",
                    "Name": "/hiphi-staging-relay-1",
                    "Config": {"Image": "hiphi-relay:staging"},
                    "State": {"Status": "exited", "FinishedAt": "2026-09-10T12:00:00Z"},
                    "Mounts": [],
                }
            ]
        });
        let facts = parse_value(&value);
        let image = &facts.images[0];
        assert_eq!(image.containers.len(), 1);
        assert_eq!(image.containers[0].name, "hiphi-staging-relay-1");
        assert_eq!(image.containers[0].state, "exited");
        assert_eq!(
            image.containers[0].finished_at.as_deref(),
            Some("2026-09-10T12:00:00Z")
        );
    }

    #[test]
    fn container_never_finished_normalizes_to_none() {
        let value = serde_json::json!({
            "Images": [{"ID": "sha256:aaa", "Repository": "running-img", "Tag": "latest", "Size": "10"}],
            "Containers": [
                {
                    "Id": "c1",
                    "Name": "/still-running",
                    "Config": {"Image": "running-img:latest"},
                    "State": {"Status": "running", "FinishedAt": "0001-01-01T00:00:00Z"},
                    "Mounts": [],
                }
            ]
        });
        let facts = parse_value(&value);
        assert_eq!(facts.images[0].containers[0].finished_at, None);
    }
}
