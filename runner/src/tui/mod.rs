//! The terminal interface.
//!
//! Screen regions, following the layout in REQUIREMENTS.md:
//!
//! ```text
//! ╭─── Cellular ───────────╮
//! │ Now opened at: (path)  │   the config list, selectable and scrollable
//! │ ● index_depth: 2 (…)   │
//! ╰────────────────────────╯
//!   (log area, with completion candidates drawn over it)
//! ──────────────────────────
//! ❯ (command line)
//! ──────────────────────────
//! (key guide)
//! ```

mod commands;
mod editor;
mod keys;
mod state;
#[cfg(test)]
mod tests;
mod ui;
mod viewer;

use anyhow::{Context, Result};
use crossterm::event::{self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyEventKind};
use crossterm::execute;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, mpsc};
use std::time::Duration;

use crate::builder::{self, Progress};
use crate::commits::ResolvedCommit;
use crate::config::{self, Config, FIELDS, Origin};
use crate::gitignore;
use crate::index::store::IndexStore;
use crate::model::Snapshot;

use self::editor::ArrayEditor;
use self::state::UiState;

/// How long the loop waits for a key before advancing animations.
const TICK: Duration = Duration::from_millis(100);
/// Lines of log kept in memory.
const LOG_LIMIT: usize = 2000;
/// Columns taken by the `❯ ` prompt on the command line.
pub const PROMPT_WIDTH: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    ConfigList,
    CommandLine,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogKind {
    Info,
    Command,
    Warn,
    Error,
    Success,
}

#[derive(Debug, Clone)]
pub struct LogLine {
    pub text: String,
    pub kind: LogKind,
}

/// A warning that stays on screen until the user dismisses it.
#[derive(Debug, Clone)]
pub struct Notice {
    pub key: String,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct CompletionItem {
    pub label: String,
    pub description: String,
}

/// The candidate list drawn over the log area.
#[derive(Debug, Clone)]
pub struct Completion {
    pub items: Vec<CompletionItem>,
    /// None until the user picks one with the arrow keys. While nothing is
    /// picked, enter runs the command instead of inserting a candidate.
    pub selected: Option<usize>,
    /// Character index in the input where the accepted text replaces from.
    pub replace_from: usize,
    /// Where the candidate block lines up horizontally.
    pub anchor: Anchor,
}

/// Command names line up with the left edge, as in the examples in
/// REQUIREMENTS.md; argument candidates follow the cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    LeftEdge,
    Cursor,
}

#[derive(Debug, Clone)]
pub enum ConfirmAction {
    /// Leaving with unsaved config changes.
    QuitWithUnsaved,
    /// A build large enough to be worth asking about.
    StartBuild {
        resolved: Vec<ResolvedCommit>,
        force: bool,
    },
    /// The stored index failed verification.
    RebuildDamaged { specs: Vec<String> },
}

#[derive(Debug, Clone)]
pub enum Mode {
    Normal,
    /// Editing a single-valued field in place.
    EditScalar {
        field: &'static str,
        buffer: String,
        cursor: usize,
    },
    /// Editing a list-valued field on its own screen.
    EditArray(ArrayEditor),
    Confirm {
        question: String,
        action: ConfirmAction,
    },
}

/// What a background thread reports back to the loop.
#[derive(Debug)]
pub enum JobMessage {
    Line(String),
    /// Commits settled so far, out of the whole resolved list.
    Progress {
        done: usize,
        total: usize,
    },
    Warn(String),
    Failed(String),
    BuildFinished(String),
    ViewerStarted(viewer::ViewerRecord),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    Build,
    Viewer,
}

pub struct Job {
    pub kind: JobKind,
    pub label: String,
    pub cancel: Arc<AtomicBool>,
    /// Commits settled out of the total the job covers, where it counts them.
    /// The viewer job has no commits to count and leaves this alone.
    pub progress: Option<(usize, usize)>,
    receiver: Receiver<JobMessage>,
}

