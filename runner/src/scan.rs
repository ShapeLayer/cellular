//! Walking a commit's tree and aggregating per-module metrics.

use anyhow::{Context, Result, bail};
use gix::ObjectId;
use gix::prelude::FindExt;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::commits::RefMap;
use crate::config::Config;
use crate::filters::{DirectoryMatcher, NameMatcher, SubtreeMatcher};
use crate::lang;
use crate::model::{
    METRIC_CHARS, METRIC_LANGUAGES, METRIC_LINES, ModuleStats, ROOT_MODULE, Snapshot,
};

/// Compiled matchers and metric flags for one index build.
#[derive(Debug, Clone)]
pub struct ScanOptions {
    pub index_depth: u32,
    pub metrics: u8,
    /// Digest of every setting that changes what a scan produces.
    pub fingerprint: [u8; 16],
    /// Set from another thread to abandon a scan in progress.
    pub cancel: Option<Arc<AtomicBool>>,
    exclude: SubtreeMatcher,
    detect_as_module: DirectoryMatcher,
    ignoring_extensions: NameMatcher,
    ignoring_files: NameMatcher,
}

impl ScanOptions {
    pub fn from_config(config: &Config) -> Result<Self> {
        let index_depth = config
            .index_depth
            .context("index_depth is not set; pass --index-depth or set it in config.json")?;

        let mut metrics = 0;
        if config.wants_metric("lines") {
            metrics |= METRIC_LINES;
        }
        if config.wants_metric("chars") {
            metrics |= METRIC_CHARS;
        }
        if config.wants_metric("languages") {
            metrics |= METRIC_LANGUAGES;
        }

        let fingerprint = fingerprint_of(config, index_depth, metrics);

        if metrics == 0 {
            bail!(
                "metric selects nothing to measure; it must match at least one \
                 of lines, chars, languages"
            );
        }

        Ok(ScanOptions {
            index_depth,
            metrics,
            fingerprint,
            cancel: None,
            exclude: SubtreeMatcher::new(&config.index_exclude)?,
            detect_as_module: DirectoryMatcher::new(&config.index_detect_as_module)?,
            ignoring_extensions: NameMatcher::new(&config.ignoring_extensions)?,
            ignoring_files: NameMatcher::new(&config.ignoring_files)?,
        })
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::Relaxed))
    }

    fn skips_directory(&self, rel_path: &str) -> bool {
        self.exclude.matches(rel_path)
    }

    fn skips_file(&self, rel_path: &str, file_name: &str) -> bool {
        if self.exclude.matches(rel_path) {
            return true;
        }
        if self.ignoring_files.matches(file_name) || self.ignoring_files.matches(rel_path) {
            return true;
        }
        match lang::dotted_extension(file_name) {
            Some(extension) => self.ignoring_extensions.matches(&extension),
            // Dot files such as `.gitignore` have no extension of their own, so
            // patterns like `.git*` are matched against the whole name.
            None => self.ignoring_extensions.matches(file_name),
        }
    }
}

/// Digest the settings that decide what a scan measures, so a stored record is
/// only reused when it was produced the same way.
fn fingerprint_of(config: &Config, index_depth: u32, metrics: u8) -> [u8; 16] {
    let canonical = format!(
        "depth={index_depth}\nmetrics={metrics}\nexclude={}\ndetect={}\nextensions={}\nfiles={}\n",
        config.index_exclude.join("\u{1f}"),
        config.index_detect_as_module.join("\u{1f}"),
        config.ignoring_extensions.join("\u{1f}"),
        config.ignoring_files.join("\u{1f}"),
    );
    crate::index::format::md5_of(canonical.as_bytes())
}

/// Live counts from a scan that is still running, handed to the caller often
/// enough that a commit taking minutes is visibly moving rather than silent.
#[derive(Debug, Clone, Copy, Default)]
pub struct ScanTick {
    pub files: u64,
    pub bytes: u64,
}

/// Files measured between two reports. Reporting on every file would call back
/// millions of times on a large commit for no extra information.
const TICK_EVERY: u64 = 256;

