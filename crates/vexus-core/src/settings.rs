//! User settings, persisted as JSON with atomic writes and defensive loading.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::model::DeletionPolicy;
use crate::util::write_atomic;

const FILE: &str = "settings.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Theme {
    #[default]
    Dark,
    Light,
    System,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub theme: Theme,
    /// A `#rrggbb` colour. Everything else about the palette is derived from it.
    pub accent: String,
    pub reduce_motion: bool,
    /// Show every category with its risk, instead of just the recommended ones.
    pub advanced: bool,
    /// Days Recovery keeps undoable removals.
    pub undo_days: u32,
    pub deletion_policy: DeletionPolicy,
    /// Age limit (hours) for rules that allow adjusting it, such as Temporary files.
    pub temp_min_age_hours: u32,
    pub close_to_tray: bool,
    /// Log every path touched (not just summaries).
    pub verbose_logging: bool,
    pub onboarded: bool,
    /// The user's last category selection (so it survives restarts).
    pub selected: Option<Vec<String>>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: Theme::Dark,
            accent: "#5ec8ff".to_string(),
            reduce_motion: false,
            advanced: false,
            undo_days: 7,
            deletion_policy: DeletionPolicy::Smart,
            temp_min_age_hours: 72,
            close_to_tray: false,
            verbose_logging: false,
            onboarded: false,
            selected: None,
        }
    }
}

impl Settings {
    /// Clamps values into their valid ranges and repairs anything malformed.
    pub fn sanitize(&mut self) {
        self.undo_days = self.undo_days.clamp(1, 60);
        self.temp_min_age_hours = self.temp_min_age_hours.min(24 * 90);
        if !is_hex_colour(&self.accent) {
            self.accent = Settings::default().accent;
        }
        self.accent.make_ascii_lowercase();
    }

    pub fn path(state_dir: &Path) -> PathBuf {
        state_dir.join(FILE)
    }

    /// Loads settings; a missing or damaged file yields defaults (the damaged file is kept
    /// aside as `settings.json.bad` so nothing is silently lost).
    pub fn load(state_dir: &Path) -> Settings {
        let path = Self::path(state_dir);
        let Ok(bytes) = std::fs::read(&path) else { return Settings::default() };
        match serde_json::from_slice::<Settings>(&bytes) {
            Ok(mut s) => {
                s.sanitize();
                s
            }
            Err(e) => {
                tracing::warn!("settings file is damaged ({e}); using defaults");
                let _ = std::fs::rename(&path, path.with_extension("json.bad"));
                Settings::default()
            }
        }
    }

    pub fn save(&self, state_dir: &Path) -> io::Result<()> {
        std::fs::create_dir_all(state_dir)?;
        let json = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        write_atomic(&Self::path(state_dir), &json)
    }
}

fn is_hex_colour(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 7 && b[0] == b'#' && b[1..].iter().all(u8::is_ascii_hexdigit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let s = Settings {
            theme: Theme::Light,
            accent: "#ff00aa".into(),
            undo_days: 14,
            onboarded: true,
            selected: Some(vec!["windows.logs".into()]),
            ..Settings::default()
        };
        s.save(dir.path()).unwrap();
        assert_eq!(Settings::load(dir.path()), s);
    }

    #[test]
    fn missing_file_gives_defaults_and_partial_files_fill_in() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Settings::load(dir.path()), Settings::default());
        std::fs::write(Settings::path(dir.path()), br#"{"theme":"light","undoDays":3}"#).unwrap();
        let s = Settings::load(dir.path());
        assert_eq!((s.theme, s.undo_days), (Theme::Light, 3));
        assert_eq!(s.accent, Settings::default().accent);
    }

    #[test]
    fn out_of_range_and_malformed_values_are_repaired() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            Settings::path(dir.path()),
            br##"{"undoDays":0,"tempMinAgeHours":99999999,"accent":"javascript:alert(1)"}"##,
        )
        .unwrap();
        let s = Settings::load(dir.path());
        assert_eq!(s.undo_days, 1);
        assert_eq!(s.temp_min_age_hours, 24 * 90);
        assert_eq!(
            s.accent,
            Settings::default().accent,
            "accent flows into CSS, so it must be a hex colour"
        );
    }

    #[test]
    fn damaged_files_are_kept_aside() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(Settings::path(dir.path()), b"{ not json").unwrap();
        assert_eq!(Settings::load(dir.path()), Settings::default());
        assert!(dir.path().join("settings.json.bad").exists());
    }
}
