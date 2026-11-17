//! GUI-only local state (window size, last active page) that doesn't belong
//! in `stackcrate_core::config::Config`, which is shared with the CLI and
//! daemon and shouldn't need to know anything about window chrome.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowState {
    pub width: i32,
    pub height: i32,
    pub maximized: bool,
}

impl Default for WindowState {
    fn default() -> Self {
        Self {
            width: 900,
            height: 680,
            maximized: false,
        }
    }
}

fn path() -> PathBuf {
    stackcrate_core::config::Paths::cache_dir().join("gui-state.json")
}

impl WindowState {
    pub fn load() -> Self {
        std::fs::read_to_string(path())
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path(), text);
        }
    }
}
