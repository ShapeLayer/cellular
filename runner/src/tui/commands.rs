//! The command line of the terminal interface: the command table, execution
//! and the candidate lists that back tab completion.

use anyhow::{Context, Result, bail};
use std::path::PathBuf;

use crate::commits;
use crate::config::{self, Config, ConfigFile, DateMultiple, DateNone, FIELDS, Origin, Threads};
use crate::filters::{split_patterns, validate_patterns};
use crate::index::store::{BLOBS_DIR, INDEX_FILE};

use super::viewer;
use super::{Anchor, App, Completion, CompletionItem, LogKind};

pub struct CommandSpec {
    pub name: &'static str,
    pub summary: &'static str,
    pub usage: &'static str,
    pub details: &'static str,
}

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        name: "help",
        summary: "Show commands and their descriptions. Use help <command> for more information on a specific command.",
        usage: "help [command]",
        details: "Without an argument, lists every command. With one, shows that command's usage.",
    },
    CommandSpec {
        name: "cd",
        summary: "Change the current working directory to the specified path.",
        usage: "cd <path>",
        details: "Path arguments resolve against this directory. Moving into a different git repository reopens the interface on that project.",
    },
    CommandSpec {
        name: "ls",
        summary: "List the contents of the directory the interface is opened on.",
        usage: "ls [path]",
        details: "Without an argument, lists the directory shown as `Now opened at`. A path resolves against the current working directory, as `cd` does.",
    },
    CommandSpec {
        name: "open",
        summary: "Open the directory the interface is opened on in the system file manager.",
        usage: "open [path]",
        details: "Finder on macOS, File Explorer on Windows, xdg-open elsewhere; where the system has none, the command says so. Without an argument, opens the directory shown as `Now opened at`.",
    },
    CommandSpec {
        name: "set",
        summary: "Set the value of a configuration option.",
        usage: "set <config_key> <config_value>",
        details: "The change is kept in memory only; write it out with `save`. List values accept `a,b` or `[a, b]`.",
    },
    CommandSpec {
        name: "get",
        summary: "Get the value of a configuration option.",
        usage: "get <config_key>",
        details: "Prints the current value and where it came from.",
    },
    CommandSpec {
        name: "save",
        summary: "Write configuration changes to a config.json file.",
        usage: "save [--project | --user]",
        details: "Writes only the fields changed in this session. Defaults to the project config at .cellular/config.json.",
    },
    CommandSpec {
        name: "index",
        summary: "Build or remove index data. Use index help for more information on available subcommands.",
        usage: "index <build <commits> [--force] | cancel build | export [path] | clear | help>",
        details: "build runs in the background and streams progress into the log; `index cancel build` stops it. export packages the index as a .cellexport file for the viewer, and works part way through a build. clear removes INDEX and BLOBS but keeps config.json.",
    },
    CommandSpec {
        name: "viewer",
        summary: "Control the viewer. Use viewer help for more information on available subcommands.",
        usage: "viewer <start | stop | restart | open | help>",
        details: "start builds the viewer and serves it on port 8080 in a detached process that outlives this interface.",
    },
    CommandSpec {
        name: "clear",
        summary: "Clear the log area.",
        usage: "clear",
        details: "Removes every line currently shown in the log area.",
    },
    CommandSpec {
        name: "exit",
        summary: "Leave the terminal interface.",
        usage: "exit",
        details: "Asks what to do about unsaved configuration changes before leaving.",
    },
];

const INDEX_SUBCOMMANDS: &[(&str, &str)] = &[
    (
        "build",
        "Index the given commits with the current configuration.",
    ),
    ("cancel", "Stop the index build that is running."),
    ("clear", "Delete the index files built for this project."),
    ("help", "Show the index subcommands."),
];

const INDEX_CANCEL_TARGETS: &[(&str, &str)] = &[("build", "Stop the running index build.")];

const VIEWER_SUBCOMMANDS: &[(&str, &str)] = &[
    ("start", "Build the viewer and serve it on port 8080."),
    ("stop", "Stop the running viewer web server."),
    (
        "restart",
        "Stop the viewer web server, then start it again.",
    ),
    ("open", "Open the viewer in a web browser."),
    ("help", "Show the viewer subcommands."),
];

