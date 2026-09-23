//! target: crates/core/src/scope.rs
//! expect: accept
//! source: accept
//! why: a store module persisting one of the `JsonFile` variants
/// Accept: the scope file, through the allow-listed writer.
fn sweep_accept_persist(store: &Path, roots: &[String]) -> std::io::Result<()> {
    crate::fs_gate::store::write_json(crate::fs_gate::store::JsonFile::Scope { store }, roots)
}
