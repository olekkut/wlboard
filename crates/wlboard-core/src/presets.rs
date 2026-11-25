//! JSON User Presets manager (`~/.config/wlboard/presets/*.json`).

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::config::AppConfig;
use crate::theme::AppearanceSettings;

#[derive(Debug, Error)]
pub enum PresetError {
    #[error("preset name cannot be empty")]
    EmptyName,
    #[error("preset '{0}' not found")]
    NotFound(String),
    #[error("I/O error accessing preset: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid preset JSON: {0}")]
    Json(#[from] serde_json::Error),
}

/// A named user-saved appearance preset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserPreset {
    pub name: String,
    pub appearance: AppearanceSettings,
}

/// Manages CRUD operations for user appearance presets on disk.
#[derive(Debug, Clone)]
pub struct PresetStore {
    dir: PathBuf,
}

impl PresetStore {
    #[must_use]
    pub fn default_store() -> Self {
        Self {
            dir: AppConfig::presets_dir(),
        }
    }

    #[must_use]
    pub fn with_dir(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Convert a display name into a safe, portable lowercase ASCII slug without path separators.
    #[must_use]
    pub fn sanitize_slug(name: &str) -> String {
        let mut slug = String::with_capacity(name.len());
        let mut prev_dash = false;
        for ch in name.trim().chars() {
            if ch.is_ascii_alphanumeric() {
                slug.push(ch.to_ascii_lowercase());
                prev_dash = false;
            } else if (ch.is_whitespace() || ch == '-' || ch == '_')
                && !prev_dash
                && !slug.is_empty()
            {
                slug.push('-');
                prev_dash = true;
            }
        }
        slug.trim_end_matches('-').to_owned()
    }

    fn path_for_name(&self, name: &str) -> Result<PathBuf, PresetError> {
        let slug = Self::sanitize_slug(name);
        if slug.is_empty() {
            return Err(PresetError::EmptyName);
        }
        Ok(self.dir.join(format!("{slug}.json")))
    }

    /// List all saved user presets sorted alphabetically by display name.
    pub fn list(&self) -> Result<Vec<UserPreset>, PresetError> {
        if !self.dir.exists() {
            return Ok(Vec::new());
        }
        let mut items = Vec::new();
        for entry in fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json")
                && let Ok(raw) = fs::read_to_string(&path)
                && let Ok(mut preset) = serde_json::from_str::<UserPreset>(&raw)
            {
                preset.appearance.clamp();
                items.push(preset);
            }
        }
        items.sort_by_key(|a| a.name.to_lowercase());
        Ok(items)
    }

    /// Save (create or overwrite) a user preset atomically.
    pub fn save(
        &self,
        name: &str,
        appearance: &AppearanceSettings,
    ) -> Result<UserPreset, PresetError> {
        let clean_name = name.trim();
        if clean_name.is_empty() {
            return Err(PresetError::EmptyName);
        }
        let path = self.path_for_name(clean_name)?;
        fs::create_dir_all(&self.dir)?;

        let mut clamped = appearance.clone();
        clamped.clamp();
        let preset = UserPreset {
            name: clean_name.to_owned(),
            appearance: clamped,
        };
        let serialized = serde_json::to_string_pretty(&preset)?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, serialized)?;
        fs::rename(&tmp, &path)?;
        Ok(preset)
    }

    /// Load a single preset by display name.
    pub fn load(&self, name: &str) -> Result<UserPreset, PresetError> {
        let path = self.path_for_name(name)?;
        if !path.exists() {
            return Err(PresetError::NotFound(name.to_owned()));
        }
        let raw = fs::read_to_string(&path)?;
        let mut preset: UserPreset = serde_json::from_str(&raw)?;
        preset.appearance.clamp();
        Ok(preset)
    }

    /// Delete a saved preset by display name.
    pub fn delete(&self, name: &str) -> Result<(), PresetError> {
        let path = self.path_for_name(name)?;
        if !path.exists() {
            return Err(PresetError::NotFound(name.to_owned()));
        }
        fs::remove_file(&path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Color;

    #[test]
    fn sanitizes_unsafe_preset_names() {
        assert_eq!(PresetStore::sanitize_slug("Midnight Cyan"), "midnight-cyan");
        assert_eq!(PresetStore::sanitize_slug("../../etc/passwd"), "etcpasswd");
        assert_eq!(
            PresetStore::sanitize_slug("  TV   High_Contrast! "),
            "tv-high-contrast"
        );
        assert_eq!(PresetStore::sanitize_slug("!!!"), "");
    }

    #[test]
    fn preset_crud_round_trip() {
        let dir = std::env::temp_dir().join(format!("wlboard_presets_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let store = PresetStore::with_dir(&dir);

        assert!(store.list().expect("empty list").is_empty());

        let mut app = AppearanceSettings::default();
        app.colors.window_bg = Color::rgb(0x0D, 0x13, 0x24);
        app.colors.accent = Color::rgb(0x0E, 0xA5, 0xE9);
        app.colors.window_outline = Color::rgb(0x38, 0xBD, 0xF8);

        store.save("Midnight Cyan", &app).expect("save preset");
        let list = store.list().expect("list presets");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "Midnight Cyan");

        let loaded = store.load("Midnight Cyan").expect("load preset");
        assert_eq!(
            loaded.appearance.colors.accent,
            Color::rgb(0x0E, 0xA5, 0xE9)
        );

        store.delete("Midnight Cyan").expect("delete preset");
        assert!(store.list().expect("empty after delete").is_empty());

        let _ = fs::remove_dir_all(&dir);
    }
}
