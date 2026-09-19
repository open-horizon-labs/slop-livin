//! Which ecosystems a checkout belongs to, from the marker files at its
//! root, and which artifact directories each ecosystem generates — the
//! per-language detection clean-dev-dirs and kondo run per project, kept
//! here as facts: a project row wears its tags/glyphs and can be filtered
//! `type:rust`; an artifact row knows which ecosystem produced it; and an
//! ambiguous directory name (`build`, `dist`, `vendor`, `bin`) counts as
//! an artifact only next to the marker of an ecosystem that generates it.
//! A checkout can be several ecosystems at once (a Tauri app is Rust and
//! Node); every match is kept.

use crate::report::ArtifactKind;
use std::path::Path;

pub struct Ecosystem {
    /// Short tag shown in brackets: `rs`, `js`, `py`, …
    pub tag: &'static str,
    /// Human name for help text and MCP output.
    pub name: &'static str,
    /// Root markers. `*.ext` matches by extension; anything else is an
    /// exact file or directory name.
    pub markers: &'static [&'static str],
    /// Directories this ecosystem generates inside a project, and what
    /// kind each is. `*.suffix` entries match by suffix (`*.egg-info`).
    pub cleans: &'static [(&'static str, ArtifactKind)],
    /// One glyph for the TUI badge column. Emoji here all have default
    /// emoji presentation (display width 2); the two text glyphs (λ, ⬢)
    /// are width 1. Padding is by display width, never by char count.
    pub glyph: &'static str,
    /// Where the project's own name lives, for a checkout with no remote:
    /// (file, how to read it).
    pub name_source: Option<(&'static str, NameField)>,
}

/// How to pull a project name out of a manifest without a parser per
/// format: a `key = "value"` / `key: value` / `"key": "value"` line.
#[derive(Clone, Copy)]
pub enum NameField {
    /// `name = "x"` in a TOML `[package]`/`[project]`/`[tool.poetry]` table.
    TomlName,
    /// `"name": "x"` in JSON.
    JsonName,
    /// `module x` in go.mod.
    GoModule,
    /// `<artifactId>x</artifactId>` in pom.xml.
    PomArtifactId,
    /// `name: x` in YAML (pubspec, package.yaml).
    YamlName,
    /// `name := "x"` in build.sbt.
    SbtName,
    /// `name: "x"` in Package.swift.
    SwiftName,
    /// `app: :x` in mix.exs.
    MixApp,
    /// `project(x ...)` in CMakeLists.txt.
    CmakeProject,
    /// The file's own stem (`Foo.csproj` → `Foo`).
    FileStem,
}

use ArtifactKind::{BuildOutput as Build, Cache, DependencyTree as Deps};

