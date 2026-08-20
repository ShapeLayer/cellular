//! How much room is left where the index is written, and what a build should
//! do about it.
//!
//! Threads do not decide how large an index ends up: that is set by how many
//! commits are covered and how many modules each one has. What they decide is
//! how fast it gets there and how much measured-but-not-yet-written data the
//! build is holding at once. So the governor here does two separate things:
//!
//!   * it lowers the worker count as the volume fills, which slows the growth
//!     and shrinks the batch a flush has to write, and
//!   * it stops the build while there is still room to write cleanly, because
//!     a `flush` that fails halfway leaves `INDEX` disagreeing with `BLOBS/`.
//!
//! The second one is what actually protects the index. The first buys it the
//! time to happen.

use std::path::{Path, PathBuf};

/// Most the governor will ever hold back. A machine whose disk is this close
/// to full has other things about to fail, and an index build should not be
/// the one that takes the last of it.
pub const MAX_RESERVE_BYTES: u64 = 256 * 1024 * 1024;

/// Share of a volume held back when a tenth of it is less than the cap. A
/// fixed 256 MiB is nothing on a laptop disk and a quarter of a small tmpfs,
/// where it would refuse to write an index of a few hundred kilobytes.
const RESERVE_SHARE: u64 = 10;

/// How much of the projected growth must still fit, on top of the reserve,
/// before a build is allowed every worker it asked for.
const COMFORTABLE: f64 = 2.0;

/// Rounds of in-flight work a build keeps room for. Below this it stops rather
/// than risk a half-written flush.
const FLOOR_ROUNDS: u64 = 2;

/// Weight given to the newest commit when tracking the size trend. History is
/// walked oldest first and projects grow, so the last commits measured are a
/// better guide to what is left than the average of all of them.
const TREND_WEIGHT: f64 = 0.5;

/// What one volume has left, and how big it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Space {
    pub available: u64,
    pub total: u64,
}

impl Space {
    /// Space this volume keeps back. Capped at [`MAX_RESERVE_BYTES`], but a
    /// share of the volume when that is smaller, so a small disk is not
    /// declared full while most of it is free.
    pub fn reserve(&self) -> u64 {
        MAX_RESERVE_BYTES.min(self.total / RESERVE_SHARE)
    }

    /// What a build may plan on using.
    pub fn headroom(&self) -> u64 {
        self.available.saturating_sub(self.reserve())
    }
}

/// The volume that holds `path`, or `None` when the platform gives no answer.
/// The path need not exist yet — the nearest existing parent is on the same
/// volume as the directory that will be created under it.
pub fn space(path: &Path) -> Option<Space> {
    let mut candidate: Option<&Path> = Some(path);
    while let Some(current) = candidate {
        if current.exists() {
            return space_on_existing(current);
        }
        candidate = current.parent();
    }
    None
}

#[cfg(unix)]
// The block size fields are u64 on some targets and narrower on others, so the
// casts are only redundant on the platform that happens to be compiling.
#[allow(clippy::unnecessary_cast)]
fn space_on_existing(path: &Path) -> Option<Space> {
    use std::os::unix::ffi::OsStrExt;

    let path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: statvfs reads the path and writes the struct it is handed; both
    // outlive the call, and the result is only read after it reports success.
    unsafe {
        let mut stats: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(path.as_ptr(), &mut stats) != 0 {
            return None;
        }
        // f_bavail, not f_bfree: blocks reserved for root are not ours.
        let block = if stats.f_frsize > 0 {
            stats.f_frsize as u64
        } else {
            stats.f_bsize as u64
        };
        Some(Space {
            available: (stats.f_bavail as u64).saturating_mul(block),
            total: (stats.f_blocks as u64).saturating_mul(block),
        })
    }
}

#[cfg(windows)]
fn space_on_existing(path: &Path) -> Option<Space> {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetDiskFreeSpaceExW(
            directory: *const u16,
            free_to_caller: *mut u64,
            total: *mut u64,
            total_free: *mut u64,
        ) -> i32;
    }

    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut free = 0u64;
    let mut total = 0u64;
    // SAFETY: the string is null terminated and outlives the call, as do both
    // counters; the third total is passed as null, which the call accepts.
    let ok =
        unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, &mut total, std::ptr::null_mut()) };
    (ok != 0).then_some(Space {
        available: free,
        total,
    })
}

#[cfg(not(any(unix, windows)))]
fn space_on_existing(_path: &Path) -> Option<Space> {
    None
}

/// What the governor wants a build to do next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Carry on with this many workers. Never more than the build started
    /// with, and never fewer than one.
    Workers(usize),
    /// There is no longer room to measure and write another round. Whatever is
    /// already stored stays; the rest of the commits are not attempted.
    ///
    /// `round` is what the commits in flight still have to write; it sits on
    /// top of [`RESERVE_BYTES`], which is why a stop can happen while there is
    /// still free space to look at.
    Stop { available: u64, round: u64 },
}

