//! The `.gitignore` checks the runner reports on.
//!
//! Index data lives under `~/.cellular/index/`, so a project's `.cellular`
//! holds nothing but `config.json`, which should stay tracked. A project that
//! ignores `.cellular` therefore needs a `!.cellular/config.json` negation
//! alongside it.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct GitignoreStatus {
    pub path: PathBuf,
    /// False when the project has no `.gitignore` at all.
    #[allow(dead_code)]
    pub exists: bool,
    pub ignores_cellular: bool,
    pub keeps_config: bool,
}

pub fn check(project_root: &Path) -> GitignoreStatus {
    let path = project_root.join(".gitignore");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return GitignoreStatus {
            path,
            exists: false,
            ignores_cellular: false,
            keeps_config: false,
        };
    };

    let mut ignores_cellular = false;
    let mut keeps_config = false;
    for line in text.lines() {
        let entry = line.trim();
        if entry.is_empty() || entry.starts_with('#') {
            continue;
        }
        let normalized = entry.trim_start_matches('/').trim_end_matches('/');
        if normalized == ".cellular" {
            ignores_cellular = true;
        }
        if normalized == "!.cellular/config.json" {
            keeps_config = true;
        }
    }

    GitignoreStatus {
        path,
        exists: true,
        ignores_cellular,
        keeps_config,
    }
}

impl GitignoreStatus {
    /// Messages to show the user, empty when nothing is wrong.
    pub fn warnings(&self) -> Vec<String> {
        let mut warnings = Vec::new();
        if self.ignores_cellular && !self.keeps_config {
            warnings.push(format!(
                "{} ignores `.cellular` without a `!.cellular/config.json` entry; \
                 the project config should stay tracked",
                self.path.display()
            ));
        }
        warnings
    }

    /// The lines that would fix the warnings above, offered by the terminal
    /// interface when it asks to amend `.gitignore`.
    #[allow(dead_code)]
    pub fn suggested_lines(&self) -> Vec<&'static str> {
        let mut lines = Vec::new();
        if self.ignores_cellular && !self.keeps_config {
            lines.push("!.cellular/config.json");
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(ignores_cellular: bool, keeps_config: bool) -> GitignoreStatus {
        GitignoreStatus {
            path: PathBuf::from(".gitignore"),
            exists: true,
            ignores_cellular,
            keeps_config,
        }
    }

    #[test]
    fn only_an_ignored_config_is_worth_reporting() {
        // The project config would be left out of version control.
        assert_eq!(status(true, false).warnings().len(), 1);
        // Every other combination leaves config.json tracked.
        assert!(status(true, true).warnings().is_empty());
        assert!(status(false, false).warnings().is_empty());
        assert!(status(false, true).warnings().is_empty());
    }
}
