//! target: crates/core/src/build_adapters/registry.rs
//! mode: replace
//! why: a duplicate registration identifies every Node container twice
use super::BuildAdapter;
use super::{cargo, gradle, maven, node};

pub struct Registry {
    adapters: Vec<Box<dyn BuildAdapter>>,
}

impl Registry {
    pub fn with_builtins() -> Self {
        Self {
            adapters: vec![
                Box::new(cargo::Adapter),
                Box::new(node::Adapter),
                Box::new(gradle::Adapter),
                Box::new(maven::Adapter),
                Box::new(node::Adapter),
            ],
        }
    }
}
