//! Neutral parsing shared by the Gradle and Maven adapters.
//!
//! The allow-listed helper of
//! `.oh/guardrails/build-adapters-are-pluggable.md`. It exists so
//! `maven.rs` never calls into `gradle.rs` and vice versa: Gradle's
//! `modules-2` cache and Maven's local repository both lay artifacts out
//! by Maven coordinates, and the alternative to a neutral module is one
//! adapter reaching into the other -- exactly the Pi/Oh-My-Pi coupling
//! section 13 had to remove from the agent side.
//!
//! Nothing here knows which adapter called it, and nothing here reads a
//! file. It is string and path arithmetic over layouts both tools
//! document.

/// Maven coordinates, as far as a path can establish them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Coordinates {
    /// Dotted group id (`org.apache.commons`), from the directory
    /// segments above the artifact.
    pub group: String,
    pub artifact: String,
    /// `None` when the path stops at the artifact directory. A version
    /// is never invented from a filename.
    pub version: Option<String>,
}

impl Coordinates {
    pub fn display(&self) -> String {
        match &self.version {
            Some(v) => format!("{}:{}:{}", self.group, self.artifact, v),
            None => format!("{}:{}", self.group, self.artifact),
        }
    }
}

/// Reads coordinates out of a repository-relative path.
///
/// Maven's local repository and Gradle's `modules-2` both lay out
/// `<group as directories>/<artifact>/<version>/`, so a path of at least
/// three segments gives group, artifact and version, and one of exactly
/// two gives group and artifact with an unknown version.
///
/// `Gradle`'s `modules-2` differs in one way: its group is a *single*
/// dotted segment rather than one directory per dotted component. Both
/// shapes are accepted, and `dotted_group` selects which.
pub fn coordinates_from_relative(rel: &str, dotted_group: bool) -> Option<Coordinates> {
    let parts: Vec<&str> = rel.split('/').filter(|s| !s.is_empty()).collect();
    if dotted_group {
        // <group.with.dots>/<artifact>[/<version>]
        let group = (*parts.first()?).to_string();
        if !group.contains('.') && parts.len() < 2 {
            return None;
        }
        let artifact = (*parts.get(1)?).to_string();
        return Some(Coordinates {
            group,
            artifact,
            version: parts.get(2).map(|v| (*v).to_string()),
        });
    }
    if parts.len() < 2 {
        return None;
    }
    // The last segment is a version only when it looks like one: a
    // digit-led token. `commons-io/commons-io` has no version, and
    // guessing one from `commons-io` would be an invented identity.
    let (head, version) = if parts.len() >= 3 && looks_like_version(parts[parts.len() - 1]) {
        (&parts[..parts.len() - 1], Some(parts[parts.len() - 1]))
    } else {
        (&parts[..], None)
    };
    if head.len() < 2 {
        return None;
    }
    let artifact = head[head.len() - 1].to_string();
    let group = head[..head.len() - 1].join(".");
    Some(Coordinates {
        group,
        artifact,
        version: version.map(str::to_string),
    })
}

/// Whether a path segment is plausibly a version: it starts with a digit
/// (`1.2.3`, `2024.1`, `4.13.2-SNAPSHOT`) or is a dated snapshot.
///
/// Deliberately conservative. A false positive turns an artifact name
/// into a version and produces coordinates nobody can look up; a false
/// negative leaves the version explicitly unknown, which is the honest
/// failure.
pub fn looks_like_version(segment: &str) -> bool {
    segment.chars().next().is_some_and(|c| c.is_ascii_digit())
}

/// Where a JVM build tool's user home is, given the environment the
/// caller resolved.
///
/// Adapters never read the environment themselves; this takes the value
/// and applies the tool's documented default, so a fixture can inject
/// one.
pub fn user_home(
    explicit: Option<&std::path::Path>,
    home: &std::path::Path,
    dir: &str,
) -> std::path::PathBuf {
    explicit
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| home.join(dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_maven_repository_path_gives_all_three_coordinates() {
        let c = coordinates_from_relative("org/apache/commons/commons-lang3/3.12.0", false)
            .expect("coordinates");
        assert_eq!(c.group, "org.apache.commons");
        assert_eq!(c.artifact, "commons-lang3");
        assert_eq!(c.version.as_deref(), Some("3.12.0"));
        assert_eq!(c.display(), "org.apache.commons:commons-lang3:3.12.0");
    }

    #[test]
    fn a_path_stopping_at_the_artifact_leaves_the_version_unknown() {
        let c =
            coordinates_from_relative("org/apache/commons/commons-lang3", false).expect("coords");
        assert_eq!(c.artifact, "commons-lang3");
        assert_eq!(
            c.version, None,
            "a version is never invented from an artifact name"
        );
    }

    #[test]
    fn a_gradle_modules_path_keeps_its_dotted_group() {
        let c = coordinates_from_relative("com.squareup.okhttp3/okhttp/4.12.0", true)
            .expect("coordinates");
        assert_eq!(c.group, "com.squareup.okhttp3");
        assert_eq!(c.artifact, "okhttp");
        assert_eq!(c.version.as_deref(), Some("4.12.0"));
    }

    #[test]
    fn a_non_numeric_last_segment_is_not_read_as_a_version() {
        // `commons-io/commons-io` is group `commons-io`, artifact
        // `commons-io`, no version -- not artifact `commons-io` at
        // version `commons-io`.
        let c = coordinates_from_relative("commons-io/commons-io", false).expect("coordinates");
        assert_eq!(c.group, "commons-io");
        assert_eq!(c.artifact, "commons-io");
        assert_eq!(c.version, None);
    }

    #[test]
    fn a_snapshot_version_is_recognised() {
        assert!(looks_like_version("4.13.2-SNAPSHOT"));
        assert!(looks_like_version("2024.1"));
        assert!(!looks_like_version("commons-io"));
        assert!(!looks_like_version("SNAPSHOT"));
    }
}
