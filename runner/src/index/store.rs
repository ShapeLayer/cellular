//! Locating, reading, writing and verifying a project index directory.

use anyhow::{Context, Result, bail};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use crate::config::{CELLULAR_DIR, profile_dir, profile_index_dir};
use crate::index::format::{
    BLOB_HEADER_SIZE, BlobMeta, MAX_FILE_SIZE, RECORD_HEADER_SIZE, decode_blob, decode_index,
    decode_snapshot, encode_blob, encode_index, hex, md5_of,
};
use crate::model::Snapshot;

pub const INDEX_FILE: &str = "INDEX";
pub const BLOBS_DIR: &str = "BLOBS";
pub const BLOB_PREFIX: &str = "BLOB_";
pub const PROFILE_MAP_FILE: &str = "INDEX.json";

/// Held while `INDEX` and `BLOBS/` are rewritten, and by any reader that needs
/// the two to agree with each other.
///
/// [`IndexStore::write_records`] writes the blob files one at a time and
/// `INDEX` last, so there is a moment in every write where the two disagree. A
/// build flushes every couple of seconds, which makes that moment come around
/// often enough to matter for a reader as picky as the exporter. Everything
/// that writes here runs in this process — the build runs on a thread of it —
/// so an in-process lock covers the whole race.
static STORE_WRITES: Mutex<()> = Mutex::new(());

/// Wait until no one is part way through rewriting the store.
///
/// A caller holds the guard for as long as it needs `INDEX` and the blobs to
/// stay in step. Poisoning is ignored: what the lock guards lives on disk, and
/// a writer that panicked has already reported it.
pub fn lock_store() -> MutexGuard<'static, ()> {
    STORE_WRITES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The directory holding `INDEX` and `BLOBS/`.
#[derive(Debug, Clone)]
pub struct IndexStore {
    pub root: PathBuf,
}

impl IndexStore {
    /// Resolve the index directory for a project.
    ///
    /// Index data always lives at `~/.cellular/index/<uuid>`, with the path to
    /// uuid pair recorded in `~/.cellular/INDEX.json` so two projects that
    /// share a directory name do not collide. A project-local `.cellular` still
    /// holds the project `config.json` (see
    /// [`crate::config::project_config_path`]), but never generated data.
    ///
    /// With `create`, a project that has no uuid yet is assigned one and the
    /// mapping is written. Without it, such a project resolves to `None`:
    /// reading or verifying an index must not create state on disk.
    pub fn locate(project_root: &Path, create: bool) -> Result<Option<Self>> {
        let root = match profile_uuid_for(project_root, create)? {
            Some(uuid) => profile_index_dir()?.join(uuid),
            None => return Ok(None),
        };
        Ok(Some(IndexStore { root }))
    }

    /// Like [`IndexStore::locate`] with `create`, which always resolves.
    pub fn locate_for_writing(project_root: &Path) -> Result<Self> {
        Ok(Self::locate(project_root, true)?.expect("locate always resolves when creating"))
    }

    /// The project-local `.cellular` directory. Index data is not kept here —
    /// [`IndexStore::locate`] always resolves under the profile — but the path
    /// stays available for the project-scoped files that do live in it.
    #[allow(dead_code)]
    pub fn project_dir(project_root: &Path) -> PathBuf {
        project_root.join(CELLULAR_DIR)
    }

    pub fn index_path(&self) -> PathBuf {
        self.root.join(INDEX_FILE)
    }

    pub fn blobs_dir(&self) -> PathBuf {
        self.root.join(BLOBS_DIR)
    }

    pub fn blob_path(&self, number: usize) -> PathBuf {
        self.blobs_dir().join(format!("{BLOB_PREFIX}{number}"))
    }

    pub fn exists(&self) -> bool {
        self.index_path().exists()
    }

    /// Read `INDEX` and check every blob's size and digest against it.
    pub fn verify(&self) -> Result<VerifyReport> {
        let mut report = VerifyReport::default();
        if !self.exists() {
            report.problems.push("no INDEX file was found".to_string());
            return Ok(report);
        }

        let index_bytes = std::fs::read(self.index_path())
            .with_context(|| format!("failed to read {}", self.index_path().display()))?;
        let metas = match decode_index(&index_bytes) {
            Ok(metas) => metas,
            Err(error) => {
                report
                    .problems
                    .push(format!("INDEX is unreadable: {error}"));
                return Ok(report);
            }
        };
        report.blob_count = metas.len();

        for (number, meta) in metas.iter().enumerate() {
            let path = self.blob_path(number);
            let Ok(bytes) = std::fs::read(&path) else {
                report
                    .problems
                    .push(format!("{BLOB_PREFIX}{number} is missing"));
                continue;
            };
            if bytes.len() as u64 != meta.size {
                report.problems.push(format!(
                    "{BLOB_PREFIX}{number} is {} bytes, INDEX expects {}",
                    bytes.len(),
                    meta.size
                ));
                continue;
            }
            let digest = md5_of(&bytes);
            if digest != meta.md5 {
                report.problems.push(format!(
                    "{BLOB_PREFIX}{number} has md5 {}, INDEX expects {}",
                    hex(&digest),
                    hex(&meta.md5)
                ));
                continue;
            }
            match decode_blob(&bytes) {
                Ok(records) => report.record_count += records.len(),
                Err(error) => report
                    .problems
                    .push(format!("{BLOB_PREFIX}{number} is unreadable: {error}")),
            }
        }

        // Blob files past the recorded count are leftovers from an older build.
        let mut extra = metas.len();
        while self.blob_path(extra).exists() {
            report
                .problems
                .push(format!("{BLOB_PREFIX}{extra} is not listed in INDEX"));
            extra += 1;
        }

        Ok(report)
    }

