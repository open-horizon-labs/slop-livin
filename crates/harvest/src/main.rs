//! Harvests candidate artifact directories from upstream sources and
//! reports what our table is missing. **It never writes the table.**
//!
//! A `.gitignore` entry is evidence that a path is generated, not proof
//! that deleting it is safe: the Python template alone lists `var/`,
//! `instance/` and `lib/`, which in some projects hold a database, a
//! Flask instance config, or a checked-out library. So the output here is
//! a review list for a human and the additions land in `ecosystem.rs` by
//! hand with their provenance.
//!
//! What decides an entry is upstream authority — the ecosystem's own
//! template says it generates this directory — plus our marker gate. One
//! machine's tree decides nothing: `--challenge <root>` exists only to
//! *contradict* a candidate, by finding the name holding git-tracked
//! (authored) content somewhere. Sizes are not evidence; n=1 never is.
//!
//! Sources (vendored, dev-only; the shipped binaries never read them):
//! - `vendor/gitignore` — github/gitignore, one template per ecosystem.
//! - `vendor/linguist-vendor.yml` — github/linguist's vendored paths.
//!
//! ```text
//! cargo run -p slop-livin-harvest                    # gaps, by ecosystem
//! cargo run -p slop-livin-harvest -- --challenge ~/src  # …which ones hold authored content
//! ```

use slop_livin_core::ecosystem::ECOSYSTEMS;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Which upstream template describes which of our ecosystems. Curated:
/// the upstream file names are product names, ours are ecosystem tags,
/// and several templates describe one ecosystem (Gradle and Maven are
/// both JVM). A template with no tag here is an ecosystem we do not
/// model yet; the report lists those separately.
const TEMPLATE_TAGS: &[(&str, &str)] = &[
    ("Rust", "rs"),
    ("Node", "js"),
    ("Nextjs", "js"),
    ("Nestjs", "js"),
    ("bun", "js"),
    ("Yeoman", "js"),
    ("Deno", "deno"),
    ("Python", "py"),
    ("Django", "py"),
    ("Go", "go"),
    ("Java", "java"),
    ("Gradle", "java"),
    ("Maven", "java"),
    ("Kotlin", "java"),
    ("Scala", "scala"),
    ("C", "cpp"),
    ("C++", "cpp"),
    ("CMake", "cpp"),
    ("Autotools", "cpp"),
    ("Objective-C", "swift"),
    ("Swift", "swift"),
    ("Dotnet", "net"),
    ("VisualStudio", "net"),
    ("Ruby", "rb"),
    ("Rails", "rb"),
    ("Elixir", "ex"),
    ("Composer", "php"),
    ("Laravel", "php"),
    ("Symfony", "php"),
    ("Haskell", "hs"),
    ("Dart", "dart"),
    ("Flutter", "dart"),
    ("Zig", "zig"),
    ("Terraform", "tf"),
    ("Unity", "unity"),
    ("UnrealEngine", "ue"),
];

/// Entries that are never a delete candidate whatever a template says:
/// authored content, data, or editor state that happens to be ignored.
const NEVER_AN_ARTIFACT: &[&str] = &[
    "var",
    "instance",
    "lib",
    "lib64",
    "share",
    "docs",
    "doc",
    "log",
    "logs",
    "tmp",
    "temp",
    "data",
    "db",
    "database",
    "media",
    "uploads",
    "static",
    "public",
    "site",
    "config",
    "secrets",
    "env",
    "ENV",
    "bin",
    "sbin",
    "include",
    "man",
    "src",
    "test",
    "tests",
    "spec",
    "examples",
    "vendor-bundle",
    "Backup",
    "backups",
    "output",
    "out",
];

fn directory_entries(template: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for line in template.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() || line.starts_with('!') {
            continue;
        }
        // Directory entries only: a trailing slash is the explicit form.
        let Some(entry) = line.strip_suffix('/') else {
            continue;
        };
        let entry = entry.trim_start_matches('/');
        // One path component, no globs: a name we could match on.
        if entry.is_empty() || entry.contains('/') || entry.contains('*') || entry.contains('?') {
            continue;
        }
        out.insert(entry.to_string());
    }
    out
}

/// Directories named `name` under `root` that hold **git-tracked**
/// content: a candidate that turns up as authored content somewhere is
/// one to think twice about, whatever a template says. This is a
/// falsifier, not a ranking, so it reports names and example paths and
/// no sizes.
fn challenge(root: &Path, names: &BTreeSet<String>) -> BTreeMap<String, Vec<PathBuf>> {
    let mut out: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    let mut seen = 0usize;
    while let Some(dir) = stack.pop() {
        seen += 1;
        if seen > 200_000 {
            break;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_symlink() || !ft.is_dir() {
                continue;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            if name == ".git" {
                continue;
            }
            let path = e.path();
            if names.contains(&name) {
                if has_tracked_content(&path) {
                    out.entry(name).or_default().push(path);
                }
                continue; // never descend into a candidate
            }
            stack.push(path);
        }
    }
    out
}

/// Does git track anything inside this directory? `git ls-files` answers
/// from the containing repository; anything else (no repo, git missing)
/// is "no evidence", never a claim.
fn has_tracked_content(dir: &Path) -> bool {
    std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["ls-files", "--error-unmatch", "."])
        .output()
        .map(|o| o.status.success() && !o.stdout.is_empty())
        .unwrap_or(false)
}