const SAVE_FLAGS: &[(&str, &str)] = &[
    (
        "--project",
        "Write to the project config at .cellular/config.json.",
    ),
    (
        "--user",
        "Write to the user config at ~/.cellular/config.json.",
    ),
];

fn find(name: &str) -> Option<&'static CommandSpec> {
    COMMANDS.iter().find(|command| command.name == name)
}

// ------------------------------------------------------------- execution --

pub fn execute(app: &mut App, line: &str) {
    let line = line.trim();
    if line.is_empty() {
        return;
    }
    app.log(LogKind::Command, format!("❯ {line}"));

    let mut parts = line.split_whitespace();
    let name = parts.next().unwrap_or_default();
    let arguments: Vec<&str> = parts.collect();

    let result = match name {
        "help" => run_help(app, &arguments),
        "cd" => run_cd(app, &arguments),
        "ls" => run_ls(app, &arguments),
        "open" => run_open(app, &arguments),
        "set" => run_set(app, &arguments),
        "get" => run_get(app, &arguments),
        "save" => run_save(app, &arguments),
        "index" => run_index(app, &arguments),
        "viewer" => run_viewer(app, &arguments),
        "clear" => {
            app.log.clear();
            Ok(())
        }
        "exit" | "quit" => {
            app.request_quit();
            Ok(())
        }
        other => Err(anyhow::anyhow!(
            "unknown command {other:?}; type `help` to see the commands"
        )),
    };

    if let Err(error) = result {
        app.error(format!("{error:#}"));
    }
}

fn run_help(app: &mut App, arguments: &[&str]) -> Result<()> {
    match arguments.first() {
        None => {
            for command in COMMANDS {
                app.info(format!("  {:<10} {}", command.name, command.summary));
            }
        }
        Some(name) => {
            let command = find(name).with_context(|| format!("no such command: {name}"))?;
            app.info(format!("  usage: {}", command.usage));
            app.info(format!("  {}", command.details));
        }
    }
    Ok(())
}

fn run_cd(app: &mut App, arguments: &[&str]) -> Result<()> {
    let target = arguments.first().copied().unwrap_or("~");
    let path = resolve_path(&app.cwd, target);
    let path = path
        .canonicalize()
        .with_context(|| format!("no such directory: {}", path.display()))?;
    if !path.is_dir() {
        bail!("not a directory: {}", path.display());
    }

    // Moving into another repository reopens the interface on that project.
    let new_root = gix::discover(&path)
        .ok()
        .and_then(|repo| repo.workdir().map(Path::to_path_buf));

    app.cwd = path.clone();
    match new_root {
        Some(root) if root != app.project_root => {
            // The build indexes the project the interface was opened on;
            // moving to another one would leave it writing out of sight.
            if app.build_running() {
                app.cwd = app.project_root.clone();
                bail!(
                    "an index build is running for this project; wait for it or run                      `index cancel build`, then change directory"
                );
            }
            if !app.dirty.is_empty() {
                app.cwd = app.project_root.clone();
                bail!(
                    "there are unsaved configuration changes; run `save` first, \
                     then change directory"
                );
            }
            app.project_root = root.clone();
            app.notices.clear();
            app.reload_config()?;
            app.success(format!("opened {}", root.display()));
        }
        Some(_) => app.info(format!("now at {}", path.display())),
        None => app.warn(format!(
            "{} is not inside a git repository; still opened on {}",
            path.display(),
            app.project_root.display()
        )),
    }
    Ok(())
}

use std::path::Path;

fn resolve_path(base: &Path, raw: &str) -> PathBuf {
    if let Some(rest) = raw.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest);
    }
    if raw == "~"
        && let Some(home) = dirs::home_dir()
    {
        return home;
    }
    let candidate = Path::new(raw);
    if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        base.join(candidate)
    }
}

/// How many entries a listing prints before it stops. A directory such as
/// `node_modules` would otherwise push the whole log out of reach.
const LS_LIMIT: usize = 200;

