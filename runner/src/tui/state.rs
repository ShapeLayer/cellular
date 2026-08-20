//! Small pieces of terminal-interface state that outlive one session.
//!
//! Kept in `~/.cellular/state.json`, separate from `config.json`, so
//! dismissing a warning never rewrites a file the project tracks in git.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::config::profile_dir;

pub const STATE_FILE: &str = "state.json";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UiState {
    /// Keys of notices the user asked not to see again.
    #[serde(default)]
    pub dismissed_notices: BTreeSet<String>,
}

impl UiState {
    pub fn path() -> Result<PathBuf> {
        Ok(profile_dir()?.join(STATE_FILE))
    }

    pub fn load() -> Self {
        let Ok(path) = Self::path() else {
            return Self::default();
        };
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, serde_json::to_string_pretty(self)? + "\n")
            .with_context(|| format!("failed to write {}", path.display()))?;
        Ok(())
    }
}
