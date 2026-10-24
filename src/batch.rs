//! A batch is a progress ledger, never an atomic filesystem transaction.
use crate::{
    engine::{checked_read, checked_write, Record},
    Result,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Item {
    pub source: PathBuf,
    pub transaction_id: Option<String>,
    pub status: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Batch {
    pub version: u32,
    pub id: String,
    pub kind: String,
    pub phase: String,
    pub items: Vec<Item>,
}
pub(crate) struct Tracker {
    path: PathBuf,
    value: Batch,
}
impl Tracker {
    pub fn start(
        store: &Path,
        kind: &str,
        sources: impl IntoIterator<Item = PathBuf>,
    ) -> Result<Self> {
        fs::create_dir_all(store)?;
        let id = format!(
            "{}-{}",
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos(),
            std::process::id()
        );
        let value = Batch {
            version: 1,
            id: id.clone(),
            kind: kind.into(),
            phase: "running".into(),
            items: sources
                .into_iter()
                .map(|source| Item {
                    source,
                    transaction_id: None,
                    status: "pending".into(),
                })
                .collect(),
        };
        let tracker = Self {
            path: store.join(format!("{id}.batch")),
            value,
        };
        tracker.save()?;
        Ok(tracker)
    }
    fn save(&self) -> Result<()> {
        checked_write(&self.path, &self.value)
    }
    pub fn begin(&mut self, index: usize) -> Result<()> {
        self.value.items[index].status = "running".into();
        self.save()
    }
    pub fn transaction(&mut self, index: usize, id: &str) -> Result<()> {
        self.value.items[index].transaction_id = Some(id.into());
        self.save()
    }
    pub fn finish(&mut self, index: usize) -> Result<()> {
        self.value.items[index].status = "completed".into();
        self.save()
    }
    pub fn complete(&mut self) -> Result<()> {
        if self.value.items.iter().all(|i| i.status == "completed") {
            self.value.phase = "completed".into();
        }
        self.save()
    }
}
impl Drop for Tracker {
    fn drop(&mut self) {
        if self.value.phase != "completed" {
            self.value.phase = "interrupted".into();
            let _ = self.save();
        }
    }
}
pub fn history(store: &Path) -> Result<Vec<Batch>> {
    let mut out = vec![];
    if !store.exists() {
        return Ok(out);
    }
    for file in fs::read_dir(store)? {
        let path = file?.path();
        if path.extension().is_some_and(|s| s == "batch") {
            let mut batch: Batch = checked_read(&path)?;
            // A crash can happen after a directory commits and before its batch ledger updates.
            // Derive current per-item outcomes from durable directory records, read-only.
            for item in &mut batch.items {
                if let Some(id) = &item.transaction_id {
                    let record: Record = checked_read(&crate::engine::record_path(store, id)?)?;
                    item.status = match record.phase.as_str() {
                        "completed" | "restored" => "completed",
                        "rolled_back" => "rolled_back",
                        _ => "interrupted",
                    }
                    .into();
                }
            }
            if batch.items.iter().all(|i| i.status == "completed") {
                batch.phase = "completed".into();
            } else if batch.phase == "completed" {
                batch.phase = "interrupted".into();
            }
            out.push(batch);
        }
    }
    out.sort_by(|a, b| b.id.cmp(&a.id));
    Ok(out)
}
