//! The data an index snapshot holds: per-module metrics for one commit.

use std::collections::BTreeMap;

pub const METRIC_LINES: u8 = 1 << 0;
pub const METRIC_CHARS: u8 = 1 << 1;
pub const METRIC_LANGUAGES: u8 = 1 << 2;

/// The module every repository-root file is attributed to.
pub const ROOT_MODULE: &str = ".";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Counts {
    pub files: u64,
    pub lines: u64,
    pub chars: u64,
}

impl Counts {
    pub fn add_file(&mut self, lines: u64, chars: u64) {
        self.files += 1;
        self.lines += lines;
        self.chars += chars;
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModuleStats {
    /// Repository-relative module path, `.` for the repository root.
    pub path: String,
    pub totals: Counts,
    /// Language name to counts; empty when the `languages` metric is off.
    pub languages: BTreeMap<String, Counts>,
}

/// One commit's worth of measurements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// Raw commit object id bytes (20 for SHA-1, 32 for SHA-256).
    pub oid: Vec<u8>,
    /// Committer time, seconds since the Unix epoch.
    pub commit_time: i64,
    /// Committer time zone offset in seconds east of UTC.
    pub commit_tz_offset: i32,
    /// The commit spec the user typed that resolved to this commit.
    pub spec: String,
    /// First line of the commit message, truncated.
    pub summary: String,
    /// Commit author name, shown in the timeline tooltip.
    pub author: String,
    /// Raw object ids of this commit's parents, so the viewer can draw the
    /// branch structure rather than a flat list.
    pub parents: Vec<Vec<u8>>,
    /// Branch and tag names that point at this commit.
    pub refs: Vec<String>,
    pub index_depth: u32,
    /// Bit set of `METRIC_*`.
    pub metrics: u8,
    /// Digest of the scan settings that produced this record. A record may
    /// only be reused when the current settings hash to the same value.
    pub scan_fingerprint: [u8; 16],
    pub modules: Vec<ModuleStats>,
}

impl Snapshot {
    pub fn parents_hex(&self) -> Vec<String> {
        self.parents.iter().map(|oid| hex(oid)).collect()
    }

    pub fn oid_hex(&self) -> String {
        hex(&self.oid)
    }

    pub fn short_oid(&self) -> String {
        self.oid_hex().chars().take(10).collect()
    }

    pub fn totals(&self) -> Counts {
        let mut totals = Counts::default();
        for module in &self.modules {
            totals.files += module.totals.files;
            totals.lines += module.totals.lines;
            totals.chars += module.totals.chars;
        }
        totals
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
