//! Turning the `commits` argument into a list of commits to index.
//!
//! Accepted forms, comma separated:
//! hashes and hash prefixes, `HEAD` / `HEAD^` / `HEAD~N`, branch names, tag
//! names, ranges `a..b` (exclusive of `a`, inclusive of `b`) and date queries
//! `date:YYYY-MM-DD[-HH-MM[-SS]]`.

use anyhow::{Context, Result, anyhow, bail};
use chrono::{Local, NaiveDate, NaiveDateTime, TimeZone};
use gix::ObjectId;
use gix::bstr::BStr;
use std::collections::{HashMap, HashSet};

use crate::config::{Config, DateMultiple, DateNone};

/// Branch and tag names, keyed by the commit they point at. The viewer labels
/// its timeline with these.
pub type RefMap = HashMap<ObjectId, Vec<String>>;

/// Collect every local and remote reference, shortened for display.
pub fn ref_names(repo: &gix::Repository) -> Result<RefMap> {
    let mut map: RefMap = HashMap::new();
    for reference in repo.references()?.all()? {
        let Ok(mut reference) = reference else {
            continue;
        };
        let full = reference.name().as_bstr().to_string();
        let Ok(id) = reference.peel_to_id() else {
            continue;
        };
        let Ok(commit_id) = peel_to_commit_id(repo, id.detach()) else {
            continue;
        };
        let short = full
            .strip_prefix("refs/heads/")
            .or_else(|| full.strip_prefix("refs/tags/"))
            .or_else(|| full.strip_prefix("refs/remotes/"))
            .unwrap_or(&full)
            .to_string();
        let names = map.entry(commit_id).or_default();
        if !names.contains(&short) {
            names.push(short);
        }
    }
    for names in map.values_mut() {
        names.sort();
    }
    Ok(map)
}

/// A commit selected by one of the user's specs.
#[derive(Debug, Clone)]
pub struct ResolvedCommit {
    pub id: ObjectId,
    /// The spec the user typed that produced this commit.
    pub spec: String,
}

/// Half-open local-time interval `[start, end)` derived from a `date:` spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateRange {
    pub start: i64,
    pub end: i64,
}

/// Split the `commits` argument on commas, keeping the original text of each
/// spec for reporting.
pub fn split_specs(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|part| part.trim().to_string())
        .filter(|part| !part.is_empty())
        .collect()
}

pub fn resolve_all(
    repo: &gix::Repository,
    specs: &[String],
    config: &Config,
) -> Result<Vec<ResolvedCommit>> {
    let mut resolved = Vec::new();
    let mut seen = HashSet::new();
    for spec in specs {
        for commit in resolve_one(repo, spec, config)? {
            if seen.insert(commit.id) {
                resolved.push(commit);
            }
        }
    }
    Ok(resolved)
}

fn resolve_one(repo: &gix::Repository, spec: &str, config: &Config) -> Result<Vec<ResolvedCommit>> {
    if spec.eq_ignore_ascii_case("all") {
        let ids = all_commits(repo)?;
        return Ok(ids
            .into_iter()
            .map(|id| ResolvedCommit {
                id,
                spec: spec.to_string(),
            })
            .collect());
    }

    if let Some(date_text) = spec.strip_prefix("date:") {
        let range = parse_date_range(date_text)?;
        let id = resolve_date(repo, range, config)
            .with_context(|| format!("failed to resolve {spec:?}"))?;
        return Ok(vec![ResolvedCommit {
            id,
            spec: spec.to_string(),
        }]);
    }

    if let Some((from, to)) = split_range(spec) {
        let ids = resolve_range(repo, from, to)
            .with_context(|| format!("failed to resolve range {spec:?}"))?;
        return Ok(ids
            .into_iter()
            .map(|id| ResolvedCommit {
                id,
                spec: spec.to_string(),
            })
            .collect());
    }

    let id = resolve_rev(repo, spec)?;
    Ok(vec![ResolvedCommit {
        id,
        spec: spec.to_string(),
    }])
}

