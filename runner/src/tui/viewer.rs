//! Controlling the viewer's web server from the terminal interface.
//!
//! The server is a pnpm project: `pnpm run build` produces the static site and
//! `pnpm run preview` serves it on port 8080. The preview process is detached
//! into its own process group and its pid recorded under the user profile, so
//! it keeps running after the terminal interface exits and a later session can
//! still stop it.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::config::profile_dir;

pub const PORT: u16 = 8080;
pub const RECORD_FILE: &str = "viewer.json";
pub const LOG_FILE: &str = "viewer.log";
/// Opened by `viewer open` when no local server is running.
pub const PUBLISHED_URL: &str = "https://shapelayer.github.io/cellular/";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewerRecord {
    pub pid: u32,
    pub port: u16,
    pub url: String,
    /// The process start time as the system reports it, captured when the
    /// server was spawned. Pids get reused, so this is what actually
    /// identifies the process before anything signals it.
    #[serde(default)]
    pub started: Option<String>,
}

pub fn record_path() -> Result<PathBuf> {
    Ok(profile_dir()?.join(RECORD_FILE))
}

pub fn log_path() -> Result<PathBuf> {
    Ok(profile_dir()?.join(LOG_FILE))
}

pub fn load_record() -> Option<ViewerRecord> {
    let path = record_path().ok()?;
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn save_record(record: &ViewerRecord) -> Result<()> {
    let path = record_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, serde_json::to_string_pretty(record)? + "\n")?;
    Ok(())
}

pub fn clear_record() {
    if let Ok(path) = record_path() {
        let _ = std::fs::remove_file(path);
    }
}

/// The record of a server that is actually still alive.
///
/// A recorded pid is not enough on its own: the operating system reuses pids,
/// and `stop` signals a whole process group, so acting on a stale record could
/// terminate something unrelated. The recorded process must still look like
/// the server that was started.
pub fn running() -> Option<ViewerRecord> {
    let record = load_record()?;
    if is_our_server(&record) {
        Some(record)
    } else {
        clear_record();
        None
    }
}

#[cfg(unix)]
fn is_our_server(record: &ViewerRecord) -> bool {
    // Matching on the command line is not enough: any process whose arguments
    // happen to mention pnpm or vite would qualify, and `stop` signals a whole
    // process group. The start time pins down one specific process.
    let Some(started) = &record.started else {
        return false;
    };
    process_start_time(record.pid).is_some_and(|current| &current == started)
}

#[cfg(not(unix))]
fn is_our_server(_record: &ViewerRecord) -> bool {
    // Without a portable way to inspect the process, trust the record until
    // stop clears it.
    true
}

/// The start time of a running process, or `None` when it is gone.
#[cfg(unix)]
fn process_start_time(pid: u32) -> Option<String> {
    let output = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "lstart="])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let started = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if started.is_empty() {
        None
    } else {
        Some(started)
    }
}

#[cfg(not(unix))]
fn process_start_time(_pid: u32) -> Option<String> {
    None
}

/// Locate the `viewer/` pnpm project that ships alongside the runner.
pub fn viewer_dir() -> Result<PathBuf> {
    if let Ok(from_env) = std::env::var("CELLULAR_VIEWER_DIR") {
        let path = PathBuf::from(from_env);
        if path.join("package.json").is_file() {
            return Ok(path);
        }
        bail!("CELLULAR_VIEWER_DIR does not point at a pnpm project: {path:?}");
    }

    // The repository layout during development: <root>/runner and <root>/viewer.
    let beside_manifest = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|root| root.join("viewer"));
    if let Some(path) = beside_manifest
        && path.join("package.json").is_file()
    {
        return Ok(path);
    }

    // An installed binary: walk up from the executable looking for viewer/.
    if let Ok(exe) = std::env::current_exe() {
        let mut current = exe.parent();
        while let Some(dir) = current {
            let candidate = dir.join("viewer");
            if candidate.join("package.json").is_file() {
                return Ok(candidate);
            }
            current = dir.parent();
        }
    }

    bail!("could not find the viewer project; set CELLULAR_VIEWER_DIR to its path")
}

