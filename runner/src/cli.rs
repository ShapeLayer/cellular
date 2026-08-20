//! Command line arguments, and how they override the layered configuration.

use clap::{ArgAction, Parser};
use std::path::PathBuf;

use crate::config::{Config, DateMultiple, DateNone, Origin, Threads};
use crate::filters::split_patterns;

const AFTER_HELP: &str = "\
COMMITS accepts a comma separated list of:
  <hash>, a hash prefix, HEAD, HEAD^, HEAD~N, a branch name, a tag name,
  a range <older>..<newer> (exclusive of <older>, inclusive of <newer>),
  date:YYYY-MM-DD[-HH[-MM[-SS]]], all (every commit in the repository)

Examples:
  cellular -i 2 main,HEAD~10,v1.0
  cellular -i 2 'date:2026-08-17,main,date:2026-08-01-9-30'
  cellular -i 2 all
  cellular --export ~/project.cellexport
  cellular --terminal";

#[derive(Parser, Debug, Default)]
#[command(
    name = "cellular",
    version,
    about = "Build code composition indexes for a git project",
    after_help = AFTER_HELP
)]
pub struct Args {
    /// Commits to index, comma separated.
    #[arg(value_name = "COMMITS")]
    pub commits: Option<String>,

    /// Folder depth treated as one module.
    #[arg(short = 'i', long = "index-depth", value_name = "N")]
    pub index_depth: Option<u32>,

    /// Files or directories to leave out of the index.
    #[arg(short = 'e', long = "index-exclude", value_name = "PATTERNS", action = ArgAction::Append)]
    pub index_exclude: Vec<String>,

    /// Directories to treat as their own module, ignoring --index-depth.
    #[arg(short = 'd', long = "index-detect-as-module", value_name = "PATTERNS", action = ArgAction::Append)]
    pub index_detect_as_module: Vec<String>,

    /// File extensions to leave out of the index.
    #[arg(short = 'x', long = "ignoring-extensions", value_name = "PATTERNS", action = ArgAction::Append)]
    pub ignoring_extensions: Vec<String>,

    /// File names to leave out of the index.
    #[arg(short = 'f', long = "ignoring-files", value_name = "PATTERNS", action = ArgAction::Append)]
    pub ignoring_files: Vec<String>,

    /// Metrics to collect: lines, chars, languages.
    #[arg(short = 'm', long = "metric", value_name = "PATTERNS", action = ArgAction::Append)]
    pub metric: Vec<String>,

    /// Which commit to take when a date query matches several.
    #[arg(long = "select-date-query-result-is-multiple", value_name = "MODE",
          value_parser = parse_date_multiple)]
    pub select_date_query_result_is_multiple: Option<DateMultiple>,

    /// Which commit to take when a date query matches none.
    #[arg(long = "select-date-query-result-is-none", value_name = "MODE",
          value_parser = parse_date_none)]
    pub select_date_query_result_is_none: Option<DateNone>,

    /// Threads to measure commits with: a positive integer, or auto.
    #[arg(short = 'j', long = "threads", value_name = "N|auto",
          value_parser = parse_threads)]
    pub threads: Option<Threads>,

    /// Run against this directory instead of the current one.
    #[arg(short = 'C', long = "path", value_name = "PATH")]
    pub path: Option<PathBuf>,

    /// Start the terminal interface.
    #[arg(long)]
    pub terminal: bool,

    /// Check the stored index against the digests in INDEX and exit.
    #[arg(long)]
    pub verify: bool,

    /// List the snapshots the stored index holds and exit.
    #[arg(long)]
    pub list: bool,

    /// Package the stored index as a .cellexport file for the viewer and exit.
    #[arg(long, value_name = "PATH", num_args = 0..=1)]
    pub export: Option<Option<PathBuf>>,

    /// With --list, also print every module of every snapshot.
    #[arg(long)]
    pub modules: bool,

    /// Measure commits again even when a matching record is already stored.
    #[arg(long)]
    pub force: bool,
}

