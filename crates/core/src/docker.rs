use crate::entities::*;
use anyhow::Result;
use std::process::Command;
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DockerFact {
    pub id: String,
    pub kind: ArtifactKind,
    pub shared_bytes: u64,
    pub unique_bytes: u64,
    pub project_id: Option<String>,
    pub confidence: Confidence,
    pub meta: FactMeta,
}
pub fn inspect() -> Result<Vec<DockerFact>> {
    let output = Command::new("docker")
        .args(["system", "df", "--format", "{{json .}}"])
        .output();
    let Ok(output) = output else {
        return Ok(vec![]);
    };
    if !output.status.success() {
        return Ok(vec![]);
    };
    let mut facts = vec![];
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let raw: serde_json::Value = serde_json::from_str(line)?;
        let kind = match raw.get("Type").and_then(|v| v.as_str()).unwrap_or("") {
            "Images" => ArtifactKind::DockerImage,
            "Build Cache" => ArtifactKind::DockerCache,
            "Containers" => ArtifactKind::DockerVolume,
            _ => ArtifactKind::Unknown,
        };
        facts.push(DockerFact {
            id: id_for(line),
            kind,
            shared_bytes: 0,
            unique_bytes: raw
                .get("Size")
                .and_then(|v| v.as_str())
                .and_then(|s| s.split_whitespace().next())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
            project_id: None,
            confidence: Confidence::Low,
            meta: FactMeta::now("docker.daemon", Confidence::Low),
        });
    }
    Ok(facts)
}