/// Watches how much the index grows per commit and how much room is left.
#[derive(Debug)]
pub struct DiskGovernor {
    /// The index directory. Checked by path, not by handle, because it is
    /// created part way through the first build.
    root: PathBuf,
    max_workers: usize,
    /// The cap in force. Only ever lowered: a worker that has stopped cannot
    /// be brought back inside the scope the others are running in, so raising
    /// it again would report a number that is not true.
    allowed: usize,
    measured: usize,
    total_bytes: u64,
    /// Recent size trend, weighted towards the last commits measured.
    trend: f64,
    /// Whether the worker count may be lowered. Off when the user pinned a
    /// thread count: that number is theirs to choose. The stop verdict still
    /// applies either way, because it guards the index, not the threads.
    throttling: bool,
    /// The reading the last verdict was made on, so what gets logged is what
    /// was decided on rather than a second, slightly different, reading.
    last_seen: Option<Space>,
    /// Set when the platform gives no free-space reading, or the first one
    /// failed. A governor that cannot measure does not interfere.
    blind: bool,
}

impl DiskGovernor {
    pub fn new(root: &Path, max_workers: usize, throttling: bool) -> Self {
        let last_seen = space(root);
        DiskGovernor {
            root: root.to_path_buf(),
            max_workers,
            allowed: max_workers,
            measured: 0,
            total_bytes: 0,
            trend: 0.0,
            throttling,
            blind: last_seen.is_none(),
            last_seen,
        }
    }

    /// True when free space cannot be read here, so no throttling will happen.
    pub fn is_blind(&self) -> bool {
        self.blind
    }

    /// The volume as the last verdict saw it.
    pub fn last_seen(&self) -> Option<Space> {
        self.last_seen
    }

    /// Take in what one measured commit added to the index.
    pub fn record(&mut self, record_bytes: u64) {
        self.measured += 1;
        self.total_bytes = self.total_bytes.saturating_add(record_bytes);
        self.trend = if self.measured == 1 {
            record_bytes as f64
        } else {
            self.trend * (1.0 - TREND_WEIGHT) + record_bytes as f64 * TREND_WEIGHT
        };
    }

    /// Bytes the next commit is expected to add: the running mean, raised to
    /// the recent trend when commits have been getting bigger.
    pub fn per_commit(&self) -> u64 {
        if self.measured == 0 {
            return 0;
        }
        let mean = self.total_bytes / self.measured as u64;
        mean.max(self.trend.round().max(0.0) as u64)
    }

    /// Re-read the free space and decide. `remaining` counts every commit not
    /// yet measured, including the ones workers are holding right now.
    ///
    /// Returns `None` while nothing needs to change, so a caller can log only
    /// the moments the answer moved.
    pub fn reassess(&mut self, remaining: usize) -> Option<Verdict> {
        if self.blind || self.measured == 0 || remaining == 0 {
            return None;
        }
        let Some(space) = space(&self.root) else {
            // A volume that stops answering is not a reason to stop working.
            self.blind = true;
            return None;
        };
        self.last_seen = Some(space);

        // The floor is about the commits actually in flight, which after a
        // throttle is the lowered count, not the one the build started on.
        match decide(
            self.max_workers,
            self.allowed,
            space,
            self.per_commit(),
            remaining,
        ) {
            stop @ Verdict::Stop { .. } => Some(stop),
            Verdict::Workers(workers) => {
                if !self.throttling {
                    return None;
                }
                let workers = workers.min(self.allowed);
                if workers >= self.allowed {
                    return None;
                }
                self.allowed = workers;
                Some(Verdict::Workers(workers))
            }
        }
    }
}

/// The whole policy, kept free of the filesystem so it can be tested.
///
/// `per_commit` is what one commit adds to the index and `remaining` how many
/// are left; `in_flight` is how many workers are measuring right now, which is
/// what the stop floor has to keep room for. The build gets every worker while
/// twice the projected growth still fits above the reserve, and loses workers
/// in proportion as that shrinks.
pub fn decide(
    max_workers: usize,
    in_flight: usize,
    space: Space,
    per_commit: u64,
    remaining: usize,
) -> Verdict {
    let max_workers = max_workers.max(1);
    if per_commit == 0 || remaining == 0 {
        return Verdict::Workers(max_workers);
    }

    let headroom = space.headroom();

    // Room for the commits in flight, twice over: one round to finish
    // measuring and one for the flush that writes them out.
    let in_flight = remaining.min(in_flight.clamp(1, max_workers)) as u64;
    let floor = per_commit
        .saturating_mul(in_flight)
        .saturating_mul(FLOOR_ROUNDS);
    if headroom < floor {
        return Verdict::Stop {
            available: space.available,
            round: floor,
        };
    }

    let projected = per_commit.saturating_mul(remaining as u64);
    let ratio = headroom as f64 / projected as f64;
    let workers = (max_workers as f64 * ratio / COMFORTABLE).round() as i64;
    Verdict::Workers(workers.clamp(1, max_workers as i64) as usize)
}