    /// All snapshot record payloads currently stored, in blob order.
    pub fn load_records(&self) -> Result<Vec<Vec<u8>>> {
        if !self.exists() {
            return Ok(Vec::new());
        }
        let index_bytes = std::fs::read(self.index_path())
            .with_context(|| format!("failed to read {}", self.index_path().display()))?;
        let metas = decode_index(&index_bytes)?;
        let mut records = Vec::new();
        for number in 0..metas.len() {
            let path = self.blob_path(number);
            let bytes = std::fs::read(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            records.extend(decode_blob(&bytes)?);
        }
        Ok(records)
    }

    /// Every record that can still be read, skipping blobs that cannot and
    /// ignoring `INDEX` entirely. Used when recovering from damage, where one
    /// unreadable blob must not cost the records held by the others.
    pub fn load_records_best_effort(&self) -> Vec<Vec<u8>> {
        let mut records = Vec::new();
        let mut number = 0;
        while self.blob_path(number).exists() {
            if let Ok(bytes) = std::fs::read(self.blob_path(number))
                && let Ok(mut found) = decode_blob(&bytes)
            {
                records.append(&mut found);
            }
            number += 1;
        }
        records
    }

    pub fn load_snapshots(&self) -> Result<Vec<Snapshot>> {
        self.load_records().and_then(|records| {
            records
                .iter()
                .map(|record| decode_snapshot(record))
                .collect()
        })
    }

    /// Pack records into blob files no larger than [`MAX_FILE_SIZE`], then
    /// rewrite `INDEX`. Blob files left over from a previous build are removed.
    ///
    /// Holds [`STORE_WRITES`] for the whole rewrite so a reader never catches
    /// the store between the blobs and the `INDEX` that describes them.
    pub fn write_records(&self, records: &[Vec<u8>]) -> Result<WriteReport> {
        let _guard = lock_store();
        std::fs::create_dir_all(self.blobs_dir())
            .with_context(|| format!("failed to create {}", self.blobs_dir().display()))?;

        let mut report = WriteReport::default();
        let mut blobs: Vec<Vec<Vec<u8>>> = Vec::new();
        let mut current: Vec<Vec<u8>> = Vec::new();
        let mut current_size = BLOB_HEADER_SIZE;

        for record in records {
            let record_size = RECORD_HEADER_SIZE + record.len() as u64;
            if BLOB_HEADER_SIZE + record_size >= MAX_FILE_SIZE {
                report.oversized_records += 1;
            }
            if !current.is_empty() && current_size + record_size >= MAX_FILE_SIZE {
                blobs.push(std::mem::take(&mut current));
                current_size = BLOB_HEADER_SIZE;
            }
            current_size += record_size;
            current.push(record.clone());
        }
        if !current.is_empty() || blobs.is_empty() {
            blobs.push(current);
        }

        let mut metas = Vec::with_capacity(blobs.len());
        for (number, blob_records) in blobs.iter().enumerate() {
            let bytes = encode_blob(blob_records);
            let path = self.blob_path(number);
            std::fs::write(&path, &bytes)
                .with_context(|| format!("failed to write {}", path.display()))?;
            metas.push(BlobMeta {
                size: bytes.len() as u64,
                md5: md5_of(&bytes),
            });
        }

        let mut stale = blobs.len();
        while self.blob_path(stale).exists() {
            std::fs::remove_file(self.blob_path(stale))?;
            stale += 1;
        }

        let index_bytes = encode_index(&metas);
        if index_bytes.len() as u64 >= MAX_FILE_SIZE {
            bail!("the INDEX file would reach the 10 MiB limit");
        }
        std::fs::write(self.index_path(), &index_bytes)
            .with_context(|| format!("failed to write {}", self.index_path().display()))?;

        report.blob_count = blobs.len();
        report.record_count = records.len();
        report.total_bytes = metas.iter().map(|meta| meta.size).sum::<u64>();
        Ok(report)
    }
}

#[derive(Debug, Default)]
pub struct VerifyReport {
    pub blob_count: usize,
    pub record_count: usize,
    pub problems: Vec<String>,
}

impl VerifyReport {
    pub fn is_healthy(&self) -> bool {
        self.problems.is_empty()
    }
}

#[derive(Debug, Default)]
pub struct WriteReport {
    pub blob_count: usize,
    pub record_count: usize,
    pub total_bytes: u64,
    /// Records that do not fit the size limit even alone in a blob.
    pub oversized_records: usize,
}

/// Look up the uuid that `~/.cellular/INDEX.json` associates with a project
/// path, creating and recording one only when asked to.
fn profile_uuid_for(project_root: &Path, create: bool) -> Result<Option<String>> {
    let key = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf())
        .to_string_lossy()
        .to_string();

    let map_path = profile_dir()?.join(PROFILE_MAP_FILE);
    let mut map: BTreeMap<String, String> = if map_path.exists() {
        let text = std::fs::read_to_string(&map_path)
            .with_context(|| format!("failed to read {}", map_path.display()))?;
        serde_json::from_str(&text)
            .with_context(|| format!("failed to parse {}", map_path.display()))?
    } else {
        BTreeMap::new()
    };

    if let Some(existing) = map.get(&key) {
        return Ok(Some(existing.clone()));
    }
    if !create {
        return Ok(None);
    }

    let uuid = uuid::Uuid::new_v4().to_string();
    map.insert(key, uuid.clone());
    if let Some(parent) = map_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&map_path, serde_json::to_string_pretty(&map)? + "\n")
        .with_context(|| format!("failed to write {}", map_path.display()))?;
    Ok(Some(uuid))
}
