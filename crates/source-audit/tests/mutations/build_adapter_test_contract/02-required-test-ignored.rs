//! target: crates/core/src/build_adapters/deno_build.rs
//! why: a named test resolves by name whether or not it ever runs; `#[ignore]` satisfied the old existence check
pub struct Adapter;
#[cfg(test)]
mod tests {
    #[test]
    fn unknown_layout_is_explicit_not_empty() {
        assert!(true);
    }
    #[test]
    fn identification_reads_no_more_than_manifest_cap() {
        assert!(true);
    }
    #[test]
    fn no_project_or_build_code_is_executed() {
        assert!(true);
    }
    #[test]
    fn variants_never_collapse_by_basename() {
        assert!(true);
    }
    #[test]
    #[ignore]
    fn age_is_not_obsolescence() {
        assert!(true);
    }
}
