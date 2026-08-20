//! The Cellular binary index format.
//!
//! Two kinds of file live under the project index directory:
//!
//! ```text
//! INDEX                 integrity metadata only
//! BLOBS/BLOB_0          self-describing container of snapshot records
//! BLOBS/BLOB_1          ...
//! ```
//!
//! `INDEX` deliberately holds nothing but the blob count and, per blob, its
//! size and MD5 digest, so a damaged index can be detected without parsing any
//! payload. Each blob is self-describing: reading the blobs alone reconstructs
//! the full catalog.
//!
//! ```text
//! INDEX
//!   magic       8 bytes  "CELLIDX\0"
//!   version     u16      = 1
//!   flags       u16      reserved, = 0
//!   blob_count  u32
//!   entries     blob_count * { size: u64, md5: [u8; 16] }
//!
//! BLOB_#
//!   magic       8 bytes  "CELLBLB\0"
//!   version     u16      = 1
//!   flags       u16      reserved, = 0
//!   records     u32
//!   record*     { payload_len: u32, payload: [u8; payload_len] }
//!
//! record payload
//!   oid_len     u8, oid bytes
//!   commit_time i64      seconds since the Unix epoch
//!   tz_offset   i32      seconds east of UTC
//!   spec        str
//!   summary     str
//!   author      str
//!   parents     u8 count, then that many { u8 length, oid bytes }
//!   refs        u16 count, then that many str (branches and tags here)
//!   index_depth u32
//!   metrics     u8       bit 0 lines, bit 1 chars, bit 2 languages
//!   fingerprint 16 bytes digest of the scan settings used
//!   languages   u32 count, then that many str (per-record language table)
//!   modules     u32 count, then that many module records
//!
//! module record
//!   path        str
//!   files       u64
//!   lines       u64
//!   chars       u64
//!   lang_stats  u32 count, then that many { lang_id: u32, files: u64,
//!               lines: u64, chars: u64 }
//!
//! str           u16 byte length, then that many UTF-8 bytes
//! ```
//!
//! All integers are little endian.

use anyhow::{Result, anyhow, bail};
use std::collections::BTreeMap;

use crate::model::{Counts, ModuleStats, Snapshot};

pub const INDEX_MAGIC: &[u8; 8] = b"CELLIDX\0";
pub const BLOB_MAGIC: &[u8; 8] = b"CELLBLB\0";
pub const FORMAT_VERSION: u16 = 2;

/// No index file may reach 10 MiB.
pub const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;

/// Commit summaries are capped, in bytes, to keep records small. Truncation
/// happens here and nowhere else: applying the same number as a character
/// count elsewhere would cut a Korean summary to a third of its length.
pub const MAX_SUMMARY_BYTES: usize = 512;

// ---------------------------------------------------------------- writing --

#[derive(Default)]
struct Writer {
    buffer: Vec<u8>,
}

impl Writer {
    fn u8(&mut self, value: u8) {
        self.buffer.push(value);
    }
    fn u16(&mut self, value: u16) {
        self.buffer.extend_from_slice(&value.to_le_bytes());
    }
    fn u32(&mut self, value: u32) {
        self.buffer.extend_from_slice(&value.to_le_bytes());
    }
    fn i32(&mut self, value: i32) {
        self.buffer.extend_from_slice(&value.to_le_bytes());
    }
    fn u64(&mut self, value: u64) {
        self.buffer.extend_from_slice(&value.to_le_bytes());
    }
    fn i64(&mut self, value: i64) {
        self.buffer.extend_from_slice(&value.to_le_bytes());
    }
    fn bytes(&mut self, value: &[u8]) {
        self.buffer.extend_from_slice(value);
    }
    fn string(&mut self, value: &str) {
        let bytes = truncate_utf8(value, u16::MAX as usize).as_bytes().to_vec();
        self.u16(bytes.len() as u16);
        self.bytes(&bytes);
    }
}