pub struct App {
    /// Working tree root of the project the interface is opened on.
    pub project_root: PathBuf,
    /// Directory `cd` moves around in; path arguments resolve against it.
    pub cwd: PathBuf,
    pub config: Config,
    /// Fields changed in this session and not written to a file yet.
    pub dirty: BTreeSet<String>,
    pub focus: Focus,
    pub selected: usize,
    /// First visible display line of the config list.
    pub list_scroll: usize,
    /// Horizontal scroll of the selected config row.
    pub horizontal_scroll: usize,
    pub log: Vec<LogLine>,
    pub input: String,
    /// Cursor position in the input, in characters.
    pub cursor: usize,
    pub history: Vec<String>,
    pub history_position: Option<usize>,
    pub completion: Option<Completion>,
    pub mode: Mode,
    pub notices: Vec<Notice>,
    pub ui_state: UiState,
    /// Background work in progress. Jobs with distinct resources may run
    /// together: an index build owns the index writer while a viewer start
    /// only builds and launches the static web app.
    pub jobs: Vec<Job>,
    /// Character offset of the marquee in the key guide.
    pub guide_offset: usize,
    pub quit: bool,
}

impl App {
    fn new(project_root: PathBuf, cwd: PathBuf, config: Config) -> Self {
        App {
            project_root,
            cwd,
            config,
            dirty: BTreeSet::new(),
            focus: Focus::CommandLine,
            selected: 0,
            list_scroll: 0,
            horizontal_scroll: 0,
            log: Vec::new(),
            input: String::new(),
            cursor: 0,
            history: Vec::new(),
            history_position: None,
            completion: None,
            mode: Mode::Normal,
            notices: Vec::new(),
            ui_state: UiState::load(),
            jobs: Vec::new(),
            guide_offset: 0,
            quit: false,
        }
    }

    pub fn log(&mut self, kind: LogKind, text: impl Into<String>) {
        for line in text.into().split('\n') {
            self.log.push(LogLine {
                text: line.to_string(),
                kind,
            });
        }
        if self.log.len() > LOG_LIMIT {
            let excess = self.log.len() - LOG_LIMIT;
            self.log.drain(..excess);
        }
    }

    pub fn info(&mut self, text: impl Into<String>) {
        self.log(LogKind::Info, text);
    }
    pub fn warn(&mut self, text: impl Into<String>) {
        self.log(LogKind::Warn, text);
    }
    pub fn error(&mut self, text: impl Into<String>) {
        self.log(LogKind::Error, text);
    }
    pub fn success(&mut self, text: impl Into<String>) {
        self.log(LogKind::Success, text);
    }