fn parse_threads(value: &str) -> Result<Threads, String> {
    Threads::parse(value)
        .ok_or_else(|| format!("expected {:?} or a positive integer", Threads::AUTO))
}

fn parse_date_multiple(value: &str) -> Result<DateMultiple, String> {
    DateMultiple::parse(value)
        .ok_or_else(|| format!("expected one of {}", DateMultiple::variants().join(", ")))
}

fn parse_date_none(value: &str) -> Result<DateNone, String> {
    DateNone::parse(value).ok_or_else(|| "expected one of fast-forward, ff, rewind, rw".to_string())
}

/// Flatten repeated occurrences, splitting `[a, b]` and `a,b` alike.
fn collect(values: &[String]) -> Vec<String> {
    values.iter().flat_map(|raw| split_patterns(raw)).collect()
}

impl Args {
    /// Apply the arguments the user actually passed on top of the file layers.
    pub fn apply_to(&self, config: &mut Config) {
        let mut set = |field: &str| {
            config.origins.insert(field.to_string(), Origin::Args);
        };

        if let Some(depth) = self.index_depth {
            config.index_depth = Some(depth);
            set("index_depth");
        }
        if !self.index_exclude.is_empty() {
            config.index_exclude = collect(&self.index_exclude);
            set("index_exclude");
        }
        if !self.index_detect_as_module.is_empty() {
            config.index_detect_as_module = collect(&self.index_detect_as_module);
            set("index_detect_as_module");
        }
        if !self.ignoring_extensions.is_empty() {
            config.ignoring_extensions = collect(&self.ignoring_extensions);
            set("ignoring_extensions");
        }
        if !self.ignoring_files.is_empty() {
            config.ignoring_files = collect(&self.ignoring_files);
            set("ignoring_files");
        }
        if !self.metric.is_empty() {
            config.metric = collect(&self.metric);
            set("metric");
        }
        if let Some(value) = self.select_date_query_result_is_multiple {
            config.select_date_query_result_is_multiple = value;
            set("select_date_query_result_is_multiple");
        }
        if let Some(value) = self.select_date_query_result_is_none {
            config.select_date_query_result_is_none = value;
            set("select_date_query_result_is_none");
        }
        if let Some(value) = self.threads {
            config.threads = value;
            set("threads");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn list_arguments_accept_both_notations() {
        let args = Args::parse_from(["cellular", "-x", "[.md, .txt]", "-x", ".json", "HEAD"]);
        let mut config = Config::default();
        args.apply_to(&mut config);
        assert_eq!(config.ignoring_extensions, vec![".md", ".txt", ".json"]);
        assert_eq!(config.origins["ignoring_extensions"], Origin::Args);
    }

    #[test]
    fn absent_arguments_do_not_override() {
        let args = Args::parse_from(["cellular", "HEAD"]);
        let mut config = Config::default();
        let before = config.ignoring_files.clone();
        args.apply_to(&mut config);
        assert_eq!(config.ignoring_files, before);
        assert_eq!(config.origins["ignoring_files"], Origin::Default);
    }

    #[test]
    fn threads_accepts_a_count_or_auto() {
        let args = Args::parse_from(["cellular", "-j", "4", "HEAD"]);
        assert_eq!(args.threads, Some(Threads::Fixed(4)));

        let mut config = Config::default();
        assert_eq!(config.threads, Threads::Auto);
        args.apply_to(&mut config);
        assert_eq!(config.threads, Threads::Fixed(4));
        assert_eq!(config.origins["threads"], Origin::Args);

        let args = Args::parse_from(["cellular", "--threads", "auto", "HEAD"]);
        assert_eq!(args.threads, Some(Threads::Auto));
        assert!(Args::try_parse_from(["cellular", "-j", "0", "HEAD"]).is_err());
    }

    #[test]
    fn date_selection_aliases_are_accepted() {
        let args = Args::parse_from(["cellular", "--select-date-query-result-is-none", "rw"]);
        assert_eq!(
            args.select_date_query_result_is_none,
            Some(DateNone::Rewind)
        );
    }
}
