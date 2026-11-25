//! Embedded JSON keyboard layouts (EN/PL/UA/RU/AR), user overrides,
//! and Standard / Compact for TV row builders.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::keycode::{IconId, KeyCode, KeyRole, ModifierKind};

const EN_JSON: &str = include_str!("../layouts/en.json");
const PL_JSON: &str = include_str!("../layouts/pl.json");
const UK_JSON: &str = include_str!("../layouts/uk.json");
const RU_JSON: &str = include_str!("../layouts/ru.json");
const AR_JSON: &str = include_str!("../layouts/ar.json");

/// Default subset of languages enabled in the top Fast Language Switcher on first run.
pub const DEFAULT_VISIBLE_LAYOUT_CODES: &[&str] = &["en", "pl", "uk", "ru", "ar"];

/// Complete built-in international layout catalog (28 languages).
pub const BUILTIN_LAYOUT_CODES: &[&str] = &[
    "en", "gb", "pl", "uk", "ru", "ar", "de", "fr", "es", "it", "pt", "nl", "cs", "sk", "hu", "ro",
    "hr", "tr", "sv", "no", "da", "fi", "el", "bg", "be", "lt", "he", "fa",
];

#[derive(Debug, Error)]
pub enum LayoutError {
    #[error("failed to parse layout JSON: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("unknown keycode name in layout: {0}")]
    UnknownKeyCode(String),
    #[error("failed to read layout file: {0}")]
    Io(#[from] std::io::Error),
}

/// Keyboard geometry style mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyboardStyle {
    #[default]
    Standard,
    CompactTv,
}

/// Active sub-page when in `CompactTv` mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompactPage {
    #[default]
    Letters,
    SymbolsNav,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PairLabel {
    pub normal: String,
    pub shift: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CharKeyDef {
    pub code: String,
    pub normal: String,
    pub shift: String,
    #[serde(default)]
    pub altgr: Option<String>,
    #[serde(default)]
    pub altgr_shift: Option<String>,
}

/// Serializable JSON layout definition.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LayoutDefinition {
    pub code: String,
    pub short_label: String,
    pub display_name: String,
    pub xkb_name: String,
    #[serde(default)]
    pub is_rtl: bool,
    pub grave: PairLabel,
    pub digits_shift: Vec<String>,
    pub top_row: Vec<CharKeyDef>,
    pub home_row: Vec<CharKeyDef>,
    pub bottom_row: Vec<CharKeyDef>,
    pub compact_rows: Vec<Vec<String>>,
    #[serde(default)]
    pub altgr: HashMap<String, PairLabel>,
}

/// Action triggered when activating a rendered keycap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyAction {
    /// Emit a physical Linux evdev keycode.
    Key(KeyCode),
    /// Toggle between `CompactPage::Letters` (`ABC`) and `CompactPage::SymbolsNav` (`123`).
    ToggleCompactPage,
}

/// A single keycap ready for layout geometry tiling and rendering.
#[derive(Debug, Clone)]
pub struct KeySpec {
    pub id: String,
    pub action: KeyAction,
    pub role: KeyRole,
    pub weight: f32,
    pub normal: String,
    pub shift: String,
    pub altgr: Option<String>,
    pub altgr_shift: Option<String>,
    pub icon: Option<IconId>,
    /// True if letters should obey CapsLock XOR Shift (vs Shift-only digits/symbols).
    pub is_alphabetic: bool,
    /// True if this key produces a diacritic/alternate symbol when AltGr is active.
    pub has_altgr: bool,
}

impl KeySpec {
    /// Compute the primary label and optional secondary corner hint for current modifier states.
    #[must_use]
    pub fn display_labels(
        &self,
        shift: bool,
        uppercase: bool,
        altgr: bool,
    ) -> (&str, Option<&str>) {
        if altgr && self.has_altgr {
            if (shift || (self.is_alphabetic && uppercase))
                && let Some(ref s) = self.altgr_shift
            {
                return (s.as_str(), Some(self.normal.as_str()));
            }
            if let Some(ref a) = self.altgr {
                return (a.as_str(), Some(self.normal.as_str()));
            }
        }

        if self.is_alphabetic {
            if uppercase {
                (&self.shift, self.altgr.as_deref())
            } else {
                (&self.normal, self.altgr.as_deref())
            }
        } else if shift {
            (&self.shift, Some(self.normal.as_str()))
        } else {
            let hint = if self.shift != self.normal && !self.shift.is_empty() {
                Some(self.shift.as_str())
            } else {
                None
            };
            (&self.normal, hint)
        }
    }

