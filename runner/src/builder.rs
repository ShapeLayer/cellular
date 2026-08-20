//! Building an index: resolve commits, measure each one, merge with what is
//! already stored and write the result out.

use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use crate::commits::{self, ResolvedCommit};
use crate::config::{Config, Threads};
use crate::disk::{self, DiskGovernor, Verdict};
use crate::index::format::{decode_snapshot, encode_snapshot};
use crate::index::store::{IndexStore, WriteReport};
use crate::model::Snapshot;
use crate::scan::{self, ScanOptions};

/// What one commit contributed to a build.
#[derive(Debug, Clone)]
pub struct BuiltSnapshot {
    pub spec: String,
    pub snapshot: Snapshot,
    /// True when the stored record was reused instead of measured again.
    pub reused: bool,
}

#[derive(Debug)]
pub struct BuildOutcome {
    pub built: Vec<BuiltSnapshot>,
    pub write: WriteReport,
    pub kept_records: usize,
    /// Set when a damaged index had to be discarded before writing.
    pub discarded_damaged_index: bool,
    /// The build stopped before covering every commit because it was
    /// cancelled. Whatever it finished is already stored.
    pub cancelled: bool,
    /// The build stopped itself because the volume holding the index was
    /// running out of room. Implies `cancelled`.
    pub low_disk_stop: bool,
    /// Set when the governor lowered the worker count part way through, to the
    /// count the build ended on.
    pub throttled_workers: Option<usize>,
    /// How many times the index was written out along the way.
    pub flushes: usize,
}

/// How often a long build writes what it has so far. Indexing one commit of a
/// large repository takes tens of seconds, so an interrupted build must not
/// throw that work away.
const FLUSH_INTERVAL: Duration = Duration::from_secs(2);

/// How often a commit that is still being measured reports what it has covered.
/// Walking one commit of a large repository takes minutes, and it produced no
/// output at all until it finished, which is indistinguishable from a hang.
const SCAN_REPORT_INTERVAL: Duration = Duration::from_secs(2);

/// Above this many commits a build is worth warning about before it starts.
pub const LARGE_BUILD: usize = 50;

/// Most scanning threads `threads: auto` will use.
///
/// Measuring a commit is CPU bound on decompressing blobs, so it scales with
/// cores, but every worker keeps its own object cache and blob buffer. The cap
/// keeps a build on a large repository from taking the whole machine, in cores
/// or in memory. A thread count the user set is not capped by it: naming a
/// number is how they say they want that number.
const MAX_AUTO_WORKERS: usize = 8;

/// Threads a build will start, however many were asked for. Each one opens its
/// own view of the object database and keeps its own caches, so a count in the
/// thousands runs the machine out of threads or memory part way through a
/// build rather than measuring anything faster.
const MAX_WORKERS: usize = 64;

/// Threads to start measuring `commits` with.
///
/// Under `auto` this is only the starting point — [`DiskGovernor`] may lower
/// it while the build runs.
fn worker_count(commits: usize, threads: Threads) -> usize {
    let wanted = match threads {
        Threads::Auto => std::thread::available_parallelism()
            .map(|count| count.get())
            .unwrap_or(1)
            .min(MAX_AUTO_WORKERS),
        Threads::Fixed(count) => count.min(MAX_WORKERS),
    };
    // More workers than commits would leave threads with nothing to pull.
    wanted.min(commits).max(1)
}

/// A message sink so the CLI and the TUI can render progress differently.
pub trait Progress {
    fn note(&mut self, message: &str);

    /// How far a build has got, in commits. Reported for every commit that is
    /// settled — reused or measured — so an interface can show the whole job's
    /// progress without reading the notes back out of the log.
    fn advance(&mut self, _done: usize, _total: usize) {}

    /// Checked between commits so a long build can be abandoned.
    fn is_cancelled(&self) -> bool {
        false
    }
}

/// Progress sink that drops everything. Used by the terminal interface.
#[allow(dead_code)]
pub struct SilentProgress;

impl Progress for SilentProgress {
    fn note(&mut self, _message: &str) {}
}

/// Work out which commits a build would cover, without doing any of the work.
/// Callers use this to report, or ask about, the size of the job first.
pub fn resolve(
    repo: &gix::Repository,
    specs: &[String],
    config: &Config,
) -> Result<Vec<ResolvedCommit>> {
    commits::resolve_all(repo, specs, config)
}

pub fn build(
    repo: &gix::Repository,
    store: &IndexStore,
    config: &Config,
    specs: &[String],
    force: bool,
    cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    progress: &mut dyn Progress,
) -> Result<BuildOutcome> {
    let resolved = resolve(repo, specs, config)?;
    progress.note(&format!(
        "resolved {} spec(s) to {} commit(s)",
        specs.len(),
        resolved.len()
    ));
    build_resolved(repo, store, config, &resolved, force, cancel, progress)
}