/// Measure one commit, reporting progress as it goes.
///
/// `tick` is called every [`TICK_EVERY`] files. A commit of a large repository
/// takes minutes to walk, and the caller has nothing to show until it returns,
/// so without this the interface cannot tell slow from stuck.
pub fn scan_commit(
    repo: &gix::Repository,
    id: ObjectId,
    spec: &str,
    options: &ScanOptions,
    refs: &RefMap,
    tick: &mut dyn FnMut(ScanTick),
) -> Result<Snapshot> {
    let commit = repo
        .find_object(id)
        .with_context(|| format!("commit {id} is missing from the object database"))?
        .into_commit();
    let time = commit.time()?;
    let summary = commit
        .message()
        .map(|message| message.summary().to_string())
        .unwrap_or_default();
    let author = commit
        .author()
        .map(|author| author.name.to_string())
        .unwrap_or_default();
    let parents: Vec<Vec<u8>> = commit
        .parent_ids()
        .map(|parent| parent.detach().as_bytes().to_vec())
        .collect();
    let tree = commit.tree()?;

    let mut walk = Walk {
        options,
        file_buffer: Vec::new(),
        modules: BTreeMap::new(),
        seen: ScanTick::default(),
        since_tick: 0,
        tick,
    };
    walk.descend(repo, tree.id, "", None)?;
    let modules = walk.modules;

    Ok(Snapshot {
        oid: id.as_bytes().to_vec(),
        commit_time: time.seconds,
        commit_tz_offset: time.offset,
        spec: spec.to_string(),
        summary,
        author,
        parents,
        refs: refs.get(&id).cloned().unwrap_or_default(),
        index_depth: options.index_depth,
        metrics: options.metrics,
        scan_fingerprint: options.fingerprint,
        modules: modules.into_values().collect(),
    })
}

/// The state carried down a tree walk: the buffer blobs are read into, the
/// module totals built up so far, and the progress counters.
struct Walk<'a> {
    options: &'a ScanOptions,
    /// One buffer for every blob, reused so a walk does not reallocate per file.
    file_buffer: Vec<u8>,
    modules: BTreeMap<String, ModuleStats>,
    seen: ScanTick,
    since_tick: u64,
    tick: &'a mut dyn FnMut(ScanTick),
}

impl Walk<'_> {
    fn descend(
        &mut self,
        repo: &gix::Repository,
        tree_id: ObjectId,
        dir_path: &str,
        forced_module: Option<&str>,
    ) -> Result<()> {
        // One buffer per directory: the tree entries borrow it while we iterate.
        let mut tree_buffer = Vec::new();
        let tree = repo.objects.find_tree(&tree_id, &mut tree_buffer)?;

        for entry in tree.entries {
            let name = entry.filename.to_string();
            let rel_path = if dir_path.is_empty() {
                name.clone()
            } else {
                format!("{dir_path}/{name}")
            };
            let mode = entry.mode;

            if mode.is_tree() {
                if self.options.skips_directory(&rel_path) {
                    continue;
                }
                // A directory matching index_detect_as_module becomes a module
                // of its own; the deepest match along a path wins.
                let child_forced = if self.options.detect_as_module.matches(&rel_path) {
                    Some(rel_path.as_str())
                } else {
                    forced_module
                };
                self.descend(repo, entry.oid.to_owned(), &rel_path, child_forced)?;
                continue;
            }

            // Symlinks and submodule pointers carry no source of their own.
            if !mode.is_blob() {
                continue;
            }
            if self.options.skips_file(&rel_path, &name) {
                continue;
            }
            if self.options.is_cancelled() {
                bail!("the index build was cancelled");
            }

            let data = repo
                .objects
                .find_blob(entry.oid, &mut self.file_buffer)?
                .data;
            let size = data.len() as u64;
            let measured = measure(data);

            // Every blob read costs the same whether or not it counts towards a
            // module, so binary files have to move the counters too. Otherwise
            // a commit that is mostly binary looks stalled.
            self.seen.files += 1;
            self.seen.bytes += size;
            self.since_tick += 1;
            if self.since_tick >= TICK_EVERY {
                self.since_tick = 0;
                (self.tick)(self.seen);
            }

            let Some((lines, chars)) = measured else {
                continue; // binary content
            };

            let module_path = match forced_module {
                Some(path) => path.to_string(),
                None => module_for(dir_path, self.options.index_depth),
            };
            let module = self
                .modules
                .entry(module_path.clone())
                .or_insert_with(|| ModuleStats {
                    path: module_path,
                    ..ModuleStats::default()
                });

            let lines = if self.options.metrics & METRIC_LINES != 0 {
                lines
            } else {
                0
            };
            let chars = if self.options.metrics & METRIC_CHARS != 0 {
                chars
            } else {
                0
            };
            module.totals.add_file(lines, chars);

            if self.options.metrics & METRIC_LANGUAGES != 0
                && let Some(language) = lang::detect(&name)
            {
                module
                    .languages
                    .entry(language)
                    .or_default()
                    .add_file(lines, chars);
            }
        }
        Ok(())
    }
}