pub const ECOSYSTEMS: &[Ecosystem] = &[
    Ecosystem {
        tag: "rs",
        name: "Rust",
        markers: &["Cargo.toml"],
        cleans: &[("target", Build)],
        glyph: "🦀",
        name_source: Some(("Cargo.toml", NameField::TomlName)),
    },
    Ecosystem {
        tag: "js",
        name: "Node.js",
        markers: &["package.json"],
        cleans: &[
            ("node_modules", Deps),
            ("dist", Build),
            ("build", Build),
            ("out", Build),
            (".next", Build),
            (".nuxt", Build),
            (".svelte-kit", Build),
            (".output", Build),
            (".turbo", Cache),
            (".parcel-cache", Cache),
            (".angular", Cache),
            (".expo", Cache),
            (".metro", Cache),
            ("coverage", Build),
        ],
        glyph: "⬢",
        name_source: Some(("package.json", NameField::JsonName)),
    },
    Ecosystem {
        tag: "deno",
        name: "Deno",
        markers: &["deno.json", "deno.jsonc"],
        cleans: &[("vendor", Deps), ("node_modules", Deps)],
        glyph: "🦕",
        name_source: Some(("deno.json", NameField::JsonName)),
    },
    Ecosystem {
        tag: "py",
        name: "Python",
        markers: &[
            "pyproject.toml",
            "requirements.txt",
            "setup.py",
            "setup.cfg",
            "Pipfile",
            "poetry.lock",
            "environment.yml",
        ],
        cleans: &[
            ("__pycache__", Build),
            (".pytest_cache", Cache),
            (".mypy_cache", Cache),
            (".ruff_cache", Cache),
            ("venv", Deps),
            (".venv", Deps),
            ("build", Build),
            ("dist", Build),
            (".eggs", Build),
            ("*.egg-info", Build),
            (".tox", Cache),
            (".nox", Cache),
            (".coverage", Cache),
            ("__pypackages__", Deps),
            (".pixi", Deps),
        ],
        glyph: "🐍",
        name_source: Some(("pyproject.toml", NameField::TomlName)),
    },
    Ecosystem {
        tag: "go",
        name: "Go",
        markers: &["go.mod"],
        cleans: &[("vendor", Deps)],
        glyph: "🐹",
        name_source: Some(("go.mod", NameField::GoModule)),
    },
    Ecosystem {
        tag: "java",
        name: "JVM",
        markers: &[
            "pom.xml",
            "build.gradle",
            "build.gradle.kts",
            "settings.gradle",
        ],
        cleans: &[("target", Build), ("build", Build), (".gradle", Cache)],
        glyph: "☕",
        name_source: Some(("pom.xml", NameField::PomArtifactId)),
    },
    Ecosystem {
        tag: "scala",
        name: "Scala",
        markers: &["build.sbt"],
        cleans: &[("target", Build)],
        glyph: "🔺",
        name_source: Some(("build.sbt", NameField::SbtName)),
    },
    Ecosystem {
        tag: "cpp",
        name: "C/C++",
        markers: &["CMakeLists.txt", "meson.build", "Makefile"],
        cleans: &[
            ("build", Build),
            ("cmake-build-debug", Build),
            ("cmake-build-release", Build),
        ],
        glyph: "🔧",
        name_source: Some(("CMakeLists.txt", NameField::CmakeProject)),
    },
    Ecosystem {
        tag: "swift",
        name: "Swift",
        markers: &["Package.swift", "*.xcodeproj", "*.xcworkspace", "Podfile"],
        cleans: &[
            (".build", Build),
            (".swiftpm", Cache),
            ("DerivedData", Build),
            ("Pods", Deps),
        ],
        glyph: "🐦",
        name_source: Some(("Package.swift", NameField::SwiftName)),
    },
    Ecosystem {
        tag: "net",
        name: ".NET",
        markers: &["*.csproj", "*.fsproj", "*.vbproj", "*.sln"],
        cleans: &[("bin", Build), ("obj", Build)],
        glyph: "🟣",
        name_source: Some(("*.csproj", NameField::FileStem)),
    },
    Ecosystem {
        tag: "rb",
        name: "Ruby",
        markers: &["Gemfile"],
        cleans: &[(".bundle", Deps), ("vendor", Deps)],
        glyph: "💎",
        name_source: Some(("*.gemspec", NameField::FileStem)),
    },
    Ecosystem {
        tag: "ex",
        name: "Elixir",
        markers: &["mix.exs"],
        cleans: &[
            ("_build", Build),
            ("deps", Deps),
            (".elixir-tools", Cache),
            (".elixir_ls", Cache),
            (".lexical", Cache),
        ],
        glyph: "💧",
        name_source: Some(("mix.exs", NameField::MixApp)),
    },
    Ecosystem {
        tag: "php",
        name: "PHP",
        markers: &["composer.json"],
        cleans: &[("vendor", Deps)],
        glyph: "🐘",
        name_source: Some(("composer.json", NameField::JsonName)),
    },
    Ecosystem {
        tag: "hs",
        name: "Haskell",
        markers: &["stack.yaml", "*.cabal", "cabal.project", "package.yaml"],
        cleans: &[(".stack-work", Build), ("dist-newstyle", Build)],
        glyph: "λ",
        name_source: Some(("package.yaml", NameField::YamlName)),
    },
    Ecosystem {
        tag: "dart",
        name: "Dart/Flutter",
        markers: &["pubspec.yaml"],
        cleans: &[(".dart_tool", Cache), ("build", Build)],
        glyph: "🎯",
        name_source: Some(("pubspec.yaml", NameField::YamlName)),
    },
    Ecosystem {
        tag: "zig",
        name: "Zig",
        markers: &["build.zig"],
        cleans: &[
            ("zig-cache", Cache),
            (".zig-cache", Cache),
            ("zig-out", Build),
        ],
        glyph: "⚡",
        name_source: None,
    },
    Ecosystem {
        tag: "tf",
        name: "Terraform",
        markers: &["*.tf"],
        cleans: &[(".terraform", Deps)],
        glyph: "🌍",
        name_source: None,
    },
    Ecosystem {
        tag: "docker",
        name: "Docker",
        markers: &[
            "Dockerfile",
            "compose.yaml",
            "compose.yml",
            "docker-compose.yml",
            "docker-compose.yaml",
        ],
        cleans: &[],
        glyph: "🐳",
        name_source: None,
    },
    Ecosystem {
        tag: "unity",
        name: "Unity",
        markers: &["ProjectSettings"],
        cleans: &[
            ("Library", Cache),
            ("Temp", Cache),
            ("Obj", Build),
            ("Logs", Cache),
            ("MemoryCaptures", Cache),
            ("Build", Build),
            ("Builds", Build),
        ],
        glyph: "🎲",
        name_source: None,
    },
    Ecosystem {
        tag: "ue",
        name: "Unreal",
        markers: &["*.uproject"],
        cleans: &[
            ("Binaries", Build),
            ("Intermediate", Build),
            ("Saved", Cache),
            ("DerivedDataCache", Cache),
            ("Build", Build),
        ],
        glyph: "🎮",
        name_source: Some(("*.uproject", NameField::FileStem)),
    },
];