    fn char_key(def: &CharKeyDef, code: KeyCode, altgr_override: Option<&PairLabel>) -> Self {
        let altgr = def
            .altgr
            .clone()
            .or_else(|| altgr_override.map(|p| p.normal.clone()));
        let altgr_shift = def
            .altgr_shift
            .clone()
            .or_else(|| altgr_override.map(|p| p.shift.clone()));
        let has_altgr = altgr.is_some();
        let is_alphabetic = def.normal.chars().next().is_some_and(|c| c.is_alphabetic());
        Self {
            id: def.code.clone(),
            action: KeyAction::Key(code),
            role: KeyRole::Character,
            weight: 1.0,
            normal: def.normal.clone(),
            shift: def.shift.clone(),
            altgr,
            altgr_shift,
            icon: None,
            is_alphabetic,
            has_altgr,
        }
    }

    fn symbol(
        code: KeyCode,
        normal: impl Into<String>,
        shift: impl Into<String>,
        weight: f32,
    ) -> Self {
        let n = normal.into();
        let s = shift.into();
        Self {
            id: format!("k_{}", code.raw()),
            action: KeyAction::Key(code),
            role: KeyRole::Character,
            weight,
            normal: n,
            shift: s,
            altgr: None,
            altgr_shift: None,
            icon: None,
            is_alphabetic: false,
            has_altgr: false,
        }
    }

    fn modifier(id: &str, kind: ModifierKind, label: &str, weight: f32) -> Self {
        Self {
            id: id.to_owned(),
            action: KeyAction::Key(kind.keycode()),
            role: KeyRole::Modifier(kind),
            weight,
            normal: label.to_owned(),
            shift: label.to_owned(),
            altgr: None,
            altgr_shift: None,
            icon: None,
            is_alphabetic: false,
            has_altgr: false,
        }
    }

    fn action_key(id: &str, code: KeyCode, label: &str, icon: Option<IconId>, weight: f32) -> Self {
        Self {
            id: id.to_owned(),
            action: KeyAction::Key(code),
            role: KeyRole::Action,
            weight,
            normal: label.to_owned(),
            shift: label.to_owned(),
            altgr: None,
            altgr_shift: None,
            icon,
            is_alphabetic: false,
            has_altgr: false,
        }
    }

    fn space(weight: f32, layout_name: &str) -> Self {
        Self {
            id: "KEY_SPACE".to_owned(),
            action: KeyAction::Key(KeyCode::SPACE),
            role: KeyRole::Space,
            weight,
            normal: layout_name.to_owned(),
            shift: layout_name.to_owned(),
            altgr: None,
            altgr_shift: None,
            icon: None,
            is_alphabetic: false,
            has_altgr: false,
        }
    }

    fn layer_toggle(label: &str, weight: f32) -> Self {
        Self {
            id: "LAYER_TOGGLE".to_owned(),
            action: KeyAction::ToggleCompactPage,
            role: KeyRole::LayerToggle,
            weight,
            normal: label.to_owned(),
            shift: label.to_owned(),
            altgr: None,
            altgr_shift: None,
            icon: None,
            is_alphabetic: false,
            has_altgr: false,
        }
    }

    fn media(id: &str, code: KeyCode, label: &str, icon: IconId, weight: f32) -> Self {
        Self {
            id: id.to_owned(),
            action: KeyAction::Key(code),
            role: KeyRole::Media,
            weight,
            normal: label.to_owned(),
            shift: label.to_owned(),
            altgr: None,
            altgr_shift: None,
            icon: Some(icon),
            is_alphabetic: false,
            has_altgr: false,
        }
    }
}