/// The module a file in `dir_path` belongs to: the first `depth` path
/// components, or the whole directory when it is shallower than `depth`.
pub fn module_for(dir_path: &str, depth: u32) -> String {
    if dir_path.is_empty() {
        return ROOT_MODULE.to_string();
    }
    if depth == 0 {
        return ROOT_MODULE.to_string();
    }
    let components: Vec<&str> = dir_path.split('/').collect();
    if components.len() <= depth as usize {
        return dir_path.to_string();
    }
    components[..depth as usize].join("/")
}

/// Line and character counts, or `None` for binary content.
///
/// Characters are counted as UTF-8 code points; a line is a `\n`, plus a final
/// line when the content does not end with one.
fn measure(data: &[u8]) -> Option<(u64, u64)> {
    if is_binary(data) {
        return None;
    }
    if data.is_empty() {
        return Some((0, 0));
    }
    let newlines = data.iter().filter(|byte| **byte == b'\n').count() as u64;
    let lines = if data.last() == Some(&b'\n') {
        newlines
    } else {
        newlines + 1
    };
    // Every UTF-8 code point starts with a non-continuation byte.
    let chars = data.iter().filter(|byte| (**byte & 0xC0) != 0x80).count() as u64;
    Some((lines, chars))
}

fn is_binary(data: &[u8]) -> bool {
    const PROBE: usize = 8000;
    data.iter().take(PROBE).any(|byte| *byte == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fingerprint_follows_every_scan_setting() {
        let mut config = Config {
            index_depth: Some(2),
            ..Config::default()
        };
        let base = ScanOptions::from_config(&config).unwrap().fingerprint;

        config.index_detect_as_module = vec!["src/components/foo".to_string()];
        let with_module = ScanOptions::from_config(&config).unwrap().fingerprint;
        assert_ne!(base, with_module);

        config.index_detect_as_module.clear();
        assert_eq!(ScanOptions::from_config(&config).unwrap().fingerprint, base);

        config.index_depth = Some(3);
        assert_ne!(ScanOptions::from_config(&config).unwrap().fingerprint, base);
    }

    #[test]
    fn modules_truncate_to_the_index_depth() {
        assert_eq!(module_for("src/components/foo", 2), "src/components");
        assert_eq!(module_for("src", 2), "src");
        assert_eq!(module_for("", 2), ROOT_MODULE);
        assert_eq!(module_for("src/components", 0), ROOT_MODULE);
    }

    #[test]
    fn counts_lines_and_characters() {
        assert_eq!(measure(b"a\nb\n"), Some((2, 4)));
        assert_eq!(measure(b"a\nb"), Some((2, 3)));
        assert_eq!(measure(b""), Some((0, 0)));
        assert_eq!(measure("가나\n".as_bytes()), Some((1, 3)));
        assert_eq!(measure(b"\x00binary"), None);
    }
}