/// The kind an entry most likely is, from the name alone. A proposal for
/// review, never a decision: the reviewer sets the kind in the table.
fn likely_kind(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    const DEPS: &[&str] = &[
        "bower_components",
        "jspm_packages",
        "web_modules",
        "packages",
        "typings",
        "paket-files",
        "vendor",
        "checkouts",
        ".haxelib",
        "elm-stuff",
    ];
    if DEPS.contains(&name) {
        "Deps"
    } else if lower.contains("cache") || lower.contains("tmp") || lower.contains("history") {
        "Cache"
    } else {
        "Build"
    }
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf();
    let dir = root.join("vendor/gitignore");
    let args: Vec<String> = std::env::args().skip(1).collect();
    let challenge_root = args
        .iter()
        .position(|a| a == "--challenge")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from);

    if !dir.exists() {
        eprintln!(
            "vendor/gitignore is missing: git submodule update --init vendor/gitignore\n\
             (dev-only input; the shipped binaries never read it)"
        );
        std::process::exit(2);
    }

    // What we already claim, per ecosystem tag and overall.
    let mut ours: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut ours_any: BTreeSet<&str> = BTreeSet::new();
    for e in ECOSYSTEMS {
        let set: BTreeSet<&str> = e.cleans.iter().map(|(n, _)| *n).collect();
        ours_any.extend(set.iter().copied());
        ours.insert(e.tag, set);
    }

    let mut by_tag: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
    let mut unmodelled: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("read vendor/gitignore")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("gitignore"))
        .collect();
    files.sort();
    for path in &files {
        let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let entries = directory_entries(&text);
        match TEMPLATE_TAGS.iter().find(|(t, _)| *t == stem) {
            Some((_, tag)) => by_tag.entry(tag).or_default().extend(entries),
            None => {
                if !entries.is_empty() {
                    unmodelled.insert(stem, entries);
                }
            }
        }
    }

    // Everything a template proposes that we do not already claim.
    let mut candidates: BTreeSet<String> = BTreeSet::new();
    println!("== ecosystems we model: what github/gitignore lists that we do not ==\n");
    for (tag, entries) in &by_tag {
        let mine = ours.get(*tag).cloned().unwrap_or_default();
        let missing: Vec<&String> = entries
            .iter()
            .filter(|e| !mine.contains(e.as_str()) && !ours_any.contains(e.as_str()))
            .filter(|e| !NEVER_AN_ARTIFACT.contains(&e.as_str()))
            .collect();
        if missing.is_empty() {
            continue;
        }
        candidates.extend(missing.iter().map(|s| (*s).clone()));
        println!(
            "{tag:<6} {}",
            missing
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        );
    }

    println!("\n== ecosystems we do not model at all (template → its directories) ==\n");
    for (name, entries) in &unmodelled {
        let novel: Vec<&String> = entries
            .iter()
            .filter(|e| !ours_any.contains(e.as_str()))
            .filter(|e| !NEVER_AN_ARTIFACT.contains(&e.as_str()))
            .collect();
        if novel.is_empty() {
            continue;
        }
        candidates.extend(novel.iter().map(|s| (*s).clone()));
        println!(
            "{name:<22} {}",
            novel
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        );
    }

    println!(
        "\n{} candidate names. Excluded as never-an-artifact: {}",
        candidates.len(),
        NEVER_AN_ARTIFACT.join(" ")
    );

    println!("\n== proposed kinds, for review (the reviewer decides) ==\n");
    for (tag, entries) in &by_tag {
        let mine = ours.get(*tag).cloned().unwrap_or_default();
        for e in entries {
            if mine.contains(e.as_str())
                || ours_any.contains(e.as_str())
                || NEVER_AN_ARTIFACT.contains(&e.as_str())
            {
                continue;
            }
            println!("{tag:<6} {:<26} {}", e, likely_kind(e));
        }
    }

    if let Some(root) = challenge_root {
        println!(
            "\n== challenged: candidates holding git-tracked content under {} ==\n",
            root.display()
        );
        let found = challenge(&root, &candidates);
        if found.is_empty() {
            println!("(none — no candidate turned up as authored content here)");
        }
        for (name, paths) in &found {
            println!("{name:<26} {} e.g. {}", paths.len(), paths[0].display());
        }
        println!("\nThis contradicts a candidate; it never justifies one. One tree is n=1.");
    }
}