/// System & Media controls strip shown when `show_media_row` is enabled.
#[must_use]
pub fn media_control_row() -> Vec<KeySpec> {
    vec![
        KeySpec::media(
            "KEY_BRIGHTNESSDOWN",
            KeyCode::BRIGHTNESSDOWN,
            "Bri-",
            IconId::BrightnessDown,
            1.0,
        ),
        KeySpec::media(
            "KEY_BRIGHTNESSUP",
            KeyCode::BRIGHTNESSUP,
            "Bri+",
            IconId::BrightnessUp,
            1.0,
        ),
        KeySpec::media(
            "KEY_PREVIOUSSONG",
            KeyCode::PREVIOUSSONG,
            "Prev",
            IconId::MediaPrev,
            1.0,
        ),
        KeySpec::media(
            "KEY_PLAYPAUSE",
            KeyCode::PLAYPAUSE,
            "Play",
            IconId::MediaPlayPause,
            1.15,
        ),
        KeySpec::media(
            "KEY_NEXTSONG",
            KeyCode::NEXTSONG,
            "Next",
            IconId::MediaNext,
            1.0,
        ),
        KeySpec::media("KEY_MUTE", KeyCode::MUTE, "Mute", IconId::AudioMute, 1.0),
        KeySpec::media(
            "KEY_VOLUMEDOWN",
            KeyCode::VOLUMEDOWN,
            "Vol-",
            IconId::AudioLow,
            1.0,
        ),
        KeySpec::media(
            "KEY_VOLUMEUP",
            KeyCode::VOLUMEUP,
            "Vol+",
            IconId::AudioHigh,
            1.0,
        ),
        KeySpec::media("KEY_SYSRQ", KeyCode::SYSRQ, "Shot", IconId::Screenshot, 1.0),
    ]
}

/// Registry holding built-in layouts and optional `~/.config/wlboard/layouts/*.json` overrides.
#[derive(Debug, Clone)]
pub struct LayoutRegistry {
    layouts: HashMap<String, LayoutDefinition>,
    order: Vec<String>,
}

impl Default for LayoutRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl LayoutRegistry {
    /// Load all 28 built-in layouts (5 core JSON layouts + 23 extended international layouts).
    #[must_use]
    pub fn new() -> Self {
        let mut reg = Self {
            layouts: HashMap::new(),
            order: Vec::new(),
        };
        for raw in [EN_JSON, PL_JSON, UK_JSON, RU_JSON, AR_JSON] {
            let def: LayoutDefinition =
                serde_json::from_str(raw).expect("built-in layout JSON must be valid");
            reg.insert(def);
        }
        for def in crate::catalog::extended_builtin_layouts() {
            reg.insert(def);
        }
        reg
    }

