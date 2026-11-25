//! TOML application configuration persistence (`~/.config/wlboard/config.toml`).

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::layout::{BUILTIN_LAYOUT_CODES, KeyboardStyle};
use crate::panel::PANEL_STANDARD;
use crate::theme::AppearanceSettings;
use crate::touch::{MIN_WINDOW_HEIGHT, MIN_WINDOW_WIDTH};

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("I/O error accessing config: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse config TOML: {0}")]
    Deserialize(#[from] toml::de::Error),
    #[error("failed to serialize config TOML: {0}")]
    Serialize(#[from] toml::ser::Error),
}

/// Visual label style for modifier and action keys (`Tab`, `Caps`, `Shift`, `Return`, `Del`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeycapStyle {
    #[default]
    Text,
    Symbols,
}

/// Floating overlay geometry persisted across sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowGeometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Default for WindowGeometry {
    fn default() -> Self {
        Self {
            x: 180,
            y: 560,
            width: 900,
            height: 315,
        }
    }
}

impl WindowGeometry {
    pub fn clamp(&mut self) {
        self.width = self.width.clamp(MIN_WINDOW_WIDTH, 3840);
        self.height = self.height.clamp(MIN_WINDOW_HEIGHT, 1600);
        self.x = self.x.clamp(0, 7680);
        self.y = self.y.clamp(0, 4320);
    }
}

/// Layout, panel, and toolbar preferences.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayoutSettings {
    pub active: String,
    pub style: KeyboardStyle,
    /// Active 2D panel ID (`"standard"`, `"compact_tv"`, `"numpad_nav"`, `"shortcuts_media"`, or custom ID).
    pub active_panel: String,
    /// Ordered subset of languages shown in the top Fast Language Switcher bar.
    pub visible_languages: Vec<String>,
    pub system_layout_sync: bool,
    /// Automatically add missing layouts into KDE Plasma (`kxkbrc`) when switching on desktop mode.
    pub auto_register_kde_layouts: bool,
    pub show_fast_switcher: bool,
    pub show_panel_switcher: bool,
    pub show_media_row: bool,
    pub altgr_amber_hints: bool,
    pub keycap_style: KeycapStyle,
}

impl Default for LayoutSettings {
    fn default() -> Self {
        Self {
            active: "en".to_owned(),
            style: KeyboardStyle::Standard,
            active_panel: PANEL_STANDARD.to_owned(),
            visible_languages: BUILTIN_LAYOUT_CODES
                .iter()
                .map(|c| (*c).to_owned())
                .collect(),
            system_layout_sync: true,
            auto_register_kde_layouts: true,
            show_fast_switcher: true,
            show_panel_switcher: true,
            show_media_row: false,
            altgr_amber_hints: true,
            keycap_style: KeycapStyle::Text,
        }
    }
}

/// Input and startup behavior settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BehaviorSettings {
    pub sticky_modifier_lock: bool,
    pub start_minimized: bool,
}

impl Default for BehaviorSettings {
    fn default() -> Self {
        Self {
            sticky_modifier_lock: true,
            start_minimized: false,
        }
    }
}

/// Root configuration document serialized as `~/.config/wlboard/config.toml`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub window: WindowGeometry,
    pub layout: LayoutSettings,
    pub behavior: BehaviorSettings,
    pub appearance: AppearanceSettings,
    pub active_preset: Option<String>,
}

impl AppConfig {
    /// Resolve `~/.config/wlboard` using XDG conventions.
    #[must_use]
    pub fn config_dir() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("wlboard")
    }

    #[must_use]
    pub fn default_config_path() -> PathBuf {
        Self::config_dir().join("config.toml")
    }

    #[must_use]
    pub fn presets_dir() -> PathBuf {
        Self::config_dir().join("presets")
    }

    #[must_use]
    pub fn panels_dir() -> PathBuf {
        Self::config_dir().join("panels")
    }

    #[must_use]
    pub fn layouts_dir() -> PathBuf {
        Self::config_dir().join("layouts")
    }

    pub fn clamp(&mut self) {
        self.window.clamp();
        self.appearance.clamp();
        // Ensure visible_languages contains no duplicates and at least one language.
        let mut dedup = Vec::new();
        for code in &self.layout.visible_languages {
            let c = code.trim().to_lowercase();
            if !c.is_empty() && !dedup.contains(&c) {
                dedup.push(c);
            }
        }
        if dedup.is_empty() {
            dedup.push("en".to_owned());
        }
        self.layout.visible_languages = dedup;
        if self.layout.active_panel.trim().is_empty() {
            self.layout.active_panel = PANEL_STANDARD.to_owned();
        }
    }

    /// Load configuration from a path, returning defaults if the file does not exist.
    pub fn load_from(path: &Path) -> Result<Self, ConfigError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(path)?;
        let mut cfg: Self = toml::from_str(&raw)?;
        cfg.clamp();
        Ok(cfg)
    }

    /// Load from the default XDG config path, falling back to defaults on parse failure.
    #[must_use]
    pub fn load_default() -> Self {
        Self::load_from(&Self::default_config_path()).unwrap_or_default()
    }

    /// Atomically write configuration to disk.
    pub fn save_to(&self, path: &Path) -> Result<(), ConfigError> {
        let mut sanitized = self.clone();
        sanitized.clamp();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let serialized = toml::to_string_pretty(&sanitized)?;
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, serialized)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn save_default(&self) -> Result<(), ConfigError> {
        self.save_to(&Self::default_config_path())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Color, ThemeBase};

    #[test]
    fn toml_config_round_trip_and_clamping() {
        let dir = std::env::temp_dir().join(format!("wlboard_cfg_test_{}", std::process::id()));
        let path = dir.join("config.toml");

        let mut cfg = AppConfig::default();
        cfg.layout.active = "pl".to_owned();
        cfg.layout.visible_languages = vec!["en".to_owned(), "pl".to_owned(), "uk".to_owned()];
        cfg.layout.keycap_style = KeycapStyle::Symbols;
        cfg.layout.style = KeyboardStyle::CompactTv;
        cfg.layout.show_media_row = true;
        cfg.appearance.apply_base(ThemeBase::Slate);
        cfg.appearance.colors.accent = Color::rgb(0x10, 0xB9, 0x81);
        cfg.appearance.opacity = 5.0;
        cfg.window.width = 100;
        cfg.save_to(&path).expect("save config");

        let loaded = AppConfig::load_from(&path).expect("load config");
        assert_eq!(loaded.layout.active, "pl");
        assert_eq!(loaded.layout.visible_languages, vec!["en", "pl", "uk"]);
        assert_eq!(loaded.layout.keycap_style, KeycapStyle::Symbols);
        assert_eq!(loaded.layout.style, KeyboardStyle::CompactTv);
        assert!(loaded.layout.show_media_row);
        assert_eq!(loaded.appearance.base, ThemeBase::Slate);
        assert_eq!(
            loaded.appearance.colors.accent,
            Color::rgb(0x10, 0xB9, 0x81)
        );
        assert!((loaded.appearance.opacity - 1.0).abs() < f32::EPSILON);
        assert_eq!(loaded.window.width, MIN_WINDOW_WIDTH);

        let _ = fs::remove_dir_all(&dir);
    }
}