/// Byte counts in the log are read by a person, not parsed.
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB: u64 = 1024 * 1024;

    /// A volume big enough that the reserve is the flat cap.
    fn volume(available: u64) -> Space {
        Space {
            available,
            total: 500 * 1024 * MIB,
        }
    }

    #[test]
    fn a_roomy_volume_keeps_every_worker() {
        let space = volume(MAX_RESERVE_BYTES + 100 * 1024 * MIB);
        assert_eq!(decide(8, 8, space, MIB, 100), Verdict::Workers(8));
    }

    #[test]
    fn workers_fall_as_the_projected_build_stops_fitting() {
        let per_commit = 10 * MIB;
        let remaining = 100;
        let projected = per_commit * remaining as u64;

        // Twice the projection still fits: no throttling.
        let roomy = volume(MAX_RESERVE_BYTES + projected * 2);
        assert_eq!(
            decide(8, 8, roomy, per_commit, remaining),
            Verdict::Workers(8)
        );

        // Exactly the projection: half the workers.
        let tight = volume(MAX_RESERVE_BYTES + projected);
        assert_eq!(
            decide(8, 8, tight, per_commit, remaining),
            Verdict::Workers(4)
        );

        // A quarter of it: down to one, but still running.
        let tighter = volume(MAX_RESERVE_BYTES + projected / 4);
        assert_eq!(
            decide(8, 8, tighter, per_commit, remaining),
            Verdict::Workers(1)
        );
    }

    #[test]
    fn no_room_for_the_next_round_stops_the_build() {
        let per_commit = 10 * MIB;
        // Room for one commit, while eight are in flight and need two rounds.
        let space = volume(MAX_RESERVE_BYTES + per_commit);
        assert!(matches!(
            decide(8, 8, space, per_commit, 100),
            Verdict::Stop { .. }
        ));
    }

    #[test]
    fn the_floor_follows_the_workers_that_are_actually_running() {
        let per_commit = 10 * MIB;
        // Room for three commits: too little for eight workers to finish and
        // write, enough for the one worker a throttled build is down to.
        let space = volume(MAX_RESERVE_BYTES + per_commit * 3);
        assert!(matches!(
            decide(8, 8, space, per_commit, 100),
            Verdict::Stop { .. }
        ));
        assert!(matches!(
            decide(8, 1, space, per_commit, 100),
            Verdict::Workers(_)
        ));
    }

    #[test]
    fn the_reserve_is_never_spent() {
        // Everything that is left is reserve, so even one commit is too many.
        assert!(matches!(
            decide(4, 4, volume(MAX_RESERVE_BYTES), MIB, 10),
            Verdict::Stop { .. }
        ));
    }

    #[test]
    fn a_small_volume_reserves_a_share_of_itself_instead_of_the_cap() {
        // 300 MiB of tmpfs: holding back the flat 256 MiB would refuse to
        // write an index of a few hundred kilobytes.
        let small = Space {
            available: 290 * MIB,
            total: 300 * MIB,
        };
        assert_eq!(small.reserve(), 30 * MIB);
        assert_eq!(decide(8, 8, small, 8 * 1024, 100), Verdict::Workers(8));

        // The cap still applies to a volume large enough for it to bite.
        let large = Space {
            available: 400 * 1024 * MIB,
            total: 500 * 1024 * MIB,
        };
        assert_eq!(large.reserve(), MAX_RESERVE_BYTES);
    }

    #[test]
    fn a_build_that_has_measured_nothing_is_not_throttled() {
        assert_eq!(decide(8, 8, volume(0), 0, 100), Verdict::Workers(8));
    }

    #[test]
    fn the_projection_follows_the_trend_upwards() {
        let mut governor = DiskGovernor {
            root: PathBuf::from("/"),
            max_workers: 4,
            allowed: 4,
            measured: 0,
            total_bytes: 0,
            trend: 0.0,
            throttling: true,
            last_seen: None,
            blind: true,
        };
        // Small early commits, much larger recent ones: the mean alone would
        // under-estimate what the rest of the build is going to cost.
        for _ in 0..8 {
            governor.record(MIB);
        }
        for _ in 0..3 {
            governor.record(16 * MIB);
        }
        assert!(governor.per_commit() > 8 * MIB, "{}", governor.per_commit());
    }

    #[test]
    fn a_cap_is_only_lowered() {
        let mut governor = DiskGovernor::new(Path::new("/"), 8, true);
        governor.blind = true;
        governor.allowed = 2;
        // Blind governors say nothing at all, so the cap stays where it was.
        assert_eq!(governor.reassess(50), None);
        assert_eq!(governor.allowed, 2);
    }

    #[test]
    fn free_space_reads_through_a_directory_that_does_not_exist_yet() {
        let deep = std::env::temp_dir().join("cellular-not-created/one/two");
        let space = space(&deep).expect("a reading for the temp volume");
        assert!(space.total > 0);
        assert!(space.available <= space.total);
    }

    #[test]
    fn byte_counts_are_rounded_to_a_unit() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(1023), "1023 B");
        assert_eq!(human_bytes(1024), "1.0 KiB");
        assert_eq!(human_bytes(1024 * 1024 * 3 / 2), "1.5 MiB");
    }
}