/// The directory a `ls` or `open` argument names. Both default to the project
/// the interface is opened on, which is the path the header shows.
fn target_directory(app: &App, arguments: &[&str]) -> Result<PathBuf> {
    let path = match arguments.first() {
        Some(raw) => resolve_path(&app.cwd, raw),
        None => app.project_root.clone(),
    };
    let path = path
        .canonicalize()
        .with_context(|| format!("no such path: {}", path.display()))?;
    if !path.is_dir() {
        bail!("not a directory: {}", path.display());
    }
    Ok(path)
}

fn run_ls(app: &mut App, arguments: &[&str]) -> Result<()> {
    let path = target_directory(app, arguments)?;
    let entries =
        std::fs::read_dir(&path).with_context(|| format!("cannot read {}", path.display()))?;

    // A broken symlink has no metadata to read; it is still an entry of the
    // directory, so it is listed with what is known about it.
    let mut rows: Vec<(bool, String, Option<u64>)> = Vec::new();
    for entry in entries {
        let entry = entry.with_context(|| format!("cannot read {}", path.display()))?;
        let name = entry.file_name().to_string_lossy().to_string();
        let metadata = entry.metadata().ok();
        let is_dir = metadata.as_ref().is_some_and(|data| data.is_dir());
        let size = metadata
            .filter(|data| data.is_file())
            .map(|data| data.len());
        rows.push((is_dir, name, size));
    }
    // Directories first, then by name, so the shape of the tree reads first.
    rows.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));

    let directories = rows.iter().filter(|(is_dir, ..)| *is_dir).count();
    let files = rows.len() - directories;
    app.info(format!("  {}", path.display()));
    if rows.is_empty() {
        app.info("  (empty)");
        return Ok(());
    }
    for (is_dir, name, size) in rows.iter().take(LS_LIMIT) {
        // The size leads so the names line up whatever they are made of;
        // padding a name by character count misaligns wide ones.
        let size = match (is_dir, size) {
            (true, _) => "<dir>".to_string(),
            (false, Some(bytes)) => crate::disk::human_bytes(*bytes),
            (false, None) => "?".to_string(),
        };
        let slash = if *is_dir { "/" } else { "" };
        app.info(format!("  {size:>9}  {name}{slash}"));
    }
    if rows.len() > LS_LIMIT {
        app.info(format!("  … {} more not listed", rows.len() - LS_LIMIT));
    }
    app.info(format!("  {directories} directories, {files} files"));
    Ok(())
}

fn run_open(app: &mut App, arguments: &[&str]) -> Result<()> {
    let path = target_directory(app, arguments)?;
    open_in_file_manager(&path)?;
    app.success(format!("opened {} in the file manager", path.display()));
    Ok(())
}

