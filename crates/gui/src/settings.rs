//! The app's own settings, in `gui.json` next to the profiles.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Files open when the app last closed, reopened at start.
    pub open_files: Vec<PathBuf>,
    /// The profile new tabs use.
    pub last_profile: Option<String>,
    /// Rows kept per result.
    pub max_rows: usize,
    pub theme: Theme,
    pub editor_font_size: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            open_files: vec![],
            last_profile: None,
            max_rows: 1000,
            theme: Theme::System,
            editor_font_size: 14.0,
        }
    }
}

impl Settings {
    fn path(dir: &Path) -> PathBuf {
        dir.join("gui.json")
    }

    pub fn load(dir: &Path) -> Self {
        std::fs::read(Self::path(dir))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let json = serde_json::to_vec_pretty(self).expect("settings serialise");
        let tmp = Self::path(dir).with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(tmp, Self::path(dir))
    }
}