    pub fn selected_field(&self) -> &'static str {
        FIELDS[self.selected.min(FIELDS.len() - 1)]
    }

    /// True while an index build runs in the background. Everything else in
    /// the interface stays usable during one; only the commands that touch the
    /// index files it is about to write have to wait.
    pub fn build_running(&self) -> bool {
        self.jobs.iter().any(|job| job.kind == JobKind::Build)
    }

    pub fn viewer_starting(&self) -> bool {
        self.jobs.iter().any(|job| job.kind == JobKind::Viewer)
    }

    /// Record a notice unless the user has already dismissed it for good.
    pub fn push_notice(&mut self, key: impl Into<String>, text: impl Into<String>) {
        let key = key.into();
        if self.ui_state.dismissed_notices.contains(&key) {
            return;
        }
        if self.notices.iter().any(|notice| notice.key == key) {
            return;
        }
        self.notices.push(Notice {
            key,
            text: text.into(),
        });
    }

    pub fn dismiss_first_notice(&mut self) {
        if self.notices.is_empty() {
            return;
        }
        let notice = self.notices.remove(0);
        self.ui_state.dismissed_notices.insert(notice.key.clone());
        if let Err(error) = self.ui_state.save() {
            self.warn(format!("could not remember the dismissal: {error:#}"));
        } else {
            self.info(format!("dismissed: {}", notice.text));
        }
    }

    /// Re-read the layered config and refresh the warnings that depend on it.
    pub fn reload_config(&mut self) -> Result<()> {
        self.config = config::load_layered(&self.project_root)?;
        self.dirty.clear();
        self.refresh_notices();
        Ok(())
    }

    pub fn refresh_notices(&mut self) {
        let project_root = self.project_root.clone();

        // (*3) The project config must stay tracked even where `.cellular` is
        // ignored.
        let status = gitignore::check(&project_root);
        for warning in status.warnings() {
            let key = format!("gitignore:{}:{warning}", project_root.display());
            self.push_notice(key, warning);
        }

        // index_depth is recommended to live in the project config.
        if !matches!(
            self.config.origin_of("index_depth"),
            Origin::ProjectConfig(_)
        ) {
            let path = config::project_config_path(&project_root);
            self.push_notice(
                format!("index_depth:{}", project_root.display()),
                format!(
                    "index_depth has no project-level value in {}",
                    path.display()
                ),
            );
        }
    }

    pub fn store_for_reading(&self) -> Result<Option<IndexStore>> {
        IndexStore::locate(&self.project_root, false)
    }

    /// Start a background index build. A second index build would contend for
    /// the store, but unrelated work such as starting the viewer may continue.
    ///
    /// Resolving the specs first is quick and tells us how much work the build
    /// is; a large one is worth confirming, since the first pass over a big
    /// repository reads every file of every commit.
    pub fn start_build(&mut self, specs: Vec<String>, force: bool) {
        if self.build_running() {
            self.error("an index build is already running; wait for it or run `index cancel build`");
            return;
        }
        if self.config.index_depth.is_none() {
            self.error("index_depth is not set; use `set index_depth <n>` first");
            return;
        }

        let resolved = (|| -> Result<Vec<ResolvedCommit>> {
            let repo = gix::discover(&self.project_root)?;
            builder::resolve(&repo, &specs, &self.config)
        })();
        let resolved = match resolved {
            Ok(resolved) => resolved,
            Err(error) => {
                self.error(format!("{error:#}"));
                return;
            }
        };
        if resolved.is_empty() {
            self.error("those specs select no commits");
            return;
        }

        if resolved.len() >= builder::LARGE_BUILD {
            self.mode = Mode::Confirm {
                question: format!(
                    "Index {} commits? The first pass reads every file in each one.",
                    resolved.len()
                ),
                action: ConfirmAction::StartBuild { resolved, force },
            };
            return;
        }
        self.spawn_build(resolved, force);
    }

    /// Run a resolved build in the background, streaming progress into the log.
    pub fn spawn_build(&mut self, resolved: Vec<ResolvedCommit>, force: bool) {
        let specs: Vec<String> = {
            let mut seen = Vec::new();
            for commit in &resolved {
                if !seen.contains(&commit.spec) {
                    seen.push(commit.spec.clone());
                }
            }
            seen
        };
        let (sender, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let label = format!("index build {}", specs.join(","));
        let total = resolved.len();
        self.info(format!("started: {label} ({total} commits)"));

        let thread_cancel = Arc::clone(&cancel);
        let project_root = self.project_root.clone();
        let config = self.config.clone();

        std::thread::spawn(move || {
            struct ChannelProgress {
                sender: mpsc::Sender<JobMessage>,
                cancel: Arc<AtomicBool>,
            }
            impl Progress for ChannelProgress {
                fn note(&mut self, message: &str) {
                    let _ = self.sender.send(JobMessage::Line(message.to_string()));
                }
                fn advance(&mut self, done: usize, total: usize) {
                    let _ = self.sender.send(JobMessage::Progress { done, total });
                }
                fn is_cancelled(&self) -> bool {
                    self.cancel.load(std::sync::atomic::Ordering::Relaxed)
                }
            }

            let mut progress = ChannelProgress {
                sender: sender.clone(),
                cancel: Arc::clone(&thread_cancel),
            };
            let outcome = (|| {
                let repo = gix::discover(&project_root)?;
                let store = IndexStore::locate_for_writing(&project_root)?;
                builder::build_resolved(
                    &repo,
                    &store,
                    &config,
                    &resolved,
                    force,
                    Some(Arc::clone(&thread_cancel)),
                    &mut progress,
                )
            })();

            let message = match outcome {
                Ok(outcome) => {
                    if outcome.discarded_damaged_index {
                        let _ = sender.send(JobMessage::Warn(
                            "the previous index was damaged and has been rewritten".to_string(),
                        ));
                    }
                    if outcome.low_disk_stop {
                        let _ = sender.send(JobMessage::Warn(
                            "stopped early to stay clear of filling the index volume; \
                             the commits that finished are stored"
                                .to_string(),
                        ));
                    } else if outcome.cancelled {
                        let _ = sender.send(JobMessage::Warn(
                            "cancelled; the commits that finished are stored".to_string(),
                        ));
                    }
                    if let Some(workers) = outcome.throttled_workers {
                        let _ = sender.send(JobMessage::Warn(format!(
                            "free space ran low, so the build finished on {workers} thread(s)"
                        )));
                    }
                    let measured = outcome.built.iter().filter(|item| !item.reused).count();
                    JobMessage::BuildFinished(format!(
                        "wrote {} record(s) across {} blob file(s), {} bytes; \
                         {measured} measured, {} reused",
                        outcome.write.record_count,
                        outcome.write.blob_count,
                        outcome.write.total_bytes,
                        outcome.built.len() - measured
                    ))
                }
                Err(error) => JobMessage::Failed(format!("{error:#}")),
            };
            let _ = sender.send(message);
        });

        self.jobs.push(Job {
            kind: JobKind::Build,
            label,
            cancel,
            // Shown from the moment the badge appears, rather than only once
            // the first commit comes back.
            progress: Some((0, total)),
            receiver,
        });
    }

    /// Start the viewer's web server in the background.
    pub fn start_viewer(&mut self) {
        if self.viewer_starting() {
            self.error("the viewer is already starting; wait for it to finish");
            return;
        }
        let (sender, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        self.info("starting the viewer web server");

        std::thread::spawn(move || {
            let forward = sender.clone();
            let mut log = move |line: String| {
                let _ = forward.send(JobMessage::Line(line));
            };
            let message = match viewer::start(&mut log) {
                Ok(record) => JobMessage::ViewerStarted(record),
                Err(error) => JobMessage::Failed(format!("{error:#}")),
            };
            let _ = sender.send(message);
        });

        self.jobs.push(Job {
            kind: JobKind::Viewer,
            label: "viewer start".to_string(),
            cancel,
            progress: None,
            receiver,
        });
    }

    pub fn cancel_job(&mut self) {
        let Some(job) = self.jobs.iter().find(|job| job.kind == JobKind::Build) else {
            return;
        };
        job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        let label = job.label.clone();
        self.warn(format!("cancelling {label}"));
    }

    /// Drain whatever the background thread has produced since the last pass.
    fn poll_job(&mut self) {
        let mut finished = Vec::new();
        let mut pending = Vec::new();
        for (index, job) in self.jobs.iter().enumerate() {
            loop {
                match job.receiver.try_recv() {
                    Ok(message) => pending.push((index, message)),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        finished.push(index);
                        break;
                    }
                }
            }
        }

        for (index, message) in pending {
            match message {
                JobMessage::Line(text) => self.info(format!("  {text}")),
                JobMessage::Progress { done, total } => {
                    if let Some(job) = self.jobs.get_mut(index) {
                        job.progress = Some((done, total));
                    }
                }
                JobMessage::Warn(text) => self.warn(text),
                JobMessage::Failed(text) => {
                    self.error(text);
                    finished.push(index);
                }
                JobMessage::BuildFinished(text) => {
                    self.success(text);
                    finished.push(index);
                }
                JobMessage::ViewerStarted(record) => {
                    self.success(format!(
                        "the viewer is running at {} (pid {})",
                        record.url, record.pid
                    ));
                    finished.push(index);
                }
            }
        }

        finished.sort_unstable();
        finished.dedup();
        for index in finished.into_iter().rev() {
            self.jobs.remove(index);
        }
    }

    /// Ask about unsaved changes before leaving, if there are any.
    pub fn request_quit(&mut self) {
        // A build writes the index at the very end; tell it to stop so leaving
        // cannot interrupt that write half way through.
        for job in self.jobs.iter().filter(|job| job.kind == JobKind::Build) {
            job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        if self.dirty.is_empty() {
            self.quit = true;
            return;
        }
        let fields: Vec<&str> = self.dirty.iter().map(String::as_str).collect();
        self.mode = Mode::Confirm {
            question: format!(
                "Save {} to {} before leaving?",
                fields.join(", "),
                config::project_config_path(&self.project_root).display()
            ),
            action: ConfirmAction::QuitWithUnsaved,
        };
    }
}

/// Enter the terminal interface. Returns once the user leaves it.
pub fn run(project_root: PathBuf, cwd: PathBuf, config: Config) -> Result<()> {
    let mut app = App::new(project_root, cwd, config);
    app.refresh_notices();
    app.info("Cellular terminal interface. Type `help` to see the commands.");
    check_stored_index(&mut app);

    let mut terminal = ratatui::init();
    // ratatui::init does not turn this on, and without it Event::Paste never
    // arrives and a paste arrives as a burst of key presses.
    let pasting = execute!(std::io::stdout(), EnableBracketedPaste).is_ok();
    let result = event_loop(&mut terminal, &mut app);
    if pasting {
        let _ = execute!(std::io::stdout(), DisableBracketedPaste);
    }
    ratatui::restore();
    result
}

fn event_loop(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> Result<()> {
    while !app.quit {
        terminal
            .draw(|frame| ui::render(frame, app))
            .context("failed to draw the terminal interface")?;

        if event::poll(TICK)? {
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => keys::handle(app, key),
                Event::Paste(text) => keys::paste(app, &text),
                _ => {}
            }
        } else {
            app.guide_offset = app.guide_offset.wrapping_add(1);
        }
        app.poll_job();
    }
    Ok(())
}

/// (*2) Warn about a damaged index at start-up and offer to rebuild it.
fn check_stored_index(app: &mut App) {
    let store = match app.store_for_reading() {
        Ok(Some(store)) => store,
        Ok(None) => return,
        Err(error) => {
            app.warn(format!("could not locate the index: {error:#}"));
            return;
        }
    };
    if !store.exists() {
        return;
    }
    let report = match store.verify() {
        Ok(report) => report,
        Err(error) => {
            app.warn(format!("could not verify the index: {error:#}"));
            return;
        }
    };
    if report.is_healthy() {
        app.info(format!(
            "index at {}: {} blob file(s), {} snapshot(s)",
            store.root.display(),
            report.blob_count,
            report.record_count
        ));
        return;
    }

    app.warn("the indexed data may be damaged:");
    for problem in &report.problems {
        app.warn(format!("  - {problem}"));
    }

    let specs = salvage_specs(&store);
    let question = if specs.is_empty() {
        "Rebuild the index? No commit could be recovered, so this only clears it.".to_string()
    } else {
        format!("Rebuild the index for {}?", specs.join(", "))
    };
    app.mode = Mode::Confirm {
        question,
        action: ConfirmAction::RebuildDamaged { specs },
    };
}

/// Recover the commits of whatever records still parse, so a rebuild after
/// damage can cover the same history without asking the user to retype it.
fn salvage_specs(store: &IndexStore) -> Vec<String> {
    store
        .load_records_best_effort()
        .iter()
        .filter_map(|record| crate::index::format::decode_snapshot(record).ok())
        .map(|snapshot: Snapshot| snapshot.oid_hex())
        .collect()
}

/// Shorten a path for display, preferring a project-relative form.
pub fn shorten_path(path: &Path, project_root: &Path) -> String {
    if let Ok(relative) = path.strip_prefix(project_root) {
        let name = project_root
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| project_root.display().to_string());
        return format!("{name}/{}", relative.display());
    }
    if let Some(home) = dirs::home_dir()
        && let Ok(relative) = path.strip_prefix(&home)
    {
        return format!("~/{}", relative.display());
    }
    path.display().to_string()
}