/// Hand a directory to whatever the system browses files with. Not every
/// system has one — a machine with no desktop has no `xdg-open` to run — so a
/// failure to start it is reported as such rather than left unexplained.
fn open_in_file_manager(path: &Path) -> Result<()> {
    // `canonicalize` hands back an extended-length path on Windows, which the
    // file manager does not understand; the prefix comes off before it is used.
    let path = path.display().to_string();
    let path = path.strip_prefix(r"\\?\").unwrap_or(&path);

    let (program, args): (&str, Vec<&str>) = if cfg!(target_os = "macos") {
        ("open", vec![path])
    } else if cfg!(target_os = "windows") {
        ("explorer", vec![path])
    } else {
        ("xdg-open", vec![path])
    };
    std::process::Command::new(program)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .with_context(|| {
            format!("could not run {program}; this system may have no file manager to open")
        })?;
    Ok(())
}

fn run_set(app: &mut App, arguments: &[&str]) -> Result<()> {
    let [field, rest @ ..] = arguments else {
        bail!("usage: set <config_key> <config_value>");
    };
    if rest.is_empty() {
        bail!("usage: set <config_key> <config_value>");
    }
    apply_field_value(app, field, &rest.join(" "))?;
    let value = app.config.display_value(field).unwrap_or_default();
    app.success(format!("{field} = {value} (unsaved)"));
    Ok(())
}

fn run_get(app: &mut App, arguments: &[&str]) -> Result<()> {
    let Some(field) = arguments.first() else {
        bail!("usage: get <config_key>");
    };
    let value = app
        .config
        .display_value(field)
        .with_context(|| format!("no such configuration field: {field}"))?;
    let origin = describe_origin(app, field);
    app.info(format!("  {field} = {value} ({origin})"));
    Ok(())
}

fn run_save(app: &mut App, arguments: &[&str]) -> Result<()> {
    let to_user = match arguments.first().copied() {
        None | Some("--project") => false,
        Some("--user") => true,
        Some(other) => bail!("unknown option {other:?}; use --project or --user"),
    };
    if app.dirty.is_empty() {
        app.info("nothing to save");
        return Ok(());
    }
    let path = save_config(app, to_user)?;
    app.success(format!("saved to {}", path.display()));
    Ok(())
}

/// Write the fields changed in this session into a config file, leaving every
/// other key in that file untouched.
pub fn save_config(app: &mut App, to_user: bool) -> Result<PathBuf> {
    let path = if to_user {
        config::user_config_path()?
    } else {
        config::project_config_path(&app.project_root)
    };

    let mut file = ConfigFile::load(&path)?.unwrap_or_default();
    let changed: Vec<String> = app.dirty.iter().cloned().collect();
    for field in &changed {
        file.take_field(&app.config, field);
    }
    file.save(&path)?;

    let origin = if to_user {
        Origin::UserConfig(path.clone())
    } else {
        Origin::ProjectConfig(path.clone())
    };
    for field in changed {
        app.config.origins.insert(field, origin.clone());
    }
    app.dirty.clear();
    app.refresh_notices();
    Ok(path)
}

fn run_index(app: &mut App, arguments: &[&str]) -> Result<()> {
    match arguments.first().copied() {
        Some("build") => {
            let rest = &arguments[1..];
            let force = rest.contains(&"--force");
            let raw: Vec<&str> = rest
                .iter()
                .copied()
                .filter(|argument| *argument != "--force")
                .collect();
            if raw.is_empty() {
                bail!("usage: index build <commits> [--force]");
            }
            let specs = commits::split_specs(&raw.join(","));
            if specs.is_empty() {
                bail!("no commits were given");
            }
            app.start_build(specs, force);
            Ok(())
        }
        Some("cancel") => {
            // Written as a sentence, `index cancel build`; the object is
            // optional because there is only one job this can stop.
            match arguments.get(1).copied() {
                None | Some("build") => {}
                Some(other) => bail!("unknown cancel target {other:?}; usage: index cancel build"),
            }
            if app.build_running() {
                app.cancel_job();
                Ok(())
            } else {
                bail!("no index build is running")
            }
        }
        Some("export") => {
            // A running build is no reason to wait. It rewrites the store
            // whole every couple of seconds, so what sits on disk is an index
            // of the commits measured so far rather than a half-written one,
            // and `export::write` holds the store lock to stay clear of the
            // rewrite itself.
            let Some(store) = app.store_for_reading()? else {
                bail!("no index has been built for this project");
            };
            let path = match arguments.get(1) {
                Some(raw) => resolve_path(&app.cwd, raw),
                None => crate::export::default_path(&app.project_root),
            };
            let part_way = app.build_running();
            let report = crate::export::write(&store, &path)?;
            let note = if part_way {
                ", holding the commits the running build has measured so far"
            } else {
                ""
            };
            app.success(format!(
                "exported {} snapshot(s) in {} blob file(s) to {} ({} bytes){note}",
                report.record_count,
                report.blob_count,
                report.path.display(),
                report.bytes
            ));
            Ok(())
        }
        Some("clear") => {
            if app.build_running() {
                bail!(
                    "an index build is running; wait for it or run `index cancel build`                      before clearing"
                );
            }
            let Some(store) = app.store_for_reading()? else {
                app.info("no index has been built for this project");
                return Ok(());
            };
            let mut removed = false;
            let index_file = store.root.join(INDEX_FILE);
            if index_file.exists() {
                std::fs::remove_file(&index_file)?;
                removed = true;
            }
            let blobs = store.root.join(BLOBS_DIR);
            if blobs.exists() {
                std::fs::remove_dir_all(&blobs)?;
                removed = true;
            }
            if removed {
                app.success(format!("cleared the index at {}", store.root.display()));
            } else {
                app.info("there was no index data to clear");
            }
            Ok(())
        }
        Some("help") | None => {
            for (name, description) in INDEX_SUBCOMMANDS {
                app.info(format!("  index {name:<8} {description}"));
            }
            Ok(())
        }
        Some(other) => bail!("unknown index subcommand {other:?}; try `index help`"),
    }
}

fn run_viewer(app: &mut App, arguments: &[&str]) -> Result<()> {
    match arguments.first().copied() {
        Some("start") => {
            app.start_viewer();
            Ok(())
        }
        Some("stop") => {
            if viewer::stop()? {
                app.success("the viewer web server has stopped");
            } else {
                app.info("the viewer web server was not running");
            }
            Ok(())
        }
        Some("restart") => {
            if viewer::stop()? {
                app.info("the viewer web server has stopped");
            }
            app.start_viewer();
            Ok(())
        }
        Some("open") => {
            let url = match viewer::running() {
                Some(record) => record.url,
                None => {
                    app.info("no local viewer is running; opening the published viewer");
                    viewer::PUBLISHED_URL.to_string()
                }
            };
            viewer::open_url(&url)?;
            app.success(format!("opened {url}"));
            Ok(())
        }
        Some("help") | None => {
            for (name, description) in VIEWER_SUBCOMMANDS {
                app.info(format!("  viewer {name:<8} {description}"));
            }
            if let Some(record) = viewer::running() {
                app.info(format!("  running at {} (pid {})", record.url, record.pid));
            }
            Ok(())
        }
        Some(other) => bail!("unknown viewer subcommand {other:?}; try `viewer help`"),
    }
}

/// Parse and store a new value for a config field, marking it unsaved.
pub fn apply_field_value(app: &mut App, field: &str, raw: &str) -> Result<()> {
    let field = FIELDS
        .iter()
        .find(|known| **known == field)
        .with_context(|| format!("no such configuration field: {field}"))?;
    parse_into(&mut app.config, field, raw)?;
    app.config
        .origins
        .insert((*field).to_string(), Origin::Session);
    app.dirty.insert((*field).to_string());
    // Editing settings during a build is allowed, but the build took its own
    // copy when it started; say so rather than let the result look wrong.
    if app.build_running() {
        app.info("the running build keeps the configuration it started with");
    }
    Ok(())
}

/// Store a list value element by element, without going through the comma
/// separated form, so elements may contain commas.
pub fn apply_list_value(app: &mut App, field: &str, value: Vec<String>) -> Result<()> {
    let field = FIELDS
        .iter()
        .find(|known| **known == field)
        .with_context(|| format!("no such configuration field: {field}"))?;
    validate_patterns(&value)?;
    let target = match *field {
        "index_exclude" => &mut app.config.index_exclude,
        "index_detect_as_module" => &mut app.config.index_detect_as_module,
        "ignoring_extensions" => &mut app.config.ignoring_extensions,
        "ignoring_files" => &mut app.config.ignoring_files,
        "metric" => &mut app.config.metric,
        other => bail!("{other} is not a list-valued field"),
    };
    *target = value;
    app.config
        .origins
        .insert((*field).to_string(), Origin::Session);
    app.dirty.insert((*field).to_string());
    Ok(())
}

fn parse_into(config: &mut Config, field: &str, raw: &str) -> Result<()> {
    let trimmed = raw.trim();
    match field {
        "index_depth" => {
            config.index_depth =
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.parse::<u32>().with_context(|| {
                        format!("index_depth must be a number, not {trimmed:?}")
                    })?)
                };
        }
        "index_exclude"
        | "index_detect_as_module"
        | "ignoring_extensions"
        | "ignoring_files"
        | "metric" => {
            // Reject a malformed wildcard here rather than at the end of an
            // index build.
            let patterns = split_patterns(trimmed);
            validate_patterns(&patterns)?;
            match field {
                "index_exclude" => config.index_exclude = patterns,
                "index_detect_as_module" => config.index_detect_as_module = patterns,
                "ignoring_extensions" => config.ignoring_extensions = patterns,
                "ignoring_files" => config.ignoring_files = patterns,
                _ => config.metric = patterns,
            }
        }
        "select_date_query_result_is_multiple" => {
            config.select_date_query_result_is_multiple = DateMultiple::parse(trimmed)
                .with_context(|| {
                    format!("expected one of {}", DateMultiple::variants().join(", "))
                })?;
        }
        "select_date_query_result_is_none" => {
            config.select_date_query_result_is_none =
                DateNone::parse(trimmed).context("expected one of fast-forward, ff, rewind, rw")?;
        }
        "threads" => {
            config.threads = Threads::parse(trimmed).with_context(|| {
                format!("threads must be {:?} or a positive integer", Threads::AUTO)
            })?;
        }
        other => bail!("no such configuration field: {other}"),
    }
    Ok(())
}

