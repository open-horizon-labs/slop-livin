//! target: crates/core/src/build_adapters/maven.rs
//! why: Maven reaching into Gradle's parser is the Pi/Oh-My-Pi coupling section 13 removed
pub fn sweep_cross_adapter(p: &std::path::Path) -> Option<String> {
    super::gradle::coordinates_from_path(p)
}