    /// Load user JSON overrides from `~/.config/wlboard/layouts/*.json`.
    pub fn load_overrides_from(&mut self, dir: &Path) -> Result<usize, LayoutError> {
        if !dir.exists() {
            return Ok(0);
        }
        let mut count = 0usize;
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                let raw = fs::read_to_string(&path)?;
                let def: LayoutDefinition = serde_json::from_str(&raw)?;
                self.insert(def);
                count += 1;
            }
        }
        Ok(count)
    }

    fn insert(&mut self, def: LayoutDefinition) {
        let code = def.code.to_lowercase();
        if !self.order.contains(&code) {
            self.order.push(code.clone());
        }
        self.layouts.insert(code, def);
    }

    /// Normalize external XKB/Plasma layout codes (e.g., `"ara"` -> `"ar"`, `"ua"` -> `"uk"`,
    /// `"us"` -> `"en"`, `"cz"` -> `"cs"`, `"se"` -> `"sv"`, `"dk"` -> `"da"`, `"gr"` -> `"el"`).
    #[must_use]
    pub fn normalize_code(&self, external: &str) -> Option<String> {
        let trimmed = external.trim().to_lowercase();
        let base = trimmed
            .split(['-', '_', '(', ':', ' '])
            .next()
            .unwrap_or("");
        let mapped = match base {
            "us" | "en" | "eng" => "en",
            "gb" | "en_gb" => "gb",
            "pl" | "pol" => "pl",
            "ua" | "uk" | "ukr" => "uk",
            "ru" | "rus" => "ru",
            "ar" | "ara" | "arb" => "ar",
            "de" | "deu" | "ger" => "de",
            "fr" | "fra" | "fre" => "fr",
            "es" | "spa" => "es",
            "it" | "ita" => "it",
            "pt" | "por" => "pt",
            "nl" | "nld" | "dut" => "nl",
            "cz" | "cs" | "ces" => "cs",
            "sk" | "slk" => "sk",
            "hu" | "hun" => "hu",
            "ro" | "ron" => "ro",
            "hr" | "hrv" => "hr",
            "tr" | "tur" => "tr",
            "se" | "sv" | "swe" => "sv",
            "no" | "nor" | "nb" => "no",
            "dk" | "da" | "dan" => "da",
            "fi" | "fin" => "fi",
            "gr" | "el" | "ell" => "el",
            "bg" | "bul" => "bg",
            "by" | "be" | "bel" => "be",
            "lt" | "lit" => "lt",
            "il" | "he" | "heb" => "he",
            "ir" | "fa" | "fas" => "fa",
            other => other,
        };
        if self.layouts.contains_key(mapped) {
            Some(mapped.to_owned())
        } else {
            None
        }
    }

    #[must_use]
    pub fn get(&self, code: &str) -> &LayoutDefinition {
        self.layouts
            .get(code)
            .or_else(|| self.layouts.get("en"))
            .expect("built-in 'en' layout must always exist")
    }

    #[must_use]
    pub fn available(&self) -> Vec<&LayoutDefinition> {
        self.order
            .iter()
            .filter_map(|code| self.layouts.get(code))
            .collect()
    }

    /// Build rows of `KeySpec` for the requested layout, style, and Compact TV sub-page.
    #[must_use]
    pub fn build_rows(
        &self,
        code: &str,
        style: KeyboardStyle,
        compact_page: CompactPage,
    ) -> Vec<Vec<KeySpec>> {
        let def = self.get(code);
        match style {
            KeyboardStyle::Standard => build_standard_rows(def),
            KeyboardStyle::CompactTv => match compact_page {
                CompactPage::Letters => build_compact_alpha_rows(def),
                CompactPage::SymbolsNav => build_compact_symbols_rows(def),
            },
        }
    }
}

fn digit_keycode(idx: usize) -> KeyCode {
    match idx {
        0 => KeyCode::KEY_1,
        1 => KeyCode::KEY_2,
        2 => KeyCode::KEY_3,
        3 => KeyCode::KEY_4,
        4 => KeyCode::KEY_5,
        5 => KeyCode::KEY_6,
        6 => KeyCode::KEY_7,
        7 => KeyCode::KEY_8,
        8 => KeyCode::KEY_9,
        9 => KeyCode::KEY_0,
        10 => KeyCode::MINUS,
        _ => KeyCode::EQUAL,
    }
}

fn digit_normal(idx: usize) -> &'static str {
    match idx {
        0 => "1",
        1 => "2",
        2 => "3",
        3 => "4",
        4 => "5",
        5 => "6",
        6 => "7",
        7 => "8",
        8 => "9",
        9 => "0",
        10 => "-",
        _ => "=",
    }
}

