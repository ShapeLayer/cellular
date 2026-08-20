//! Cellular runner: the index builder.

mod builder;
mod cli;
mod commits;
mod config;
mod disk;
mod export;
mod filters;
mod gitignore;
mod index;
mod lang;
mod model;
mod scan;
mod tui;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, FixedOffset};
use clap::Parser;
use std::path::PathBuf;

use crate::builder::Progress;
use crate::config::{Config, FIELDS, Origin};
use crate::index::store::IndexStore;

/// Prints progress lines as they happen.
struct StdoutProgress;

impl Progress for StdoutProgress {
    fn note(&mut self, message: &str) {
        println!("  {message}");
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args = cli::Args::parse();

    let start_dir = match &args.path {
        Some(path) => path.clone(),
        None => std::env::current_dir()?,
    };
    let repo = gix::discover(&start_dir)
        .with_context(|| format!("{} is not inside a git repository", start_dir.display()))?;
    let project_root = project_root_of(&repo)?;

    let mut config = config::load_layered(&project_root)?;
    args.apply_to(&mut config);

    if args.terminal {
        // `cd` resolves against this, so start from a canonical path.
        let cwd = start_dir.canonicalize().unwrap_or(start_dir);
        return tui::run(project_root, cwd, config);
    }

    for warning in gitignore::check(&project_root).warnings() {
        eprintln!("warning: {warning}");
    }

    if args.verify || args.list || args.export.is_some() {
        let Some(store) = IndexStore::locate(&project_root, false)? else {
            println!("no index has been built for {}", project_root.display());
            return Ok(());
        };
        if let Some(destination) = &args.export {
            let path = destination
                .clone()
                .unwrap_or_else(|| export::default_path(&project_root));
            let report = export::write(&store, &path)?;
            println!(
                "exported {} snapshot(s) in {} blob file(s) to {} ({} bytes)",
                report.record_count,
                report.blob_count,
                report.path.display(),
                report.bytes
            );
            return Ok(());
        }
        return if args.verify {
            report_verification(&store)
        } else {
            list_snapshots(&store, args.modules)
        };
    }

    let Some(raw_specs) = args.commits.as_deref() else {
        bail!("no commits were given; pass commit specs, --list, --verify or --terminal");
    };
    let specs = commits::split_specs(raw_specs);
    if specs.is_empty() {
        bail!("no commits were given");
    }

    if config.index_depth.is_none() {
        bail!("index_depth is not set; pass --index-depth or set it in config.json");
    }
    // Recommended, per the requirements: every project pins its own depth.
    if !matches!(config.origin_of("index_depth"), Origin::ProjectConfig(_)) {
        eprintln!(
            "warning: index_depth has no project-level value in {}",
            config::project_config_path(&project_root).display()
        );
    }

    let store = IndexStore::locate_for_writing(&project_root)?;
    println!("project: {}", project_root.display());
    println!("index:   {}", store.root.display());
    print_config(&config);

    let outcome = builder::build(
        &repo,
        &store,
        &config,
        &specs,
        args.force,
        None,
        &mut StdoutProgress,
    )?;

    if outcome.discarded_damaged_index {
        eprintln!("warning: the previous index was damaged and has been rewritten");
    }
    if outcome.low_disk_stop {
        eprintln!(
            "warning: the build stopped early to stay clear of filling {}; \
             the commits it finished are stored",
            store.root.display()
        );
    } else if outcome.cancelled {
        eprintln!("warning: the build was cancelled; the commits it finished are stored");
    }
    if let Some(workers) = outcome.throttled_workers {
        eprintln!("warning: free space ran low, so the build finished on {workers} thread(s)");
    }

    println!("snapshots:");
    for item in &outcome.built {
        let totals = item.snapshot.totals();
        println!(
            "  {}  {}  {:<16}  {} modules  {} lines  {} chars{}",
            item.snapshot.short_oid(),
            format_time(item.snapshot.commit_time, item.snapshot.commit_tz_offset),
            item.spec,
            item.snapshot.modules.len(),
            totals.lines,
            totals.chars,
            if item.reused { "  (reused)" } else { "" }
        );
    }

    let measured = outcome.built.iter().filter(|item| !item.reused).count();
    let reused = outcome.built.len() - measured;
    println!(
        "wrote {} record(s) across {} blob file(s), {} bytes total",
        outcome.write.record_count, outcome.write.blob_count, outcome.write.total_bytes
    );
    println!(
        "  {measured} measured, {reused} reused, {} untouched record(s) kept",
        outcome.kept_records
    );
    if outcome.flushes > 1 {
        println!(
            "  the index was written {} times as work progressed",
            outcome.flushes
        );
    }
    if outcome.write.oversized_records > 0 {
        eprintln!(
            "warning: {} record(s) exceed the 10 MiB blob limit on their own; \
             consider a smaller index_depth",
            outcome.write.oversized_records
        );
    }
    Ok(())
}

/// The working tree root, or the directory holding a bare repository.
fn project_root_of(repo: &gix::Repository) -> Result<PathBuf> {
    if let Some(workdir) = repo.workdir() {
        return Ok(workdir.to_path_buf());
    }
    Ok(repo.git_dir().to_path_buf())
}

fn print_config(config: &Config) {
    println!("config:");
    for field in FIELDS {
        let value = config.display_value(field).unwrap_or_default();
        let origin = match config.origin_of(field) {
            Origin::Default => "default".to_string(),
            Origin::Args => "args".to_string(),
            Origin::Session => "unsaved".to_string(),
            Origin::UserConfig(path) | Origin::ProjectConfig(path) => path.display().to_string(),
        };
        println!("  {field}: {value} ({origin})");
    }
}

fn report_verification(store: &IndexStore) -> Result<()> {
    let report = store.verify()?;
    println!("index: {}", store.root.display());
    if report.is_healthy() {
        println!(
            "ok: {} blob file(s), {} snapshot record(s)",
            report.blob_count, report.record_count
        );
        return Ok(());
    }
    println!("the stored index may be damaged:");
    for problem in &report.problems {
        println!("  - {problem}");
    }
    println!("rebuild it by running cellular again with the commit specs you need");
    std::process::exit(2);
}

fn list_snapshots(store: &IndexStore, with_modules: bool) -> Result<()> {
    if !store.exists() {
        println!("no index has been built at {}", store.root.display());
        return Ok(());
    }
    let snapshots = store.load_snapshots()?;
    println!(
        "index: {} ({} snapshot(s))",
        store.root.display(),
        snapshots.len()
    );
    for snapshot in &snapshots {
        let totals = snapshot.totals();
        let labels = if snapshot.refs.is_empty() {
            String::new()
        } else {
            format!(" ({})", snapshot.refs.join(", "))
        };
        println!(
            "  {}  {}  depth {}  {} modules  {} files  {} lines  {} chars  {}{}",
            snapshot.short_oid(),
            format_time(snapshot.commit_time, snapshot.commit_tz_offset),
            snapshot.index_depth,
            snapshot.modules.len(),
            totals.files,
            totals.lines,
            totals.chars,
            snapshot.summary,
            labels
        );
        if with_modules {
            let parents = snapshot.parents_hex();
            let parents: Vec<String> = parents
                .iter()
                .map(|oid| oid.chars().take(10).collect())
                .collect();
            println!(
                "      author {}  parents [{}]",
                snapshot.author,
                parents.join(", ")
            );
        }
        if !with_modules {
            continue;
        }
        for module in &snapshot.modules {
            let languages: Vec<String> = module
                .languages
                .iter()
                .map(|(name, counts)| format!("{name} {}", counts.lines))
                .collect();
            println!(
                "      {:<28} {:>4} files {:>7} lines {:>8} chars   {}",
                module.path,
                module.totals.files,
                module.totals.lines,
                module.totals.chars,
                languages.join(", ")
            );
        }
    }
    Ok(())
}

fn format_time(seconds: i64, offset: i32) -> String {
    let zone = FixedOffset::east_opt(offset).unwrap_or_else(|| FixedOffset::east_opt(0).unwrap());
    DateTime::from_timestamp(seconds, 0)
        .map(|time| {
            time.with_timezone(&zone)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "unknown time".to_string())
}
