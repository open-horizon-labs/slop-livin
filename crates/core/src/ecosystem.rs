//! Which ecosystems a checkout belongs to, from the marker files at its
//! root — the detection clean-dev-dirs and kondo run per project, here
//! kept as facts on the project row so a human can filter `type:rust`
//! and a row can wear its `[rs]` tag. A checkout can be several at once
//! (a Tauri app is Rust and Node); every match is kept.

use std::path::Path;

/// (tag, name, root markers). A marker starting with `*.` matches by
/// extension; anything else is an exact file or directory name.
pub const ECOSYSTEMS: &[(&str, &str, &[&str])] = &[
    ("rs", "Rust", &["Cargo.toml"]),
    ("js", "Node.js", &["package.json"]),
    ("deno", "Deno", &["deno.json", "deno.jsonc"]),
    (
        "py",
        "Python",
        &[
            "pyproject.toml",
            "requirements.txt",
            "setup.py",
            "Pipfile",
            "environment.yml",
        ],
    ),
    ("go", "Go", &["go.mod"]),
    (
        "java",
        "JVM",
        &["pom.xml", "build.gradle", "build.gradle.kts"],
    ),
    ("scala", "Scala", &["build.sbt"]),
    ("cpp", "C/C++", &["CMakeLists.txt", "meson.build"]),
    (
        "swift",
        "Swift",
        &["Package.swift", "*.xcodeproj", "*.xcworkspace"],
    ),
    ("net", ".NET", &["*.csproj", "*.fsproj", "*.sln"]),
    ("rb", "Ruby", &["Gemfile"]),
    ("ex", "Elixir", &["mix.exs"]),
    ("php", "PHP", &["composer.json"]),
    ("hs", "Haskell", &["stack.yaml", "*.cabal", "cabal.project"]),
    ("dart", "Dart/Flutter", &["pubspec.yaml"]),
    ("zig", "Zig", &["build.zig"]),
    ("tf", "Terraform", &["*.tf"]),
    (
        "docker",
        "Docker",
        &[
            "Dockerfile",
            "compose.yaml",
            "compose.yml",
            "docker-compose.yml",
            "docker-compose.yaml",
        ],
    ),
    ("unity", "Unity", &["ProjectSettings"]),
    ("ue", "Unreal", &["*.uproject"]),
];

/// Ecosystem tags present at `root`, in table order. Empty when none.
pub fn detect(root: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let names: Vec<String> = entries
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    ECOSYSTEMS
        .iter()
        .filter(|(_, _, markers)| {
            markers.iter().any(|m| match m.strip_prefix("*.") {
                Some(ext) => names.iter().any(|n| n.ends_with(&format!(".{ext}"))),
                None => names.iter().any(|n| n == m),
            })
        })
        .map(|(tag, _, _)| tag.to_string())
        .collect()
}

/// `[rs][js]` for a project row; empty string when nothing was detected.
pub fn tags(ecosystems: &[String]) -> String {
    ecosystems.iter().map(|t| format!("[{t}]")).collect()
}

/// Human name for a tag, for help text and MCP output.
pub fn name_for(tag: &str) -> Option<&'static str> {
    ECOSYSTEMS
        .iter()
        .find(|(t, _, _)| *t == tag)
        .map(|(_, n, _)| *n)
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
        let empty = tempfile::tempdir().unwrap();
        assert!(detect(empty.path()).is_empty());
    }
}