/// Build the static site, then start the preview server detached.
///
/// `log` receives progress lines; it is called from whichever thread runs this.
pub fn start(log: &mut dyn FnMut(String)) -> Result<ViewerRecord> {
    if let Some(record) = running() {
        bail!(
            "the viewer is already running at {} (pid {})",
            record.url,
            record.pid
        );
    }

    let dir = viewer_dir()?;
    log(format!("viewer project: {}", dir.display()));

    if !dir.join("node_modules").is_dir() {
        log("installing viewer dependencies with pnpm install".to_string());
        run_to_completion(&dir, &["install"], log)?;
    }

    log("building the viewer with pnpm run build".to_string());
    run_to_completion(&dir, &["run", "build"], log)?;

    let log_file = log_path()?;
    if let Some(parent) = log_file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let out = std::fs::File::create(&log_file)
        .with_context(|| format!("failed to create {}", log_file.display()))?;
    let err = out.try_clone()?;

    let mut command = Command::new("pnpm");
    command
        .args(["run", "preview"])
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::from(out))
        .stderr(Stdio::from(err));
    detach(&mut command);

    let child = command
        .spawn()
        .context("failed to start pnpm; is pnpm installed and on PATH?")?;

    let pid = child.id();
    let record = ViewerRecord {
        pid,
        port: PORT,
        url: format!("http://localhost:{PORT}"),
        started: process_start_time(pid),
    };
    save_record(&record)?;
    log(format!("server log: {}", log_file.display()));
    Ok(record)
}

#[cfg(unix)]
fn detach(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    // Its own process group, so ctrl+c in the terminal interface and the exit
    // of this process both leave the server alone.
    command.process_group(0);
}

#[cfg(not(unix))]
fn detach(_command: &mut Command) {}

/// Run a pnpm subcommand to completion, forwarding its output to `log`.
fn run_to_completion(dir: &Path, args: &[&str], log: &mut dyn FnMut(String)) -> Result<()> {
    let output = Command::new("pnpm")
        .args(args)
        .current_dir(dir)
        .output()
        .context("failed to run pnpm; is pnpm installed and on PATH?")?;

    for stream in [&output.stdout, &output.stderr] {
        for line in String::from_utf8_lossy(stream).lines() {
            if !line.trim().is_empty() {
                log(format!("  {}", line.trim_end()));
            }
        }
    }
    if !output.status.success() {
        bail!("pnpm {} failed", args.join(" "));
    }
    Ok(())
}

/// Stop a running server. Returns false when there was nothing to stop.
pub fn stop() -> Result<bool> {
    let Some(record) = running() else {
        clear_record();
        return Ok(false);
    };
    terminate(record.pid)?;
    clear_record();
    Ok(true)
}

#[cfg(unix)]
fn terminate(pid: u32) -> Result<()> {
    // The negative pid targets the whole process group started by `detach`.
    let status = Command::new("kill")
        .args(["-TERM", &format!("-{pid}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("failed to run kill")?;
    if !status.success() {
        // Fall back to the single process in case the group is already gone.
        let _ = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    Ok(())
}

#[cfg(not(unix))]
fn terminate(pid: u32) -> Result<()> {
    Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .status()
        .context("failed to run taskkill")?;
    Ok(())
}

/// Open a URL in the user's browser.
pub fn open_url(url: &str) -> Result<()> {
    let (program, args): (&str, Vec<&str>) = if cfg!(target_os = "macos") {
        ("open", vec![url])
    } else if cfg!(target_os = "windows") {
        ("cmd", vec!["/C", "start", "", url])
    } else {
        ("xdg-open", vec![url])
    };
    Command::new(program)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("failed to run {program}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(pid: u32, started: Option<String>) -> ViewerRecord {
        ViewerRecord {
            pid,
            port: PORT,
            url: format!("http://localhost:{PORT}"),
            started,
        }
    }

    #[test]
    #[cfg(unix)]
    fn only_the_recorded_process_counts_as_the_server() {
        let mine = std::process::id();
        let started = process_start_time(mine).expect("this process is running");

        // A record written for this very process matches it.
        assert!(is_our_server(&record(mine, Some(started.clone()))));

        // The same pid with a different start time is a reused pid, not ours.
        assert!(!is_our_server(&record(
            mine,
            Some("Thu Jan  1 00:00:00 1970".into())
        )));

        // A record from before start times were captured is never acted on.
        assert!(!is_our_server(&record(mine, None)));

        // A pid that cannot exist is not the server either.
        assert!(!is_our_server(&record(u32::MAX, Some(started))));
    }
}
