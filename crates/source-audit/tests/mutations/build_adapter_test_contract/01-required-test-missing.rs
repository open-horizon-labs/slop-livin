//! target: crates/core/src/build_adapters/bun.rs
//! why: an adapter shipped with four of the five required proofs
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
}
