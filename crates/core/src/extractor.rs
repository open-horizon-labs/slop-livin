use crate::entities::{ActivitySignal, Artifact, Project};
use anyhow::Result;
use std::path::Path;
pub struct Extraction {
    pub artifacts: Vec<Artifact>,
    pub projects: Vec<Project>,
    pub signals: Vec<ActivitySignal>,
}
pub trait Extractor: Send + Sync {
    fn name(&self) -> &str;
    fn can_handle(&self, path: &Path) -> bool;
    fn extract(&self, path: &Path) -> Result<Extraction>;
}