fn build_standard_rows(def: &LayoutDefinition) -> Vec<Vec<KeySpec>> {
    // Row 0: ` 1 2 3 4 5 6 7 8 9 0 - = Backspace (total weight = 15.0)
    let mut row0 = Vec::with_capacity(14);
    row0.push(KeySpec::symbol(
        KeyCode::GRAVE,
        def.grave.normal.clone(),
        def.grave.shift.clone(),
        1.0,
    ));
    for idx in 0..12 {
        let shift = def
            .digits_shift
            .get(idx)
            .cloned()
            .unwrap_or_else(|| digit_normal(idx).to_owned());
        row0.push(KeySpec::symbol(
            digit_keycode(idx),
            digit_normal(idx),
            shift,
            1.0,
        ));
    }
    row0.push(KeySpec::action_key(
        "KEY_BACKSPACE",
        KeyCode::BACKSPACE,
        "Bksp",
        Some(IconId::Backspace),
        2.0,
    ));

    // Row 1: Tab + 13 top_row keys (1.5 + 12*1.0 + 1.5 = 15.0)
    let mut row1 = Vec::with_capacity(14);
    row1.push(KeySpec::action_key(
        "KEY_TAB",
        KeyCode::TAB,
        "Tab",
        None,
        1.5,
    ));
    for (i, key) in def.top_row.iter().enumerate() {
        if let Some(kc) = KeyCode::from_name(&key.code) {
            let mut spec = KeySpec::char_key(key, kc, def.altgr.get(&key.code));
            if i + 1 == def.top_row.len() {
                spec.weight = 1.5;
            }
            row1.push(spec);
        }
    }

    // Row 2: Caps + 11 home_row keys + Enter (1.8 + 11*1.0 + 2.2 = 15.0)
    let mut row2 = Vec::with_capacity(13);
    row2.push(KeySpec::modifier(
        "KEY_CAPSLOCK",
        ModifierKind::CapsLock,
        "Caps",
        1.8,
    ));
    for key in &def.home_row {
        if let Some(kc) = KeyCode::from_name(&key.code) {
            row2.push(KeySpec::char_key(key, kc, def.altgr.get(&key.code)));
        }
    }
    row2.push(KeySpec::action_key(
        "KEY_ENTER",
        KeyCode::ENTER,
        "Return",
        None,
        2.2,
    ));

    // Row 3: Shift + 10 bottom_row keys + Up + Shift (2.1 + 10*1.0 + 1.0 + 1.9 = 15.0)
    let mut row3 = Vec::with_capacity(13);
    row3.push(KeySpec::modifier(
        "KEY_LEFTSHIFT",
        ModifierKind::Shift,
        "Shift",
        2.1,
    ));
    for key in &def.bottom_row {
        if let Some(kc) = KeyCode::from_name(&key.code) {
            row3.push(KeySpec::char_key(key, kc, def.altgr.get(&key.code)));
        }
    }
    row3.push(KeySpec::action_key(
        "KEY_UP",
        KeyCode::UP,
        "Up",
        Some(IconId::ArrowUp),
        1.0,
    ));
    row3.push(KeySpec::modifier(
        "KEY_RIGHTSHIFT",
        ModifierKind::Shift,
        "Shift",
        1.9,
    ));

    // Row 4: Ctrl Super Alt [Space] AltGr Left Down Right (1.4 + 1.3 + 1.3 + 6.4 + 1.6 + 1.0 + 1.0 + 1.0 = 15.0)
    let row4 = vec![
        KeySpec::modifier("KEY_LEFTCTRL", ModifierKind::Ctrl, "Ctrl", 1.4),
        KeySpec::modifier("KEY_LEFTMETA", ModifierKind::Super, "Super", 1.3),
        KeySpec::modifier("KEY_LEFTALT", ModifierKind::Alt, "Alt", 1.3),
        KeySpec::space(6.4, &def.display_name),
        KeySpec::modifier("KEY_RIGHTALT", ModifierKind::AltGr, "AltGr", 1.6),
        KeySpec::action_key(
            "KEY_LEFT",
            KeyCode::LEFT,
            "Left",
            Some(IconId::ArrowLeft),
            1.0,
        ),
        KeySpec::action_key(
            "KEY_DOWN",
            KeyCode::DOWN,
            "Down",
            Some(IconId::ArrowDown),
            1.0,
        ),
        KeySpec::action_key(
            "KEY_RIGHT",
            KeyCode::RIGHT,
            "Right",
            Some(IconId::ArrowRight),
            1.0,
        ),
    ];

    vec![row0, row1, row2, row3, row4]
}

