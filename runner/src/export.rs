//! Packaging a built index as a `.cellexport` file for the viewer.
//!
//! The viewer is a static site and cannot read the project index directory
//! under `~/.cellular/index/`, so the index travels as one file:
//!
//! ```text
//! magic    8 bytes  "CELLEXP\0"
//! version  u16      = 1, little endian
//! payload  a ZIP archive holding INDEX and BLOBS/BLOB_#
//! ```
//!
//! The magic sits in front of the archive so the viewer can reject any other
//! file before trying to unpack it. ZIP readers locate the central directory
//! from the end of the file, so the prefix does not stop ordinary tools from
//! opening the archive either.

use anyhow::{Context, Result, bail};
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

use crate::index::store::{BLOBS_DIR, INDEX_FILE, IndexStore, lock_store};

pub const EXPORT_MAGIC: &[u8; 8] = b"CELLEXP\0";
pub const EXPORT_VERSION: u16 = 1;
pub const EXPORT_EXTENSION: &str = "cellexport";

#[derive(Debug)]
pub struct ExportReport {
    pub path: PathBuf,
    pub blob_count: usize,
    pub record_count: usize,
    pub bytes: u64,
}

/// The name to use when the user did not give one: `<project>.cellexport`.
pub fn default_path(project_root: &Path) -> PathBuf {
    let name = project_root
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "cellular".to_string());
    project_root.join(format!("{name}.{EXPORT_EXTENSION}"))
}

/// Package the index as it stands.
///
/// A build may be running: it rewrites the whole store every couple of
/// seconds, and each of those rewrites leaves a complete, if shorter, index
/// behind, so what this packages is a real index of the commits measured so
/// far. Holding the store lock across the verify and the reads is what keeps
/// the export off the moment of a rewrite, where `INDEX` and the blobs have
/// yet to meet.
pub fn write(store: &IndexStore, destination: &Path) -> Result<ExportReport> {
    let guard = lock_store();
    if !store.exists() {
        bail!(
            "no index has been built at {}; build one before exporting",
            store.root.display()
        );
    }
    // Exporting a damaged index would only spread the damage.
    let report = store.verify()?;
    if !report.is_healthy() {
        bail!(
            "the stored index is damaged, so it was not exported: {}",
            report.problems.join("; ")
        );
    }

    let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    let index_bytes = std::fs::read(store.index_path())
        .with_context(|| format!("failed to read {}", store.index_path().display()))?;
    archive.start_file(INDEX_FILE, options)?;
    archive.write_all(&index_bytes)?;

    for number in 0..report.blob_count {
        let path = store.blob_path(number);
        let bytes =
            std::fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
        archive.start_file(format!("{BLOBS_DIR}/BLOB_{number}"), options)?;
        archive.write_all(&bytes)?;
    }

    let archive = archive.finish()?.into_inner();
    // Everything that had to agree has been read; a waiting build can move on
    // while this writes the file out.
    drop(guard);

    let mut file = Vec::with_capacity(archive.len() + 10);
    file.extend_from_slice(EXPORT_MAGIC);
    file.extend_from_slice(&EXPORT_VERSION.to_le_bytes());
    file.extend_from_slice(&archive);

    if let Some(parent) = destination.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(destination, &file)
        .with_context(|| format!("failed to write {}", destination.display()))?;

    Ok(ExportReport {
        path: destination.to_path_buf(),
        blob_count: report.blob_count,
        record_count: report.record_count,
        bytes: file.len() as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_name_follows_the_project() {
        let path = default_path(Path::new("/tmp/my-project"));
        assert_eq!(path, Path::new("/tmp/my-project/my-project.cellexport"));
    }

    /// A build flushes the whole store every couple of seconds. Exporting
    /// against that has to come away with one of the states the store passed
    /// through, never a mixture of two.
    #[test]
    fn exporting_alongside_a_rewrite_never_catches_it_half_way() {
        let root = std::env::temp_dir().join(format!("cellular-export-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("a place for the index");
        let store = IndexStore { root: root.clone() };

        // Two indexes far enough apart to span a different number of blob
        // files, so a rewrite between them moves more than the contents.
        let few: Vec<Vec<u8>> = (0..4u8).map(|byte| vec![byte; 1024]).collect();
        let many: Vec<Vec<u8>> = (0..176u8).map(|byte| vec![byte; 64 * 1024]).collect();
        store.write_records(&few).expect("a first index");

        let writer_store = store.clone();
        let writer = std::thread::spawn(move || {
            for round in 0..8 {
                let records = if round % 2 == 0 { &many } else { &few };
                writer_store.write_records(records).expect("a rewrite");
            }
        });

        let destination = root.join("part-way.cellexport");
        for _ in 0..8 {
            let report = write(&store, &destination).expect("an export during a rewrite");
            assert!(
                report.record_count == 4 || report.record_count == 176,
                "the export mixed two states: {} record(s)",
                report.record_count
            );
        }

        writer.join().expect("the writer finishes");
        std::fs::remove_dir_all(&root).ok();
    }
}