/// Truncate on a character boundary so the encoded string stays valid UTF-8.
fn truncate_utf8(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = max_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

/// Encode one snapshot record payload (without its length prefix).
pub fn encode_snapshot(snapshot: &Snapshot) -> Result<Vec<u8>> {
    if snapshot.oid.len() > u8::MAX as usize {
        bail!("commit id is too long to encode");
    }

    // Language names are interned per record so module entries stay compact.
    let mut language_ids: BTreeMap<&str, u32> = BTreeMap::new();
    let mut language_table: Vec<&str> = Vec::new();
    for module in &snapshot.modules {
        for name in module.languages.keys() {
            if !language_ids.contains_key(name.as_str()) {
                language_ids.insert(name.as_str(), language_table.len() as u32);
                language_table.push(name.as_str());
            }
        }
    }

    let mut writer = Writer::default();
    writer.u8(snapshot.oid.len() as u8);
    writer.bytes(&snapshot.oid);
    writer.i64(snapshot.commit_time);
    writer.i32(snapshot.commit_tz_offset);
    writer.string(&snapshot.spec);
    writer.string(truncate_utf8(&snapshot.summary, MAX_SUMMARY_BYTES));
    writer.string(truncate_utf8(&snapshot.author, MAX_SUMMARY_BYTES));

    if snapshot.parents.len() > u8::MAX as usize {
        bail!("a commit with more than 255 parents cannot be encoded");
    }
    writer.u8(snapshot.parents.len() as u8);
    for parent in &snapshot.parents {
        if parent.len() > u8::MAX as usize {
            bail!("a parent commit id is too long to encode");
        }
        writer.u8(parent.len() as u8);
        writer.bytes(parent);
    }

    writer.u16(snapshot.refs.len().min(u16::MAX as usize) as u16);
    for name in snapshot.refs.iter().take(u16::MAX as usize) {
        writer.string(name);
    }
    writer.u32(snapshot.index_depth);
    writer.u8(snapshot.metrics);
    writer.bytes(&snapshot.scan_fingerprint);

    writer.u32(language_table.len() as u32);
    for name in &language_table {
        writer.string(name);
    }

    writer.u32(snapshot.modules.len() as u32);
    for module in &snapshot.modules {
        writer.string(&module.path);
        writer.u64(module.totals.files);
        writer.u64(module.totals.lines);
        writer.u64(module.totals.chars);
        writer.u32(module.languages.len() as u32);
        for (name, counts) in &module.languages {
            writer.u32(language_ids[name.as_str()]);
            writer.u64(counts.files);
            writer.u64(counts.lines);
            writer.u64(counts.chars);
        }
    }

    Ok(writer.buffer)
}

/// Wrap already-encoded record payloads into one blob container.
pub fn encode_blob(records: &[Vec<u8>]) -> Vec<u8> {
    let mut writer = Writer::default();
    writer.bytes(BLOB_MAGIC);
    writer.u16(FORMAT_VERSION);
    writer.u16(0);
    writer.u32(records.len() as u32);
    for record in records {
        writer.u32(record.len() as u32);
        writer.bytes(record);
    }
    writer.buffer
}

/// Size of an empty blob container, used when packing records into blobs.
pub const BLOB_HEADER_SIZE: u64 = 16;
/// Per-record overhead inside a blob container.
pub const RECORD_HEADER_SIZE: u64 = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobMeta {
    pub size: u64,
    pub md5: [u8; 16],
}

pub fn encode_index(blobs: &[BlobMeta]) -> Vec<u8> {
    let mut writer = Writer::default();
    writer.bytes(INDEX_MAGIC);
    writer.u16(FORMAT_VERSION);
    writer.u16(0);
    writer.u32(blobs.len() as u32);
    for blob in blobs {
        writer.u64(blob.size);
        writer.bytes(&blob.md5);
    }
    writer.buffer
}

// ---------------------------------------------------------------- reading --

struct Reader<'a> {
    data: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Reader { data, position: 0 }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let end = self
            .position
            .checked_add(count)
            .ok_or_else(|| anyhow!("index record length overflow"))?;
        if end > self.data.len() {
            bail!("index record is truncated");
        }
        let slice = &self.data[self.position..end];
        self.position = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn string(&mut self) -> Result<String> {
        let length = self.u16()? as usize;
        let bytes = self.take(length)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| anyhow!("index record holds invalid UTF-8"))
    }
    fn is_done(&self) -> bool {
        self.position >= self.data.len()
    }
    fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.position)
    }
}