/// (*4) Where a field's value came from, ready for the config list.
pub fn describe_origin(app: &App, field: &str) -> String {
    match app.config.origin_of(field) {
        Origin::Default => "default".to_string(),
        Origin::Args => "args".to_string(),
        Origin::Session => "unsaved".to_string(),
        Origin::UserConfig(_) => "~/.cellular".to_string(),
        Origin::ProjectConfig(path) => path
            .parent()
            .map(|dir| dir.display().to_string())
            .unwrap_or_else(|| path.display().to_string()),
    }
}

/// The same label, shortened relative to the project root.
pub fn describe_origin_short(app: &App, field: &str) -> String {
    match app.config.origin_of(field) {
        Origin::ProjectConfig(path) => {
            let dir = path.parent().unwrap_or(path);
            super::shorten_path(dir, &app.project_root)
        }
        _ => describe_origin(app, field),
    }
}

// ------------------------------------------------------------ completion --

/// The token the cursor sits in: its start, its text up to the cursor, and how
/// many whitespace-separated tokens precede it.
fn token_at(input: &str, cursor: usize) -> (usize, String, usize) {
    let before: String = input.chars().take(cursor).collect();
    if before.is_empty() || before.ends_with(char::is_whitespace) {
        return (cursor, String::new(), before.split_whitespace().count());
    }
    let tokens: Vec<&str> = before.split_whitespace().collect();
    let current = tokens.last().copied().unwrap_or_default();
    (
        cursor - current.chars().count(),
        current.to_string(),
        tokens.len() - 1,
    )
}