fn dir_names(root: &Path) -> Vec<String> {
    std::fs::read_dir(root)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

fn marker_present(names: &[String], marker: &str) -> bool {
    match marker.strip_prefix("*.") {
        Some(ext) => names.iter().any(|n| n.ends_with(&format!(".{ext}"))),
        None => names.iter().any(|n| n == marker),
    }
}

fn detect_in(names: &[String]) -> Vec<&'static Ecosystem> {
    ECOSYSTEMS
        .iter()
        .filter(|e| e.markers.iter().any(|m| marker_present(names, m)))
        .collect()
}

/// Ecosystem tags present at `root`, in table order. Empty when none.
pub fn detect(root: &Path) -> Vec<String> {
    detect_in(&dir_names(root))
        .into_iter()
        .map(|e| e.tag.to_string())
        .collect()
}

/// `[rs][js]` for a project row; empty string when nothing was detected.
pub fn tags(ecosystems: &[String]) -> String {
    ecosystems.iter().map(|t| format!("[{t}]")).collect()
}

pub fn by_tag(tag: &str) -> Option<&'static Ecosystem> {
    ECOSYSTEMS.iter().find(|e| e.tag == tag)
}

/// Human name for a tag, for help text and MCP output.
pub fn name_for(tag: &str) -> Option<&'static str> {
    by_tag(tag).map(|e| e.name)
}

/// The badge glyph for a tag; `?` for a tag the table does not know.
pub fn glyph_for(tag: &str) -> &'static str {
    by_tag(tag).map(|e| e.glyph).unwrap_or("?")
}

fn cleans_name(pattern: &str, name: &str) -> bool {
    match pattern.strip_prefix('*') {
        Some(suffix) => name.ends_with(suffix) && name.len() > suffix.len(),
        None => pattern == name,
    }
}

/// The ecosystem, among `tags` (a project's detected ecosystems), that
/// generates a directory called `name`. When none of the project's own
/// ecosystems claims it, the name alone decides only if exactly one
/// ecosystem in the table generates it (`target` is Rust, JVM and Scala,
/// so a bare `target` stays unattributed; `.stack-work` is Haskell's
/// alone). `None` for a name no ecosystem claims (`.cache`, `.git`) or an
/// ambiguous one nobody at the root vouches for.
pub fn artifact_ecosystem(tags: &[String], name: &str) -> Option<&'static str> {
    if let Some(e) = tags
        .iter()
        .filter_map(|t| by_tag(t))
        .find(|e| e.cleans.iter().any(|(p, _)| cleans_name(p, name)))
    {
        return Some(e.tag);
    }
    let mut claimers = ECOSYSTEMS
        .iter()
        .filter(|e| e.cleans.iter().any(|(p, _)| cleans_name(p, name)));
    match (claimers.next(), claimers.next()) {
        (Some(only), None) => Some(only.tag),
        _ => None,
    }
}

/// [`artifact_ecosystem`] with the artifact's parent directory available:
/// the markers sitting next to the artifact are the strongest evidence
/// (a nested CMake project's `build/` is C/C++ even when the checkout
/// root is a Node monorepo), then the project's own tags, then a name
/// only one ecosystem generates.
pub fn artifact_ecosystem_at(parent: &Path, tags: &[String], name: &str) -> Option<&'static str> {
    let names = dir_names(parent);
    if let Some(e) = detect_in(&names)
        .into_iter()
        .find(|e| e.cleans.iter().any(|(p, _)| cleans_name(p, name)))
    {
        return Some(e.tag);
    }
    artifact_ecosystem(tags, name)
}

