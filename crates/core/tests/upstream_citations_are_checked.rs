//! Real provenance for the support matrix, replacing the 30-character
//! length check.
//!
//! What CI used to guarantee about a `Supported` row's "Verified
//! against" cell was that the string was longer than 30 characters
//! (`agent_matrix_matches_docs.rs`) and non-empty
//! (`matrix.rs::a_supported_row_cites_what_confirmed_it`). The
//! 2026-09-22 independent re-review fetched all fourteen rows' citations
//! and found six that did not establish what the row claimed -- four of
//! them consequential in the adapter (a dead path, silently uncounted
//! bytes, a missing protected store, and parsed field names that appear
//! in no cited source) -- with the suite green throughout. A length
//! check on a provenance string is not provenance.
//!
//! So each cited upstream file is **vendored** as a small excerpt under
//! `crates/core/tests/fixtures/upstream/<tool>/<commit-prefix>/`, pinned
//! by commit, recorded with its blake3 digest, and listed in `citations.toml`
//! together with the symbols the row's claim depends on. This test
//! greps the vendored excerpt for every claimed symbol and fails naming
//! the citation that does not support its claim. Nothing is fetched at
//! test time: the check is offline, deterministic, and breaks when a
//! claim is edited without re-reading the source.
//!
//! The excerpts are a few lines each, kept for verification, with the
//! upstream repository, commit and path recorded beside them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use swamp_core::agents::matrix::{MATRIX, SupportLevel};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/upstream")
}

#[derive(Debug)]
struct Citation {
    tool: String,
    repo: String,
    commit: String,
    path: String,
    symbols: Vec<String>,
    excerpt: String,
    blake3: String,
}

/// A deliberately tiny TOML-ish reader: `[[citation]]` blocks of
/// `key = "value"` and `symbols = ["a", "b"]`. A real TOML dependency in
/// a test would be fine too; this keeps the manifest format obvious to
/// anyone editing it and the failure messages specific.
fn load_manifest(path: &Path) -> Vec<Citation> {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{} could not be read: {e}", path.display()));
    let mut out = Vec::new();
    let mut cur: BTreeMap<String, String> = BTreeMap::new();
    let mut symbols: Vec<String> = Vec::new();
    let flush =
        |cur: &mut BTreeMap<String, String>, symbols: &mut Vec<String>, out: &mut Vec<Citation>| {
            if cur.is_empty() {
                return;
            }
            let get = |k: &str| -> String {
                cur.get(k)
                    .unwrap_or_else(|| panic!("citation block is missing `{k}`: {cur:?}"))
                    .clone()
            };
            out.push(Citation {
                tool: get("tool"),
                repo: get("repo"),
                commit: get("commit"),
                path: get("path"),
                symbols: std::mem::take(symbols),
                excerpt: get("excerpt"),
                blake3: get("blake3"),
            });
            cur.clear();
        };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line == "[[citation]]" {
            flush(&mut cur, &mut symbols, &mut out);
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().to_string();
        let value = value.trim();
        if key == "symbols" {
            symbols = value
                .trim_start_matches('[')
                .trim_end_matches(']')
                .split("\",")
                .map(|s| s.trim().trim_matches('"').trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            continue;
        }
        cur.insert(key, value.trim_matches('"').to_string());
    }
    flush(&mut cur, &mut symbols, &mut out);
    out
}

/// blake3 rather than SHA-256 only because it is already a dependency of
/// this crate: adding a hash crate to check a vendored excerpt would be
/// a dependency for a digest, and the property needed here -- an edited
/// excerpt no longer matches -- is the same.
fn digest_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

/// Every vendored excerpt hashes to the digest recorded beside it, and
/// contains every symbol the matrix claim depends on. An excerpt edited
/// to make a claim fit fails on the hash; a claim edited past what the
/// excerpt says fails on the symbol.
#[test]
fn every_vendored_citation_contains_the_symbol_it_is_cited_for() {
    let dir = fixtures();
    let citations = load_manifest(&dir.join("citations.toml"));
    assert!(
        !citations.is_empty(),
        "{}/citations.toml lists no citations",
        dir.display()
    );
    let mut problems: Vec<String> = Vec::new();
    for c in &citations {
        let excerpt = dir.join(&c.excerpt);
        let Ok(bytes) = std::fs::read(&excerpt) else {
            problems.push(format!(
                "{}: cited file {} @ {} has no vendored excerpt at {}",
                c.tool,
                c.path,
                c.commit,
                excerpt.display()
            ));
            continue;
        };
        let got = digest_hex(&bytes);
        if got != c.blake3 {
            problems.push(format!(
                "{}: {} hashes to {got}, manifest says {}; an excerpt edited to make a claim fit \
                 is not a citation",
                c.tool,
                excerpt.display(),
                c.blake3
            ));
            continue;
        }
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            !c.symbols.is_empty(),
            "{}: citation for {} lists no symbols; a citation with nothing to check is the \
             30-character check again",
            c.tool,
            c.path
        );
        for sym in &c.symbols {
            if !text.contains(sym) {
                problems.push(format!(
                    "{}: {} ({} @ {}) does not contain `{sym}`",
                    c.tool, c.path, c.repo, c.commit
                ));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// Every `Supported` row's citations pin a commit or a date, and every
/// citation that names a repository file has a vendored excerpt. A row
/// whose provenance is "read during chunk #93" cannot be re-checked by
/// anyone, which is what five rows said.
#[test]
fn every_supported_row_pins_its_citation_and_vendors_the_file() {
    let dir = fixtures();
    let citations = load_manifest(&dir.join("citations.toml"));
    let mut problems: Vec<String> = Vec::new();
    for entry in MATRIX {
        if entry.support != SupportLevel::Supported {
            continue;
        }
        assert!(
            !entry.verification.is_empty(),
            "{} is Supported and cites nothing",
            entry.display_name
        );
        for v in entry.verification {
            let pinned = v.revision.split_whitespace().any(|w| {
                let w = w.trim_end_matches(&[',', ';'][..]);
                (w.len() == 40 && w.chars().all(|c| c.is_ascii_hexdigit()))
                    || (w.len() == 10
                        && w.as_bytes()[4] == b'-'
                        && w.as_bytes()[7] == b'-'
                        && w.chars().filter(|c| c.is_ascii_digit()).count() == 8)
            });
            if !pinned {
                problems.push(format!(
                    "{}: revision `{}` pins neither a commit nor a date, so the citation cannot \
                     be re-checked",
                    entry.display_name, v.revision
                ));
            }
            // A citation that names a repository file (not a docs page)
            // must be vendored so the symbol check above can run on it.
            let is_repo_file = !v.source.starts_with("http") || v.source.contains("/blob/");
            if is_repo_file {
                let tail = v.source.rsplit("/blob/").next().unwrap_or(v.source);
                let matched = citations.iter().any(|c| {
                    tail.ends_with(&c.path) || c.path.ends_with(tail) || v.source.contains(&c.path)
                });
                if !matched {
                    problems.push(format!(
                        "{}: citation `{}` names an upstream file with no vendored excerpt in \
                         citations.toml",
                        entry.display_name, v.source
                    ));
                }
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