pub fn decode_snapshot(payload: &[u8]) -> Result<Snapshot> {
    let mut reader = Reader::new(payload);
    let oid_len = reader.u8()? as usize;
    let oid = reader.take(oid_len)?.to_vec();
    let commit_time = reader.i64()?;
    let commit_tz_offset = reader.i32()?;
    let spec = reader.string()?;
    let summary = reader.string()?;
    let author = reader.string()?;

    let parent_count = reader.u8()? as usize;
    let mut parents = Vec::with_capacity(parent_count);
    for _ in 0..parent_count {
        let length = reader.u8()? as usize;
        parents.push(reader.take(length)?.to_vec());
    }

    let ref_count = reader.u16()? as usize;
    let mut refs: Vec<String> = with_capacity_for(ref_count, reader.remaining());
    for _ in 0..ref_count {
        refs.push(reader.string()?);
    }

    let index_depth = reader.u32()?;
    let metrics = reader.u8()?;
    let mut scan_fingerprint = [0u8; 16];
    scan_fingerprint.copy_from_slice(reader.take(16)?);

    let language_count = reader.u32()? as usize;
    let mut language_table: Vec<String> = with_capacity_for(language_count, reader.remaining());
    for _ in 0..language_count {
        language_table.push(reader.string()?);
    }

    let module_count = reader.u32()? as usize;
    let mut modules: Vec<ModuleStats> = with_capacity_for(module_count, reader.remaining());
    for _ in 0..module_count {
        let path = reader.string()?;
        let totals = Counts {
            files: reader.u64()?,
            lines: reader.u64()?,
            chars: reader.u64()?,
        };
        let entry_count = reader.u32()? as usize;
        let mut languages = BTreeMap::new();
        for _ in 0..entry_count {
            let language_id = reader.u32()? as usize;
            let name = language_table
                .get(language_id)
                .ok_or_else(|| anyhow!("index record references an unknown language id"))?
                .clone();
            languages.insert(
                name,
                Counts {
                    files: reader.u64()?,
                    lines: reader.u64()?,
                    chars: reader.u64()?,
                },
            );
        }
        modules.push(ModuleStats {
            path,
            totals,
            languages,
        });
    }

    Ok(Snapshot {
        oid,
        commit_time,
        commit_tz_offset,
        spec,
        summary,
        author,
        parents,
        refs,
        index_depth,
        metrics,
        scan_fingerprint,
        modules,
    })
}

/// Split a blob container back into raw record payloads.
pub fn decode_blob(data: &[u8]) -> Result<Vec<Vec<u8>>> {
    let mut reader = Reader::new(data);
    if reader.take(8)? != BLOB_MAGIC {
        bail!("not a Cellular blob file");
    }
    let version = reader.u16()?;
    if version != FORMAT_VERSION {
        bail!("unsupported blob format version {version}");
    }
    let _flags = reader.u16()?;
    let record_count = reader.u32()? as usize;
    let mut records: Vec<Vec<u8>> = with_capacity_for(record_count, reader.remaining());
    for _ in 0..record_count {
        let length = reader.u32()? as usize;
        records.push(reader.take(length)?.to_vec());
    }
    Ok(records)
}

pub fn decode_index(data: &[u8]) -> Result<Vec<BlobMeta>> {
    let mut reader = Reader::new(data);
    if reader.take(8)? != INDEX_MAGIC {
        bail!("not a Cellular INDEX file");
    }
    let version = reader.u16()?;
    if version != FORMAT_VERSION {
        bail!("unsupported index format version {version}");
    }
    let _flags = reader.u16()?;
    let blob_count = reader.u32()? as usize;
    let mut blobs: Vec<BlobMeta> = with_capacity_for(blob_count, reader.remaining());
    for _ in 0..blob_count {
        let size = reader.u64()?;
        let mut md5 = [0u8; 16];
        md5.copy_from_slice(reader.take(16)?);
        blobs.push(BlobMeta { size, md5 });
    }
    if !reader.is_done() {
        bail!("INDEX file has trailing bytes");
    }
    Ok(blobs)
}

