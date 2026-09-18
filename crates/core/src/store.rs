use crate::entities::*;
use anyhow::{Context, Result};
use arrow_array::{ArrayRef, RecordBatch, StringArray, UInt64Array};
use arrow_schema::{DataType, Field, Schema};
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::{WriterProperties, WriterVersion};
use std::{
    fs::{self, File},
    path::PathBuf,
    sync::Arc,
};

#[derive(Debug, Clone)]
pub struct Store {
    pub root: PathBuf,
}

impl Store {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        fs::create_dir_all(&root)?;
        Ok(Self { root })
    }
    fn path(&self, volume: u64) -> PathBuf {
        self.root.join(format!("volume-{volume}.parquet"))
    }
    pub fn write(&self, observation: &Observation) -> Result<()> {
        let schema = Arc::new(Schema::new(vec![
            Field::new("path", DataType::Utf8, false),
            Field::new("bytes", DataType::UInt64, false),
            Field::new("kind", DataType::Utf8, false),
            Field::new("project_id", DataType::Utf8, true),
            Field::new("observed_at", DataType::UInt64, false),
        ]));
        let paths: Vec<String> = observation
            .artifacts
            .iter()
            .map(|a| a.path.display().to_string())
            .collect();
        let bytes: Vec<u64> = observation.artifacts.iter().map(|a| a.bytes).collect();
        let kinds: Vec<String> = observation
            .artifacts
            .iter()
            .map(|a| format!("{:?}", a.kind))
            .collect();
        let projects: Vec<Option<String>> = observation
            .artifacts
            .iter()
            .map(|a| a.project_id.clone())
            .collect();
        let observed = vec![observation.observed_at; observation.artifacts.len()];
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(paths)) as ArrayRef,
                Arc::new(UInt64Array::from(bytes)),
                Arc::new(StringArray::from(kinds)),
                Arc::new(StringArray::from(projects)),
                Arc::new(UInt64Array::from(observed)),
            ],
        )?;
        let file = File::create(self.path(observation.volume_id))?;
        let properties = WriterProperties::builder()
            .set_compression(Compression::ZSTD(Default::default()))
            .set_writer_version(WriterVersion::PARQUET_2_0)
            .build();
        let mut writer = ArrowWriter::try_new(file, schema, Some(properties))?;
        writer.write(&batch)?;
        writer.close()?;
        Ok(())
    }
    pub fn read_observation(&self, volume: u64) -> Result<Vec<Artifact>> {
        let path = self.path(volume);
        let bytes = fs::read(&path).with_context(|| format!("read {}", path.display()))?;
        if bytes.len() < 4 || &bytes[bytes.len() - 4..] != b"PAR1" {
            anyhow::bail!("store is not a Parquet file")
        }
        Ok(Vec::new())
    }
    pub fn volume_path(&self, volume: u64) -> PathBuf {
        self.path(volume)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::{ScanOptions, observation};
    use tempfile::tempdir;
    #[test]
    fn writes_real_parquet() {
        let d = tempdir().unwrap();
        let obs = observation(&ScanOptions {
            roots: vec![d.path().to_path_buf()],
            cross_device: false,
            max_depth: Some(1),
        })
        .unwrap();
        let store = Store::open(d.path().join("state")).unwrap();
        store.write(&obs).unwrap();
        let data = fs::read(store.volume_path(obs.volume_id)).unwrap();
        assert_eq!(&data[data.len() - 4..], b"PAR1");
    }
}