/// Bumped whenever the classification tables above change what counts as
/// an artifact. The store records the version it was walked with; a
/// mismatch forces one full walk so rows that no longer classify leave
/// and rows that now do arrive, instead of lingering until something
/// happens to touch their directory.
pub const RULES_VERSION: u32 = 3;

/// Marker-gated classification: `name` inside `parent` is an artifact of
/// the kind an ecosystem declares, if that ecosystem's marker sits in
/// `parent`. This is what lets `build/`, `dist/`, `vendor/`, `bin/` and
/// `obj/` count only where the project type that produces them lives.
pub fn classify_gated(parent: &Path, name: &str) -> Option<ArtifactKind> {
    let names = dir_names(parent);
    detect_in(&names).into_iter().find_map(|e| {
        e.cleans
            .iter()
            .find(|(p, _)| cleans_name(p, name))
            .map(|(_, k)| k.clone())
    })
}

/// The project's own name from its manifest, for a checkout without a
/// remote to name it: first ecosystem in table order whose manifest is
/// present and yields a name.
pub fn manifest_name(root: &Path) -> Option<String> {
    let names = dir_names(root);
    for e in detect_in(&names) {
        let Some((file, field)) = e.name_source else {
            continue;
        };
        let file_name = match file.strip_prefix("*.") {
            Some(ext) => names
                .iter()
                .find(|n| n.ends_with(&format!(".{ext}")))?
                .clone(),
            None => file.to_string(),
        };
        if let NameField::FileStem = field {
            return Path::new(&file_name)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned());
        }
        let Ok(text) = std::fs::read_to_string(root.join(&file_name)) else {
            continue;
        };
        if let Some(n) = extract_name(&text, field) {
            return Some(n);
        }
    }
    None
}

fn quoted(s: &str) -> Option<String> {
    let s = s.trim().trim_end_matches(',').trim();
    let inner = s
        .strip_prefix('"')
        .and_then(|r| r.split('"').next())
        .or_else(|| s.strip_prefix('\'').and_then(|r| r.split('\'').next()))?;
    (!inner.is_empty()).then(|| inner.to_string())
}