fn matching(items: &[(String, String)], prefix: &str) -> Vec<CompletionItem> {
    items
        .iter()
        .filter(|(label, _)| label.starts_with(prefix))
        .map(|(label, description)| CompletionItem {
            label: label.clone(),
            description: description.clone(),
        })
        .collect()
}

fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
    items
        .iter()
        .map(|(label, description)| (label.to_string(), description.to_string()))
        .collect()
}

/// Candidates for the token the cursor is in, or `None` when there are none.
pub fn complete(app: &App, input: &str, cursor: usize) -> Option<Completion> {
    let (start, prefix, index) = token_at(input, cursor);
    let words: Vec<&str> = input.split_whitespace().collect();
    let command = words.first().copied().unwrap_or_default();

    let candidates: Vec<CompletionItem> = if index == 0 {
        let items: Vec<(String, String)> = COMMANDS
            .iter()
            .map(|spec| (spec.name.to_string(), spec.summary.to_string()))
            .collect();
        matching(&items, &prefix)
    } else {
        match (command, index) {
            ("help", 1) => {
                let items: Vec<(String, String)> = COMMANDS
                    .iter()
                    .map(|spec| (spec.name.to_string(), spec.summary.to_string()))
                    .collect();
                matching(&items, &prefix)
            }
            ("cd", 1) | ("ls", 1) | ("open", 1) => {
                matching(&directory_candidates(app, &prefix), &prefix)
            }
            ("get", 1) | ("set", 1) => matching(&field_candidates(app), &prefix),
            ("set", 2) => {
                let field = words.get(1).copied().unwrap_or_default();
                matching(&value_candidates(field), &prefix)
            }
            ("save", 1) => matching(&pairs(SAVE_FLAGS), &prefix),
            ("index", 1) => matching(&pairs(INDEX_SUBCOMMANDS), &prefix),
            ("index", 2) if words.get(1) == Some(&"cancel") => {
                matching(&pairs(INDEX_CANCEL_TARGETS), &prefix)
            }
            ("viewer", 1) => matching(&pairs(VIEWER_SUBCOMMANDS), &prefix),
            _ => Vec::new(),
        }
    };

    if candidates.is_empty() {
        return None;
    }

    Some(Completion {
        items: candidates,
        selected: None,
        replace_from: start,
        anchor: if index == 0 {
            Anchor::LeftEdge
        } else {
            Anchor::Cursor
        },
    })
}

