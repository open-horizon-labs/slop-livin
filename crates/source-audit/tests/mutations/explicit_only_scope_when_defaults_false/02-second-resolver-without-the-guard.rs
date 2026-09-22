//! target: crates/core/src/scope.rs
//! why: a second `resolve_effective_scope` that infers detectors without asking `detectors_permitted` -- explicit-only scope silently stops being explicit-only
pub mod sweep_scope {
    pub fn resolve_effective_scope(
        env: &super::Environment,
        config: &super::ScanConfig,
        roots: &[std::path::PathBuf],
        registry: &super::Registry,
        at: u64,
    ) -> super::EffectiveScope {
        let _ = (env, config, roots, registry, at);
        unimplemented!("infers from every detector, permitted or not")
    }
}