/// Build from an already resolved commit list.
pub fn build_resolved(
    repo: &gix::Repository,
    store: &IndexStore,
    config: &Config,
    resolved: &[ResolvedCommit],
    force: bool,
    cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    progress: &mut dyn Progress,
) -> Result<BuildOutcome> {
    let mut options = ScanOptions::from_config(config)?;
    options.cancel = cancel;
    let refs = commits::ref_names(repo).unwrap_or_default();
    let total = resolved.len();
    progress.advance(0, total);
    if total >= LARGE_BUILD {
        progress.note(&format!(
            "{total} commits to cover; the first pass reads every file in each one"
        ));
    }

    // Existing records, keyed by commit id so a rebuild replaces in place.
    let mut discarded_damaged_index = false;
    let mut stored: HashMap<Vec<u8>, (Snapshot, Vec<u8>)> = HashMap::new();
    if store.exists() {
        let report = store.verify()?;
        if report.is_healthy() {
            for record in store.load_records()? {
                match decode_snapshot(&record) {
                    Ok(snapshot) => {
                        stored.insert(snapshot.oid.clone(), (snapshot, record));
                    }
                    Err(error) => {
                        progress.note(&format!("skipping an unreadable record: {error}"));
                        discarded_damaged_index = true;
                    }
                }
            }
        } else {
            discarded_damaged_index = true;
            progress.note("the stored index is damaged; rebuilding it from scratch");
        }
    }

    // Cancellation reaches the workers through the scan options, so the two
    // sources of it — the caller's flag and the progress sink — are merged into
    // one flag here rather than checked separately in two places.
    let stop = match &options.cancel {
        Some(flag) => Arc::clone(flag),
        None => {
            let flag = Arc::new(AtomicBool::new(false));
            options.cancel = Some(Arc::clone(&flag));
            flag
        }
    };

    // Results keep their place in the resolved order, so a build reports the
    // commits in the order they were asked for however they finish.
    let mut built: Vec<Option<BuiltSnapshot>> = vec![None; total];
    let mut cancelled = false;
    let mut low_disk_stop = false;
    let mut throttled_workers = None;
    let mut flushes = 0;
    let mut last_flush = Instant::now();

    // Reusing a stored record is a map lookup, so it happens up front and in
    // order; only the commits that have to be measured reach the workers.
    let mut to_measure: Vec<(usize, &ResolvedCommit)> = Vec::new();
    // Counted separately from the measured ones so the two loops can report a
    // single running total over the whole resolved list.
    let mut reused = 0usize;
    for (position, commit) in resolved.iter().enumerate() {
        if progress.is_cancelled() || options.is_cancelled() {
            cancelled = true;
            break;
        }
        let key = commit.id.as_bytes().to_vec();
        let reusable = !force
            && stored
                .get(&key)
                .is_some_and(|(snapshot, _)| snapshot.scan_fingerprint == options.fingerprint);
        if !reusable {
            to_measure.push((position, commit));
            continue;
        }
        let (snapshot, record) = stored.get_mut(&key).expect("checked just above");
        // The stored record may carry the spec of an earlier build.
        if snapshot.spec != commit.spec {
            snapshot.spec = commit.spec.clone();
            *record = encode_snapshot(snapshot)?;
        }
        let snapshot = snapshot.clone();
        progress.note(&format!(
            "reusing {} ({})",
            snapshot.short_oid(),
            commit.spec
        ));
        built[position] = Some(BuiltSnapshot {
            spec: commit.spec.clone(),
            snapshot,
            reused: true,
        });
        reused += 1;
        progress.advance(reused, total);
    }

    if !cancelled && !to_measure.is_empty() {
        let workers = worker_count(to_measure.len(), config.threads);
        if let Threads::Fixed(asked) = config.threads
            && asked > MAX_WORKERS
        {
            progress.note(&format!(
                "threads is set to {asked}; running {MAX_WORKERS}, which is as many \
                 as a build will start"
            ));
        }
        if workers > 1 {
            progress.note(&format!(
                "measuring {} commit(s) on {workers} thread(s){}",
                to_measure.len(),
                // Worth saying only when the number was not asked for by name,
                // and so may not be the number the build ends on.
                if config.threads.is_auto() {
                    " (auto)"
                } else {
                    ""
                }
            ));
        }

        // Watches what each commit adds to the index against what is left on
        // the volume. Under `auto` it may take threads away; under a fixed
        // count it only steps in to stop a build that can no longer be
        // written out safely.
        let mut governor = DiskGovernor::new(&store.root, workers, config.threads.is_auto());
        if governor.is_blind() && config.threads.is_auto() {
            progress.note("free space cannot be read here, so the thread count will not adapt");
        }
        // The cap the workers watch. Lowered by the governor, never raised.
        let allowed = AtomicUsize::new(workers);

        // A worker cannot share the caller's repository handle, so each opens
        // its own view of the same object database.
        let shared = gix::ThreadSafeRepository::open(repo.git_dir())?;
        let next = AtomicUsize::new(0);
        let (sender, receiver) = mpsc::channel::<ScanMessage>();
        let queue = to_measure.as_slice();
        let options = &options;
        let refs = &refs;

        let mut failure: Option<anyhow::Error> = None;
        let mut done = 0usize;

        std::thread::scope(|scope| -> Result<()> {
            for id in 0..workers {
                let sender = sender.clone();
                let shared = &shared;
                let next = &next;
                let allowed = &allowed;
                scope.spawn(move || {
                    let repo = shared.to_thread_local();
                    loop {
                        // Checked before claiming a slot: a worker that stands
                        // down has to leave its commit for the others rather
                        // than take it along.
                        if id >= allowed.load(Ordering::Relaxed) {
                            break;
                        }
                        let slot = next.fetch_add(1, Ordering::Relaxed);
                        let Some((position, commit)) = queue.get(slot) else {
                            break;
                        };
                        if options.is_cancelled() {
                            break;
                        }
                        let short = commit.id.to_hex_with_len(10).to_string();
                        // Throttled here rather than in the coordinator, so a
                        // slow commit costs one message every interval instead
                        // of one per batch of files.
                        let mut last_report = Instant::now();
                        let scanned = scan::scan_commit(
                            &repo,
                            commit.id,
                            &commit.spec,
                            options,
                            refs,
                            &mut |tick| {
                                if last_report.elapsed() < SCAN_REPORT_INTERVAL {
                                    return;
                                }
                                last_report = Instant::now();
                                let _ = sender.send(ScanMessage::Scanning {
                                    text: format!(
                                        "scanning {short} ({}): {} files, {} read so far",
                                        commit.spec,
                                        tick.files,
                                        disk::human_bytes(tick.bytes)
                                    ),
                                });
                            },
                        );
                        if sender
                            .send(ScanMessage::Measured {
                                position: *position,
                                spec: commit.spec.clone(),
                                scanned: scanned.map(Box::new),
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                });
            }
            // The coordinator holds no sender of its own, so the channel closes
            // as soon as the last worker stops.
            drop(sender);

            for message in &receiver {
                match message {
                    ScanMessage::Scanning { text } => progress.note(&text),
                    ScanMessage::Measured {
                        position,
                        spec,
                        scanned,
                    } => {
                        done += 1;
                        progress.advance(reused + done, total);
                        let snapshot = match scanned {
                            Ok(snapshot) => *snapshot,
                            // A cancelled scan is not a failure; keep what is
                            // already stored.
                            Err(_) if options.is_cancelled() => {
                                cancelled = true;
                                continue;
                            }
                            Err(error) => {
                                // Let the other workers stop before returning.
                                stop.store(true, Ordering::Relaxed);
                                failure.get_or_insert(error);
                                continue;
                            }
                        };
                        let totals = snapshot.totals();
                        progress.note(&format!(
                            "[{done}/{}] indexed {} ({spec}): {} modules, {} files, \
                             {} lines, {} chars",
                            queue.len(),
                            snapshot.short_oid(),
                            snapshot.modules.len(),
                            totals.files,
                            totals.lines,
                            totals.chars
                        ));
                        let record = encode_snapshot(&snapshot)?;
                        governor.record(record.len() as u64);
                        stored.insert(snapshot.oid.clone(), (snapshot.clone(), record));
                        built[position] = Some(BuiltSnapshot {
                            spec,
                            snapshot,
                            reused: false,
                        });
                    }
                }

                if progress.is_cancelled() {
                    stop.store(true, Ordering::Relaxed);
                }
                // Persist regularly so an interrupted build keeps the commits
                // it has already measured instead of starting over.
                if last_flush.elapsed() >= FLUSH_INTERVAL {
                    flush(store, &stored)?;
                    flushes += 1;
                    last_flush = Instant::now();

                    // Straight after a flush, what is on the volume is what
                    // the governor is reasoning about.
                    let remaining = queue.len().saturating_sub(done);
                    match governor.reassess(remaining) {
                        Some(Verdict::Workers(count)) => {
                            allowed.store(count, Ordering::Relaxed);
                            throttled_workers = Some(count);
                            progress.note(&format!(
                                "{} free where the index lives, about {} still to \
                                 write: measuring on {count} thread(s) from here",
                                governor
                                    .last_seen()
                                    .map(|space| disk::human_bytes(space.available))
                                    .unwrap_or_else(|| "an unknown amount".to_string()),
                                disk::human_bytes(
                                    governor.per_commit().saturating_mul(remaining as u64)
                                ),
                            ));
                        }
                        Some(Verdict::Stop { available, round }) => {
                            low_disk_stop = true;
                            stop.store(true, Ordering::Relaxed);
                            progress.note(&format!(
                                "stopping here with everything measured so far stored: \
                                 {} free where the index lives, and writing out the \
                                 commits in flight needs {} on top of the {} this \
                                 build keeps in reserve",
                                disk::human_bytes(available),
                                disk::human_bytes(round),
                                disk::human_bytes(
                                    governor
                                        .last_seen()
                                        .map(|space| space.reserve())
                                        .unwrap_or(0)
                                ),
                            ));
                        }
                        None => {}
                    }
                }
            }
            Ok(())
        })?;

        if let Some(error) = failure {
            return Err(error);
        }
        if options.is_cancelled() {
            cancelled = true;
        }
    }

    let built: Vec<BuiltSnapshot> = built.into_iter().flatten().collect();

    // A partial build has not seen every spec, so leaving the existing labels
    // alone is the only safe choice.
    let mut relabelled = 0;
    if !cancelled {
        relabelled = relabel(&mut stored, resolved)?;
    }
    if relabelled > 0 {
        progress.note(&format!(
            "{relabelled} older record(s) no longer carry a spec that moved"
        ));
    }

    let write = flush(store, &stored)?;
    let kept_records = stored.len().saturating_sub(built.len());
    Ok(BuildOutcome {
        built,
        write,
        kept_records,
        discarded_damaged_index,
        cancelled,
        low_disk_stop,
        throttled_workers,
        flushes: flushes + 1,
    })
}

/// What a scanning worker reports back to the coordinator.
enum ScanMessage {
    /// A commit still being measured, already throttled by the worker.
    Scanning { text: String },
    /// A commit the worker finished with, or failed on.
    Measured {
        /// Where the commit sits in the resolved order.
        position: usize,
        spec: String,
        /// Boxed: a snapshot dwarfs every other field, and the whole enum is
        /// sized by its largest variant.
        scanned: Result<Box<Snapshot>>,
    },
}

/// Write every stored record out, oldest commit first so the viewer can read
/// them in order.
fn flush(
    store: &IndexStore,
    stored: &HashMap<Vec<u8>, (Snapshot, Vec<u8>)>,
) -> Result<WriteReport> {
    let mut all: Vec<&(Snapshot, Vec<u8>)> = stored.values().collect();
    all.sort_by(|left, right| {
        (left.0.commit_time, &left.0.oid).cmp(&(right.0.commit_time, &right.0.oid))
    });
    let records: Vec<Vec<u8>> = all.into_iter().map(|(_, record)| record.clone()).collect();
    store.write_records(&records)
}

/// A spec such as HEAD, HEAD~2 or a branch name points somewhere else as
/// Whatever a spec used in this build no longer resolves to loses that label,
/// so the index never holds two snapshots both claiming to be HEAD.
fn relabel(
    stored: &mut HashMap<Vec<u8>, (Snapshot, Vec<u8>)>,
    resolved: &[ResolvedCommit],
) -> Result<usize> {
    let mut claimed: HashMap<&str, HashSet<&[u8]>> = HashMap::new();
    for commit in resolved {
        claimed
            .entry(commit.spec.as_str())
            .or_default()
            .insert(commit.id.as_bytes());
    }
    let mut relabelled = 0;
    for (oid, (snapshot, record)) in stored.iter_mut() {
        let stale = claimed
            .get(snapshot.spec.as_str())
            .is_some_and(|owners| !owners.contains(oid.as_slice()));
        if stale {
            snapshot.spec.clear();
            *record = encode_snapshot(snapshot)?;
            relabelled += 1;
        }
    }
    Ok(relabelled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_auto_worker_count_never_exceeds_the_work_or_the_cap() {
        // A build of one commit must not pay for a pool.
        assert_eq!(worker_count(1, Threads::Auto), 1);
        // Fewer commits than cores means idle threads, so the work decides.
        assert!(worker_count(2, Threads::Auto) <= 2);
        assert!(worker_count(1000, Threads::Auto) <= MAX_AUTO_WORKERS);
        assert!(worker_count(1000, Threads::Auto) >= 1);
    }

    #[test]
    fn a_fixed_thread_count_is_taken_as_given() {
        // Above the auto cap: an explicit number is a decision, not a hint.
        assert_eq!(worker_count(1000, Threads::Fixed(32)), 32);
        assert_eq!(worker_count(1000, Threads::Fixed(1)), 1);
        // Still no more workers than there are commits to hand out.
        assert_eq!(worker_count(3, Threads::Fixed(32)), 3);
        // A count no machine can spawn is not honoured to the point of dying
        // part way through the build.
        assert_eq!(worker_count(100_000, Threads::Fixed(100_000)), MAX_WORKERS);
    }
}