/// `a..b`, but not `a...b`, which git gives a different meaning.
fn split_range(spec: &str) -> Option<(&str, &str)> {
    if spec.contains("...") {
        return None;
    }
    let (from, to) = spec.split_once("..")?;
    if from.is_empty() || to.is_empty() {
        return None;
    }
    Some((from, to))
}

/// Resolve a single revision through git's own revision syntax, which already
/// covers hashes, prefixes, `HEAD~N`, branches and tags.
pub fn resolve_rev(repo: &gix::Repository, spec: &str) -> Result<ObjectId> {
    let id = repo
        .rev_parse_single(BStr::new(spec.as_bytes()))
        .with_context(|| format!("no commit matches {spec:?}"))?;
    peel_to_commit_id(repo, id.detach())
        .with_context(|| format!("{spec:?} does not point at a commit"))
}

fn peel_to_commit_id(repo: &gix::Repository, id: ObjectId) -> Result<ObjectId> {
    let object = repo.find_object(id)?;
    let commit = object.peel_to_kind(gix::object::Kind::Commit)?;
    Ok(commit.id)
}

/// Commits reachable from `to` but not from `from`, oldest first.
fn resolve_range(repo: &gix::Repository, from: &str, to: &str) -> Result<Vec<ObjectId>> {
    let from_id = resolve_rev(repo, from)?;
    let to_id = resolve_rev(repo, to)?;

    let mut hidden = HashSet::new();
    for info in repo.rev_walk([from_id]).all()? {
        hidden.insert(info?.id);
    }

    let mut ids = Vec::new();
    for info in repo.rev_walk([to_id]).all()? {
        let id = info?.id;
        if !hidden.contains(&id) {
            ids.push(id);
        }
    }
    if ids.is_empty() {
        bail!("{from}..{to} selects no commits; is {from} really the older commit?");
    }

    // The walk is topological; present the commits oldest first instead.
    let mut dated = Vec::with_capacity(ids.len());
    for id in ids {
        let time = repo.find_object(id)?.into_commit().time()?.seconds;
        dated.push((time, id));
    }
    dated.sort_by_key(|(time, id)| (*time, *id));
    Ok(dated.into_iter().map(|(_, id)| id).collect())
}

/// Every commit in the repository, oldest first.
fn all_commits(repo: &gix::Repository) -> Result<Vec<ObjectId>> {
    let mut commits = walk_all_commits(repo)?;
    if commits.is_empty() {
        bail!("the repository has no commits to index");
    }
    commits.sort_by_key(|(time, id)| (*time, *id));
    Ok(commits.into_iter().map(|(_, id)| id).collect())
}

/// Parse `YYYY-MM-DD`, `YYYY-MM-DD-HH`, `YYYY-MM-DD-HH-MM` and
/// `YYYY-MM-DD-HH-MM-SS`, with or without zero padding. The interval covers
/// the precision that was given, in the local time zone.
pub fn parse_date_range(text: &str) -> Result<DateRange> {
    let parts: Vec<&str> = text.trim().split('-').filter(|p| !p.is_empty()).collect();
    let number = |index: usize| -> Result<u32> {
        parts[index]
            .parse::<u32>()
            .map_err(|_| anyhow!("{text:?} is not a valid date"))
    };

    if parts.len() < 3 || parts.len() > 6 {
        bail!("{text:?} is not a valid date; use YYYY-MM-DD[-HH[-MM[-SS]]]");
    }

    let year = parts[0]
        .parse::<i32>()
        .map_err(|_| anyhow!("{text:?} is not a valid date"))?;
    let date = NaiveDate::from_ymd_opt(year, number(1)?, number(2)?)
        .ok_or_else(|| anyhow!("{text:?} is not a valid calendar date"))?;

    let (start, span_seconds) = match parts.len() {
        3 => (date.and_hms_opt(0, 0, 0), 24 * 60 * 60),
        4 => (date.and_hms_opt(number(3)?, 0, 0), 60 * 60),
        5 => (date.and_hms_opt(number(3)?, number(4)?, 0), 60),
        _ => (date.and_hms_opt(number(3)?, number(4)?, number(5)?), 1),
    };
    let start: NaiveDateTime =
        start.ok_or_else(|| anyhow!("{text:?} is not a valid time of day"))?;
    // Add the span in local time rather than to the timestamp, so a day that
    // is 23 or 25 hours long across a clock change still covers exactly it.
    let end = start + chrono::TimeDelta::seconds(span_seconds);

    let to_timestamp = |naive: &NaiveDateTime| -> Result<i64> {
        Ok(Local
            .from_local_datetime(naive)
            .earliest()
            .ok_or_else(|| anyhow!("{text:?} does not exist in the local time zone"))?
            .timestamp())
    };

    Ok(DateRange {
        start: to_timestamp(&start)?,
        end: to_timestamp(&end)?,
    })
}