/// Pre-allocate for a count read out of a file without trusting it: a damaged
/// header must not turn into a huge allocation before the data is validated.
fn with_capacity_for<T>(count: usize, remaining_bytes: usize) -> Vec<T> {
    let per_item = std::mem::size_of::<T>().max(1);
    Vec::with_capacity(count.min(remaining_bytes / per_item + 1))
}

pub fn md5_of(data: &[u8]) -> [u8; 16] {
    use md5::{Digest, Md5};
    let mut hasher = Md5::new();
    hasher.update(data);
    hasher.finalize().into()
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{METRIC_CHARS, METRIC_LANGUAGES, METRIC_LINES};

    fn sample() -> Snapshot {
        let mut languages = BTreeMap::new();
        languages.insert(
            "Rust".to_string(),
            Counts {
                files: 2,
                lines: 30,
                chars: 400,
            },
        );
        Snapshot {
            oid: vec![0xab; 20],
            commit_time: 1_700_000_000,
            commit_tz_offset: 9 * 3600,
            spec: "HEAD~1".to_string(),
            summary: "안녕 commit".to_string(),
            author: "Jonghyeon Park".to_string(),
            parents: vec![vec![0xcd; 20]],
            refs: vec!["main".to_string(), "v1.0".to_string()],
            index_depth: 2,
            metrics: METRIC_LINES | METRIC_CHARS | METRIC_LANGUAGES,
            scan_fingerprint: [0x5a; 16],
            modules: vec![ModuleStats {
                path: "src/index".to_string(),
                totals: Counts {
                    files: 2,
                    lines: 30,
                    chars: 400,
                },
                languages,
            }],
        }
    }

    #[test]
    fn snapshot_round_trips() {
        let original = sample();
        let encoded = encode_snapshot(&original).unwrap();
        assert_eq!(decode_snapshot(&encoded).unwrap(), original);
    }

    #[test]
    fn blob_round_trips() {
        let records = vec![
            encode_snapshot(&sample()).unwrap(),
            encode_snapshot(&sample()).unwrap(),
        ];
        let blob = encode_blob(&records);
        assert_eq!(decode_blob(&blob).unwrap(), records);
        assert_eq!(BLOB_HEADER_SIZE, 16);
    }

    #[test]
    fn index_round_trips() {
        let metas = vec![
            BlobMeta {
                size: 42,
                md5: [1; 16],
            },
            BlobMeta {
                size: 7,
                md5: [2; 16],
            },
        ];
        let encoded = encode_index(&metas);
        assert_eq!(decode_index(&encoded).unwrap(), metas);
    }

    #[test]
    fn a_damaged_header_does_not_cause_a_huge_allocation() {
        // Claims four billion blob entries in a file that holds none.
        let mut data = INDEX_MAGIC.to_vec();
        data.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(&0xF000_0000u32.to_le_bytes());
        assert!(decode_index(&data).is_err());
    }

    #[test]
    fn rejects_foreign_files() {
        assert!(decode_index(b"not an index at all").is_err());
        assert!(decode_blob(b"not a blob at all").is_err());
    }

    #[test]
    fn summaries_are_capped_in_bytes_not_characters() {
        // A Korean subject line of ordinary length must survive intact.
        let subject = "한글로 적은 커밋 제목".repeat(4);
        assert!(subject.len() <= MAX_SUMMARY_BYTES);
        let mut snapshot = sample();
        snapshot.summary = subject.clone();
        let decoded = decode_snapshot(&encode_snapshot(&snapshot).unwrap()).unwrap();
        assert_eq!(decoded.summary, subject);

        // A pathological one is cut on a character boundary, not mid sequence.
        snapshot.summary = "한".repeat(MAX_SUMMARY_BYTES);
        let decoded = decode_snapshot(&encode_snapshot(&snapshot).unwrap()).unwrap();
        assert!(decoded.summary.len() <= MAX_SUMMARY_BYTES);
        assert!(decoded.summary.chars().all(|c| c == '한'));
    }

    #[test]
    fn truncates_on_character_boundaries() {
        let text = "가나다라";
        assert_eq!(truncate_utf8(text, 7), "가나");
    }
}