fn field_candidates(app: &App) -> Vec<(String, String)> {
    FIELDS
        .iter()
        .map(|field| {
            let value = app.config.display_value(field).unwrap_or_default();
            ((*field).to_string(), format!("current: {value}"))
        })
        .collect()
}

fn value_candidates(field: &str) -> Vec<(String, String)> {
    Config::value_candidates(field)
        .iter()
        .map(|value| ((*value).to_string(), format!("set {field} to {value}")))
        .collect()
}

/// Subdirectories that could complete a `cd` argument.
fn directory_candidates(app: &App, prefix: &str) -> Vec<(String, String)> {
    // Split the typed text into the part that names a directory to list and
    // the part that filters its entries.
    let (listed, _) = match prefix.rsplit_once('/') {
        Some((head, tail)) => (format!("{head}/"), tail.to_string()),
        None => (String::new(), prefix.to_string()),
    };
    let base = if listed.is_empty() {
        app.cwd.clone()
    } else {
        resolve_path(&app.cwd, &listed)
    };

    let Ok(entries) = std::fs::read_dir(&base) else {
        return Vec::new();
    };
    let mut candidates: Vec<(String, String)> = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            let label = format!("{listed}{name}/");
            let description = if entry.path().join(".git").exists() {
                "git repository".to_string()
            } else {
                "directory".to_string()
            };
            (label, description)
        })
        .collect();
    candidates.sort_by(|left, right| left.0.cmp(&right.0));
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn app(project_root: PathBuf, cwd: PathBuf) -> App {
        App::new(project_root, cwd, Config::default())
    }

    #[test]
    fn finds_the_token_under_the_cursor() {
        assert_eq!(token_at("he", 2), (0, "he".to_string(), 0));
        assert_eq!(token_at("set ", 4), (4, String::new(), 1));
        assert_eq!(token_at("set ind", 7), (4, "ind".to_string(), 1));
        assert_eq!(token_at("", 0), (0, String::new(), 0));
    }

    #[test]
    fn every_command_has_a_help_entry() {
        for command in COMMANDS {
            assert!(find(command.name).is_some());
            assert!(!command.summary.is_empty());
        }
    }

    #[test]
    fn ls_and_open_default_to_the_path_shown_in_the_header() {
        let project_root = std::env::temp_dir().join(format!(
            "cellular-command-test-{}",
            uuid::Uuid::new_v4()
        ));
        let nested_cwd = project_root.join("nested");
        std::fs::create_dir_all(&nested_cwd).expect("create test directories");

        let app = app(project_root.clone(), nested_cwd);
        assert_eq!(
            target_directory(&app, &[]).expect("default directory"),
            project_root.canonicalize().expect("canonical project root")
        );

        std::fs::remove_dir_all(app.project_root).expect("remove test directories");
    }

    #[test]
    fn ls_lists_directories_files_and_a_summary() {
        let project_root = std::env::temp_dir().join(format!(
            "cellular-command-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(project_root.join("directory")).expect("create test directory");
        std::fs::write(project_root.join("file.txt"), "cellular").expect("write test file");

        let mut app = app(project_root.clone(), project_root.clone());
        run_ls(&mut app, &[]).expect("list default directory");
        let output = app
            .log
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(output.contains("directory/"), "{output}");
        assert!(output.contains("file.txt"), "{output}");
        assert!(output.contains("1 directories, 1 files"), "{output}");

        std::fs::remove_dir_all(project_root).expect("remove test directory");
    }
}