fn build_compact_alpha_rows(def: &LayoutDefinition) -> Vec<Vec<KeySpec>> {
    let mut lookup: HashMap<&str, &CharKeyDef> = HashMap::new();
    for k in def
        .top_row
        .iter()
        .chain(def.home_row.iter())
        .chain(def.bottom_row.iter())
    {
        lookup.insert(k.code.as_str(), k);
    }

    // Row 0: compact top letters + Backspace
    let mut row0 = Vec::new();
    if let Some(codes) = def.compact_rows.first() {
        for code in codes {
            if let (Some(kdef), Some(kc)) = (lookup.get(code.as_str()), KeyCode::from_name(code)) {
                row0.push(KeySpec::char_key(kdef, kc, def.altgr.get(code)));
            }
        }
    }
    row0.push(KeySpec::action_key(
        "KEY_BACKSPACE",
        KeyCode::BACKSPACE,
        "Bksp",
        Some(IconId::Backspace),
        1.5,
    ));

    // Row 1: compact home letters + Enter
    let mut row1 = Vec::new();
    if let Some(codes) = def.compact_rows.get(1) {
        for code in codes {
            if let (Some(kdef), Some(kc)) = (lookup.get(code.as_str()), KeyCode::from_name(code)) {
                row1.push(KeySpec::char_key(kdef, kc, def.altgr.get(code)));
            }
        }
    }
    row1.push(KeySpec::action_key(
        "KEY_ENTER",
        KeyCode::ENTER,
        "Return",
        None,
        1.8,
    ));

    // Row 2: Shift + compact bottom letters + Up
    let mut row2 = Vec::new();
    row2.push(KeySpec::modifier(
        "KEY_LEFTSHIFT",
        ModifierKind::Shift,
        "Shift",
        1.5,
    ));
    if let Some(codes) = def.compact_rows.get(2) {
        for code in codes {
            if let (Some(kdef), Some(kc)) = (lookup.get(code.as_str()), KeyCode::from_name(code)) {
                row2.push(KeySpec::char_key(kdef, kc, def.altgr.get(code)));
            }
        }
    }
    row2.push(KeySpec::action_key(
        "KEY_UP",
        KeyCode::UP,
        "Up",
        Some(IconId::ArrowUp),
        1.1,
    ));

    // Row 3: 123 + AltGr + Space + Left Down Right
    let row3 = vec![
        KeySpec::layer_toggle("123", 1.5),
        KeySpec::modifier("KEY_RIGHTALT", ModifierKind::AltGr, "AltGr", 1.4),
        KeySpec::space(5.6, &def.display_name),
        KeySpec::action_key(
            "KEY_LEFT",
            KeyCode::LEFT,
            "Left",
            Some(IconId::ArrowLeft),
            1.0,
        ),
        KeySpec::action_key(
            "KEY_DOWN",
            KeyCode::DOWN,
            "Down",
            Some(IconId::ArrowDown),
            1.0,
        ),
        KeySpec::action_key(
            "KEY_RIGHT",
            KeyCode::RIGHT,
            "Right",
            Some(IconId::ArrowRight),
            1.0,
        ),
    ];

    vec![row0, row1, row2, row3]
}