fn extract_name(text: &str, field: NameField) -> Option<String> {
    match field {
        NameField::TomlName => {
            // The first `name = "..."` after a [package]/[project]/[tool.poetry]
            // header, so a dependency table's `name` never wins.
            let mut in_table = false;
            for line in text.lines() {
                let l = line.trim();
                if l.starts_with('[') {
                    in_table = matches!(
                        l,
                        "[package]" | "[project]" | "[tool.poetry]" | "[metadata]"
                    );
                }
                if in_table && let Some(rest) = l.strip_prefix("name") {
                    let rest = rest.trim_start();
                    if let Some(v) = rest.strip_prefix('=') {
                        return quoted(v);
                    }
                }
            }
            None
        }
        NameField::JsonName => text.lines().find_map(|l| {
            let l = l.trim();
            let rest = l.strip_prefix("\"name\"")?.trim_start().strip_prefix(':')?;
            quoted(rest)
        }),
        NameField::GoModule => text.lines().find_map(|l| {
            let m = l.trim().strip_prefix("module ")?.trim();
            m.rsplit('/').next().map(str::to_string)
        }),
        NameField::PomArtifactId => text.lines().find_map(|l| {
            let l = l.trim();
            let inner = l
                .strip_prefix("<artifactId>")?
                .strip_suffix("</artifactId>")?;
            (!inner.is_empty()).then(|| inner.to_string())
        }),
        NameField::YamlName => text.lines().find_map(|l| {
            let rest = l.strip_prefix("name:")?.trim();
            let v = quoted(rest).unwrap_or_else(|| rest.to_string());
            (!v.is_empty()).then_some(v)
        }),
        NameField::SbtName => text.lines().find_map(|l| {
            let rest = l
                .trim()
                .strip_prefix("name")?
                .trim_start()
                .strip_prefix(":=")?;
            quoted(rest)
        }),
        NameField::SwiftName => text.lines().find_map(|l| {
            let rest = l.trim().strip_prefix("name:")?;
            quoted(rest)
        }),
        NameField::MixApp => text.lines().find_map(|l| {
            let rest = l.trim().strip_prefix("app:")?.trim().strip_prefix(':')?;
            let v: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            (!v.is_empty()).then_some(v)
        }),
        NameField::CmakeProject => text.lines().find_map(|l| {
            let rest = l.trim().strip_prefix("project(")?;
            let v: String = rest
                .trim_start()
                .chars()
                .take_while(|c| !c.is_whitespace() && *c != ')')
                .collect();
            (!v.is_empty()).then_some(v)
        }),
        NameField::FileStem => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_several_ecosystems_at_one_root() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("Cargo.toml"), "").unwrap();
        std::fs::write(tmp.path().join("package.json"), "{}").unwrap();
        std::fs::write(tmp.path().join("App.csproj"), "").unwrap();
        std::fs::write(tmp.path().join("main.tf"), "").unwrap();
        assert_eq!(detect(tmp.path()), vec!["rs", "js", "net", "tf"]);
        assert_eq!(tags(&detect(tmp.path())), "[rs][js][net][tf]");
        assert_eq!(name_for("hs"), Some("Haskell"));
        assert_eq!(glyph_for("rs"), "🦀");
        let empty = tempfile::tempdir().unwrap();
        assert!(detect(empty.path()).is_empty());
    }

    #[test]
    fn ambiguous_names_classify_only_next_to_their_marker() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(classify_gated(tmp.path(), "build"), None);
        assert_eq!(classify_gated(tmp.path(), "vendor"), None);
        std::fs::write(tmp.path().join("CMakeLists.txt"), "project(x)").unwrap();
        assert_eq!(classify_gated(tmp.path(), "build"), Some(Build));
        assert_eq!(classify_gated(tmp.path(), "vendor"), None);
        std::fs::write(tmp.path().join("go.mod"), "module a/b").unwrap();
        assert_eq!(classify_gated(tmp.path(), "vendor"), Some(Deps));
        std::fs::write(tmp.path().join("setup.py"), "").unwrap();
        assert_eq!(classify_gated(tmp.path(), "foo.egg-info"), Some(Build));
        assert_eq!(classify_gated(tmp.path(), ".egg-info"), None);
    }

    #[test]
    fn artifact_ecosystem_prefers_the_projects_own_tags() {
        let both = vec!["py".to_string(), "js".to_string()];
        assert_eq!(artifact_ecosystem(&both, "dist"), Some("py"));
        assert_eq!(artifact_ecosystem(&[], ".stack-work"), Some("hs"));
        assert_eq!(
            artifact_ecosystem(&[], "target"),
            None,
            "rs/java/scala all claim it"
        );
        assert_eq!(
            artifact_ecosystem(&[], "node_modules"),
            None,
            "js and deno claim it"
        );
        assert_eq!(artifact_ecosystem(&["rs".into()], "target"), Some("rs"));
        assert_eq!(artifact_ecosystem(&[], ".cache"), None);
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("CMakeLists.txt"), "").unwrap();
        assert_eq!(
            artifact_ecosystem_at(tmp.path(), &["js".into()], "build"),
            Some("cpp"),
            "the marker next to the artifact outranks the root's tags"
        );
    }

    #[test]
    fn manifest_names_from_common_formats() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("Cargo.toml"),
            "[package]\nname = \"crabby\"\n[dependencies]\nname = \"no\"\n",
        )
        .unwrap();
        assert_eq!(manifest_name(tmp.path()).as_deref(), Some("crabby"));
        let js = tempfile::tempdir().unwrap();
        std::fs::write(
            js.path().join("package.json"),
            "{\n  \"name\": \"@org/pkg\",\n}",
        )
        .unwrap();
        assert_eq!(manifest_name(js.path()).as_deref(), Some("@org/pkg"));
        let go = tempfile::tempdir().unwrap();
        std::fs::write(go.path().join("go.mod"), "module github.com/a/gopher\n").unwrap();
        assert_eq!(manifest_name(go.path()).as_deref(), Some("gopher"));
        let net = tempfile::tempdir().unwrap();
        std::fs::write(net.path().join("Shop.csproj"), "").unwrap();
        assert_eq!(manifest_name(net.path()).as_deref(), Some("Shop"));
        let ex = tempfile::tempdir().unwrap();
        std::fs::write(ex.path().join("mix.exs"), "  app: :phoenix_app,\n").unwrap();
        assert_eq!(manifest_name(ex.path()).as_deref(), Some("phoenix_app"));
    }
}
