//! Wildcard matching for the exclude/ignore/detect options.
//!
//! Every pattern list accepts either a JSON array, a comma separated list or a
//! bracketed comma separated list (`[a, b]`), so a value typed on the command
//! line and a value read from `config.json` behave identically.
//!
//! The three matchers differ in how much of a path a pattern is tried against,
//! because the options mean different things:
//!
//! * [`SubtreeMatcher`] (`index_exclude`) drops a path when the pattern matches
//!   the whole path or *any* component, so `.git*` excludes a nested
//!   `vendor/.github` as well as a top level one.
//! * [`DirectoryMatcher`] (`index_detect_as_module`) matches only the whole
//!   path or the directory's own name, so naming `src` promotes `src` alone and
//!   leaves `src/components` folded into it.
//! * [`NameMatcher`] (`ignoring_files`, `ignoring_extensions`) matches a single
//!   name, where separators are not special.

use anyhow::{Context, Result};
use globset::{Glob, GlobBuilder, GlobSet, GlobSetBuilder};

/// Split a CLI value into pattern items: `"[a, b]"`, `"a,b"` and `"a"` all work.
pub fn split_patterns(raw: &str) -> Vec<String> {
    let trimmed = raw.trim();
    let inner = trimmed
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(trimmed);
    inner
        .split(',')
        .map(|part| part.trim().trim_matches('"').trim_matches('\'').to_string())
        .filter(|part| !part.is_empty())
        .collect()
}

/// Build a matcher. With `literal_separator`, `*` does not cross `/`.
fn build_set(patterns: &[String], literal_separator: bool) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        let normalized = pattern.trim_end_matches('/');
        if normalized.is_empty() {
            continue;
        }
        let glob = GlobBuilder::new(normalized)
            .literal_separator(literal_separator)
            .build()
            .with_context(|| format!("invalid wildcard pattern {pattern:?}"))?;
        builder.add(glob);
    }
    builder
        .build()
        .context("failed to build the pattern matcher")
}

fn all_blank(patterns: &[String]) -> bool {
    patterns.iter().all(|pattern| pattern.trim().is_empty())
}

fn basename(rel_path: &str) -> &str {
    rel_path.rsplit_once('/').map_or(rel_path, |(_, name)| name)
}

/// Drops a path and everything under it. Used for `index_exclude`.
#[derive(Debug, Clone)]
pub struct SubtreeMatcher {
    set: GlobSet,
    empty: bool,
}

impl SubtreeMatcher {
    pub fn new(patterns: &[String]) -> Result<Self> {
        Ok(SubtreeMatcher {
            set: build_set(patterns, true)?,
            empty: all_blank(patterns),
        })
    }

    /// `rel_path` is a repository-relative path with `/` separators.
    pub fn matches(&self, rel_path: &str) -> bool {
        if self.empty {
            return false;
        }
        self.set.is_match(rel_path)
            || rel_path
                .split('/')
                .any(|component| self.set.is_match(component))
    }
}

/// Selects individual directories. Used for `index_detect_as_module`, where
/// matching an ancestor's name must not promote its children too.
#[derive(Debug, Clone)]
pub struct DirectoryMatcher {
    set: GlobSet,
    empty: bool,
}

impl DirectoryMatcher {
    pub fn new(patterns: &[String]) -> Result<Self> {
        Ok(DirectoryMatcher {
            set: build_set(patterns, true)?,
            empty: all_blank(patterns),
        })
    }

    pub fn matches(&self, rel_path: &str) -> bool {
        if self.empty {
            return false;
        }
        self.set.is_match(rel_path) || self.set.is_match(basename(rel_path))
    }
}

/// Matches a single name (a file name, an extension, a metric name) against a
/// pattern list. Separators are not special here.
#[derive(Debug, Clone)]
pub struct NameMatcher {
    set: GlobSet,
    empty: bool,
}

impl NameMatcher {
    pub fn new(patterns: &[String]) -> Result<Self> {
        Ok(NameMatcher {
            set: build_set(patterns, false)?,
            empty: all_blank(patterns),
        })
    }

    pub fn matches(&self, name: &str) -> bool {
        !self.empty && self.set.is_match(name)
    }
}

/// Check that every pattern compiles, so a bad one is reported where it is
/// entered rather than at the end of an index build.
pub fn validate_patterns(patterns: &[String]) -> Result<()> {
    build_set(patterns, true)?;
    Ok(())
}

/// One-shot glob test used for small lists such as `metric`.
pub fn any_glob_matches(patterns: &[String], candidate: &str) -> bool {
    patterns.iter().any(|pattern| {
        Glob::new(pattern)
            .map(|glob| glob.compile_matcher().is_match(candidate))
            .unwrap_or_else(|_| pattern == candidate)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_bracketed_and_plain_lists() {
        assert_eq!(split_patterns("[.md, .txt]"), vec![".md", ".txt"]);
        assert_eq!(split_patterns(".md,.txt"), vec![".md", ".txt"]);
        assert_eq!(split_patterns("  .md  "), vec![".md"]);
        assert!(split_patterns("[]").is_empty());
    }

    #[test]
    fn excludes_match_any_component() {
        let matcher = SubtreeMatcher::new(&[".git*".to_string()]).unwrap();
        assert!(matcher.matches(".git"));
        assert!(matcher.matches("vendor/.github/workflows"));
        assert!(!matcher.matches("src/git_helper.rs"));
    }

    #[test]
    fn excludes_respect_separators() {
        let matcher = SubtreeMatcher::new(&["src/*".to_string()]).unwrap();
        assert!(matcher.matches("src/main"));
        assert!(!matcher.matches("src/deep/main"));
    }

    #[test]
    fn module_detection_does_not_promote_children() {
        let matcher = DirectoryMatcher::new(&["src".to_string()]).unwrap();
        assert!(matcher.matches("src"));
        // The bug this guards against: `src/components` used to match because
        // one of its components is `src`, splitting the module apart.
        assert!(!matcher.matches("src/components"));
        assert!(!matcher.matches("src/components/foo"));
    }

    #[test]
    fn module_detection_matches_full_paths_and_names() {
        let matcher =
            DirectoryMatcher::new(&["src/components/foo".to_string(), "vendor".to_string()])
                .unwrap();
        assert!(matcher.matches("src/components/foo"));
        assert!(!matcher.matches("src/components"));
        // A bare name matches a directory of that name at any depth.
        assert!(matcher.matches("vendor"));
        assert!(matcher.matches("packages/app/vendor"));
        assert!(!matcher.matches("packages/vendor/inner"));
    }

    #[test]
    fn malformed_patterns_are_rejected() {
        assert!(validate_patterns(&["src/*".to_string()]).is_ok());
        let error = validate_patterns(&["[unclosed".to_string()]).unwrap_err();
        assert!(error.to_string().contains("[unclosed"), "{error}");
    }

    #[test]
    fn name_matcher_ignores_separators() {
        let matcher = NameMatcher::new(&["*.md".to_string()]).unwrap();
        assert!(matcher.matches("README.md"));
    }
}