fn build_compact_symbols_rows(def: &LayoutDefinition) -> Vec<Vec<KeySpec>> {
    // Row 0: F1..F12
    let row0 = vec![
        KeySpec::action_key("KEY_F1", KeyCode::F1, "F1", None, 1.0),
        KeySpec::action_key("KEY_F2", KeyCode::F2, "F2", None, 1.0),
        KeySpec::action_key("KEY_F3", KeyCode::F3, "F3", None, 1.0),
        KeySpec::action_key("KEY_F4", KeyCode::F4, "F4", None, 1.0),
        KeySpec::action_key("KEY_F5", KeyCode::F5, "F5", None, 1.0),
        KeySpec::action_key("KEY_F6", KeyCode::F6, "F6", None, 1.0),
        KeySpec::action_key("KEY_F7", KeyCode::F7, "F7", None, 1.0),
        KeySpec::action_key("KEY_F8", KeyCode::F8, "F8", None, 1.0),
        KeySpec::action_key("KEY_F9", KeyCode::F9, "F9", None, 1.0),
        KeySpec::action_key("KEY_F10", KeyCode::F10, "F10", None, 1.0),
        KeySpec::action_key("KEY_F11", KeyCode::F11, "F11", None, 1.0),
        KeySpec::action_key("KEY_F12", KeyCode::F12, "F12", None, 1.0),
    ];

    // Row 1: 1..0 - = Backspace
    let mut row1 = Vec::with_capacity(13);
    for idx in 0..12 {
        let shift = def
            .digits_shift
            .get(idx)
            .cloned()
            .unwrap_or_else(|| digit_normal(idx).to_owned());
        row1.push(KeySpec::symbol(
            digit_keycode(idx),
            digit_normal(idx),
            shift,
            1.0,
        ));
    }
    row1.push(KeySpec::action_key(
        "KEY_BACKSPACE",
        KeyCode::BACKSPACE,
        "Bksp",
        Some(IconId::Backspace),
        1.6,
    ));

    // Row 2: Punctuation + Navigation (Home/End/PgUp/PgDn/Del)
    let row2 = vec![
        KeySpec::symbol(
            KeyCode::GRAVE,
            def.grave.normal.clone(),
            def.grave.shift.clone(),
            1.0,
        ),
        KeySpec::symbol(KeyCode::LEFTBRACE, "[", "{", 1.0),
        KeySpec::symbol(KeyCode::RIGHTBRACE, "]", "}", 1.0),
        KeySpec::symbol(KeyCode::BACKSLASH, "\\", "|", 1.0),
        KeySpec::symbol(KeyCode::SEMICOLON, ";", ":", 1.0),
        KeySpec::symbol(KeyCode::APOSTROPHE, "'", "\"", 1.0),
        KeySpec::symbol(KeyCode::SLASH, "/", "?", 1.0),
        KeySpec::action_key("KEY_HOME", KeyCode::HOME, "Home", None, 1.15),
        KeySpec::action_key("KEY_END", KeyCode::END, "End", None, 1.05),
        KeySpec::action_key("KEY_PAGEUP", KeyCode::PAGEUP, "PgUp", None, 1.15),
        KeySpec::action_key("KEY_PAGEDOWN", KeyCode::PAGEDOWN, "PgDn", None, 1.15),
        KeySpec::action_key("KEY_DELETE", KeyCode::DELETE, "Del", None, 1.1),
    ];

    // Row 3: ABC Tab Ctrl Super Alt Space Left Down Right Enter
    let row3 = vec![
        KeySpec::layer_toggle("ABC", 1.5),
        KeySpec::action_key("KEY_TAB", KeyCode::TAB, "Tab", None, 1.1),
        KeySpec::modifier("KEY_LEFTCTRL", ModifierKind::Ctrl, "Ctrl", 1.15),
        KeySpec::modifier("KEY_LEFTMETA", ModifierKind::Super, "Super", 1.2),
        KeySpec::modifier("KEY_LEFTALT", ModifierKind::Alt, "Alt", 1.05),
        KeySpec::space(3.0, &def.short_label),
        KeySpec::action_key(
            "KEY_LEFT",
            KeyCode::LEFT,
            "Left",
            Some(IconId::ArrowLeft),
            0.9,
        ),
        KeySpec::action_key(
            "KEY_DOWN",
            KeyCode::DOWN,
            "Down",
            Some(IconId::ArrowDown),
            0.9,
        ),
        KeySpec::action_key(
            "KEY_RIGHT",
            KeyCode::RIGHT,
            "Right",
            Some(IconId::ArrowRight),
            0.9,
        ),
        KeySpec::action_key("KEY_ENTER", KeyCode::ENTER, "Return", None, 1.5),
    ];

    vec![row0, row1, row2, row3]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_all_builtin_layouts() {
        let reg = LayoutRegistry::new();
        assert_eq!(reg.available().len(), BUILTIN_LAYOUT_CODES.len());
        assert_eq!(reg.available().len(), 28);
        assert_eq!(reg.get("en").short_label, "EN");
        assert_eq!(reg.get("gb").short_label, "UK");
        assert_eq!(reg.get("pl").short_label, "PL");
        assert_eq!(reg.get("uk").short_label, "UA");
        assert_eq!(reg.get("ru").short_label, "RU");
        assert_eq!(reg.get("ar").short_label, "AR");
        assert_eq!(reg.get("de").short_label, "DE");
        assert_eq!(reg.get("fr").short_label, "FR");
        assert_eq!(reg.get("el").short_label, "EL");
        assert!(reg.get("ar").is_rtl);
        assert!(reg.get("he").is_rtl);
        assert!(reg.get("fa").is_rtl);
    }

    #[test]
    fn normalizes_xkb_and_plasma_codes() {
        let reg = LayoutRegistry::new();
        assert_eq!(reg.normalize_code("ara"), Some("ar".to_owned()));
        assert_eq!(reg.normalize_code("ua"), Some("uk".to_owned()));
        assert_eq!(reg.normalize_code("us"), Some("en".to_owned()));
        assert_eq!(reg.normalize_code("gb"), Some("gb".to_owned()));
        assert_eq!(reg.normalize_code("pl(qwertz)"), Some("pl".to_owned()));
        assert_eq!(reg.normalize_code("ru"), Some("ru".to_owned()));
        assert_eq!(reg.normalize_code("cz"), Some("cs".to_owned()));
        assert_eq!(reg.normalize_code("gr"), Some("el".to_owned()));
        assert_eq!(reg.normalize_code("il"), Some("he".to_owned()));
        assert_eq!(reg.normalize_code("se"), Some("sv".to_owned()));
        assert_eq!(reg.normalize_code("nonexistent"), None);
    }

    #[test]
    fn polish_layout_marks_all_diacritic_altgr_keys() {
        let reg = LayoutRegistry::new();
        let rows = reg.build_rows("pl", KeyboardStyle::Standard, CompactPage::Letters);
        let altgr_keys: Vec<_> = rows
            .iter()
            .flatten()
            .filter(|k| k.has_altgr)
            .map(|k| (k.normal.as_str(), k.altgr.as_deref().unwrap_or("")))
            .collect();
        assert!(altgr_keys.contains(&("a", "ą")));
        assert!(altgr_keys.contains(&("c", "ć")));
        assert!(altgr_keys.contains(&("e", "ę")));
        assert!(altgr_keys.contains(&("l", "ł")));
        assert!(altgr_keys.contains(&("n", "ń")));
        assert!(altgr_keys.contains(&("o", "ó")));
        assert!(altgr_keys.contains(&("s", "ś")));
        assert!(altgr_keys.contains(&("x", "ź")));
        assert!(altgr_keys.contains(&("z", "ż")));
    }

    #[test]
    fn standard_rows_have_balanced_row_weights() {
        let reg = LayoutRegistry::new();
        for code in BUILTIN_LAYOUT_CODES {
            let rows = reg.build_rows(code, KeyboardStyle::Standard, CompactPage::Letters);
            assert_eq!(rows.len(), 5, "standard mode must have 5 rows for {code}");
            for (idx, row) in rows.iter().enumerate() {
                let sum: f32 = row.iter().map(|k| k.weight).sum();
                assert!(
                    (sum - 15.0).abs() < 0.01,
                    "row {idx} in {code} has unbalanced weight {sum}"
                );
            }
        }
    }

    #[test]
    fn compact_tv_pages_include_toggle_and_all_letters() {
        let reg = LayoutRegistry::new();
        for code in BUILTIN_LAYOUT_CODES {
            let alpha = reg.build_rows(code, KeyboardStyle::CompactTv, CompactPage::Letters);
            assert_eq!(alpha.len(), 4);
            assert!(
                alpha
                    .iter()
                    .flatten()
                    .any(|k| k.action == KeyAction::ToggleCompactPage && k.normal == "123")
            );

            let sym = reg.build_rows(code, KeyboardStyle::CompactTv, CompactPage::SymbolsNav);
            assert_eq!(sym.len(), 4);
            assert!(
                sym.iter()
                    .flatten()
                    .any(|k| k.action == KeyAction::ToggleCompactPage && k.normal == "ABC")
            );
        }
    }
}