/// Pick one commit for a date query, following the two selection options.
fn resolve_date(repo: &gix::Repository, range: DateRange, config: &Config) -> Result<ObjectId> {
    let mut inside: Vec<(i64, ObjectId)> = Vec::new();
    let mut before: Option<(i64, ObjectId)> = None;
    let mut after: Option<(i64, ObjectId)> = None;

    for (time, id) in walk_all_commits(repo)? {
        if time >= range.start && time < range.end {
            inside.push((time, id));
        } else if time < range.start {
            // The newest commit before the interval.
            if before.is_none_or(|(best, _)| time > best) {
                before = Some((time, id));
            }
        } else if after.is_none_or(|(best, _)| time < best) {
            // The oldest commit after the interval.
            after = Some((time, id));
        }
    }

    if !inside.is_empty() {
        inside.sort_by_key(|(time, id)| (*time, *id));
        let picked = match config.select_date_query_result_is_multiple {
            DateMultiple::Latest => inside.last(),
            DateMultiple::Oldest => inside.first(),
            DateMultiple::Median => inside.get(inside.len() / 2),
        };
        return Ok(picked.expect("inside is not empty").1);
    }

    let fallback = match config.select_date_query_result_is_none {
        DateNone::FastForward => after.or(before),
        DateNone::Rewind => before.or(after),
    };
    fallback
        .map(|(_, id)| id)
        .ok_or_else(|| anyhow!("the repository has no commits to select from"))
}

/// Every commit reachable from any local or remote reference, with its
/// committer time.
fn walk_all_commits(repo: &gix::Repository) -> Result<Vec<(i64, ObjectId)>> {
    let mut tips: Vec<ObjectId> = Vec::new();
    for reference in repo.references()?.all()? {
        let Ok(mut reference) = reference else {
            continue;
        };
        let Ok(id) = reference.peel_to_id() else {
            continue;
        };
        if let Ok(commit_id) = peel_to_commit_id(repo, id.detach()) {
            tips.push(commit_id);
        }
    }
    if let Ok(head) = repo.head_id() {
        tips.push(head.detach());
    }
    if tips.is_empty() {
        return Ok(Vec::new());
    }

    let mut commits = Vec::new();
    let mut seen = HashSet::new();
    for info in repo.rev_walk(tips).all()? {
        let info = info?;
        if !seen.insert(info.id) {
            continue;
        }
        let commit = repo.find_object(info.id)?.into_commit();
        commits.push((commit.time()?.seconds, info.id));
    }
    Ok(commits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_specs() {
        assert_eq!(
            split_specs("main, HEAD~10 ,v1.0"),
            vec!["main", "HEAD~10", "v1.0"]
        );
    }

    #[test]
    fn detects_ranges() {
        assert_eq!(split_range("abc..def"), Some(("abc", "def")));
        assert_eq!(split_range("abc...def"), None);
        assert_eq!(split_range("HEAD~2"), None);
    }

    #[test]
    fn date_precision_sets_the_interval() {
        let day = parse_date_range("2026-08-17").unwrap();
        assert_eq!(day.end - day.start, 24 * 60 * 60);
        let minute = parse_date_range("2026-8-1-9-30").unwrap();
        assert_eq!(minute.end - minute.start, 60);
        let second = parse_date_range("2026-08-01-09-30-15").unwrap();
        assert_eq!(second.end - second.start, 1);
        assert_eq!(second.start - minute.start, 15);
    }

    #[test]
    fn rejects_malformed_dates() {
        assert!(parse_date_range("2026-13-01").is_err());
        assert!(parse_date_range("2026-08").is_err());
    }
}
