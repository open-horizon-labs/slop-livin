//! target: crates/core/src/recheck.rs
//! by: audit:sinks_have_no_path_predicates, compile:E0277
//! why: sweep slip -- live_protection open-codes one direction; the audit only inspected the bool-returning predicate
pub fn sweep_live_protection(store_dir: &Path, paths: &[PathBuf]) -> Result<()> {
    let protected = crate::agents::load_protect(store_dir)?;
    for candidate in paths {
        for p in &protected {
            if candidate.starts_with(p) {
                bail!("refused");
            }
        }
    }
    Ok(())
}
