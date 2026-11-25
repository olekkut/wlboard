//! Unified 2D 0.5-unit snap-grid panel model, 4 built-in starter panels
//! (`Standard`, `Compact TV`, `Numpad & Nav`, `Shortcuts & Media`), and JSON `PanelStore`.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::config::AppConfig;
use crate::keycode::{IconId, KeyCode, KeyRole, ModifierKind};
use crate::layout::{CharKeyDef, CompactPage, LayoutDefinition};
use crate::presets::PresetStore;
use crate::theme::Color;

/// Built-in panel IDs.
pub const PANEL_STANDARD: &str = "standard";
pub const PANEL_COMPACT_TV: &str = "compact_tv";
pub const PANEL_NUMPAD_NAV: &str = "numpad_nav";
pub const PANEL_SHORTCUTS_MEDIA: &str = "shortcuts_media";

#[derive(Debug, Error)]
pub enum PanelError {
    #[error("panel name cannot be empty")]
    EmptyName,
    #[error("panel '{0}' not found")]
    NotFound(String),
    #[error("I/O error accessing panel file: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid panel JSON: {0}")]
    Json(#[from] serde_json::Error),
}

/// Action triggered by a 2D grid keycap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GridKeyAction {
    /// Emits `keycode` and dynamically adapts its glyph/AltGr hints to the active language layout.
    LayoutKey { keycode: KeyCode },
    /// Always emits `keycode` with its fixed label/icon (e.g. `Del`, `F1..F12`, Media keys).
    FixedKey { keycode: KeyCode },
    /// Emits a multi-key shortcut combo (e.g. `Ctrl+C`, `Alt+Tab`, `Super+D`).
    Shortcut {
        modifiers: Vec<ModifierKind>,
        keycode: KeyCode,
    },
    /// Inserts a custom UTF-8 text snippet.
    TextSnippet { text: String },
    /// Switches the active keyboard panel (e.g. `"standard"`, `"numpad_nav"`, custom panel ID).
    SwitchPanel { panel_id: String },
    /// Toggles `ABC` <-> `123` sub-page inside `Compact TV`.
    ToggleCompactPage,
}

/// A single positioned keycap on a 2D 0.5-unit coordinate grid.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GridKeySpec {
    pub id: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub action: GridKeyAction,
    pub role: KeyRole,
    pub label: String,
    #[serde(default)]
    pub shift_label: String,
    #[serde(default)]
    pub icon: Option<IconId>,
    /// Optional monochrome vector symbol shown when `KeycapStyle::Symbols` is enabled.
    #[serde(default)]
    pub symbol_icon: Option<IconId>,
    /// Optional per-key accent/background tint override configurable in the Panel Editor.
    #[serde(default)]
    pub color_override: Option<Color>,
}

/// Serializable 2D grid keyboard panel saved in `~/.config/wlboard/panels/*.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PanelDefinition {
    pub id: String,
    pub name: String,
    pub cols: f32,
    pub rows: f32,
    pub keys: Vec<GridKeySpec>,
    #[serde(default)]
    pub built_in: bool,
}

impl PanelDefinition {
    /// Snap any coordinate or dimension cleanly to the 0.5-unit editor grid.
    #[must_use]
    pub fn snap_half(v: f32) -> f32 {
        (v * 2.0).round() * 0.5
    }

    /// Clamp grid dimensions and key boxes into valid ranges.
    pub fn sanitize(&mut self) {
        self.cols = Self::snap_half(self.cols).clamp(4.0, 24.0);
        self.rows = Self::snap_half(self.rows).clamp(2.0, 10.0);
        for k in &mut self.keys {
            k.w = Self::snap_half(k.w).clamp(0.5, self.cols);
            k.h = Self::snap_half(k.h).clamp(0.5, self.rows);
            k.x = Self::snap_half(k.x).clamp(0.0, (self.cols - k.w).max(0.0));
            k.y = Self::snap_half(k.y).clamp(0.0, (self.rows - k.h).max(0.0));
        }
    }

    /// Find an unoccupied `(x, y)` slot for adding a new `1.0 × 1.0` button in the Panel Editor.
    #[must_use]
    pub fn find_free_slot(&self, w: f32, h: f32) -> (f32, f32) {
        let max_y = (self.rows - h).max(0.0);
        let max_x = (self.cols - w).max(0.0);
        let steps_y = (max_y * 2.0).round() as usize;
        let steps_x = (max_x * 2.0).round() as usize;

        for sy in 0..=steps_y {
            let y = sy as f32 * 0.5;
            for sx in 0..=steps_x {
                let x = sx as f32 * 0.5;
                let overlaps = self.keys.iter().any(|k| {
                    x < k.x + k.w - 0.05
                        && x + w > k.x + 0.05
                        && y < k.y + k.h - 0.05
                        && y + h > k.y + 0.05
                });
                if !overlaps {
                    return (x, y);
                }
            }
        }
        (0.0, 0.0)
    }

    /// Generate a unique key ID inside this panel.
    #[must_use]
    pub fn next_key_id(&self) -> String {
        let mut idx = self.keys.len() + 1;
        loop {
            let cand = format!("custom_key_{idx}");
            if !self.keys.iter().any(|k| k.id == cand) {
                return cand;
            }
            idx += 1;
        }
    }
}

/// Resolved, language-aware runtime keycap ready to render on screen or dispatch input.
#[derive(Debug, Clone)]
pub struct ResolvedGridKey {
    pub grid: GridKeySpec,
    pub normal: String,
    pub shift: String,
    pub altgr: Option<String>,
    pub altgr_shift: Option<String>,
    pub is_alphabetic: bool,
    pub has_altgr: bool,
}

impl ResolvedGridKey {
    /// Resolve dynamic character labels from `layout` for `GridKeyAction::LayoutKey`.
    #[must_use]
    pub fn resolve(spec: GridKeySpec, layout: &LayoutDefinition) -> Self {
        let mut normal = spec.label.clone();
        let mut shift = if spec.shift_label.is_empty() {
            spec.label.clone()
        } else {
            spec.shift_label.clone()
        };
        let mut altgr = None;
        let mut altgr_shift = None;
        let mut is_alphabetic = false;
        let mut has_altgr = false;

        if spec.role == KeyRole::Space && spec.label.is_empty() {
            normal = layout.display_name.clone();
            shift = layout.display_name.clone();
        }

        if let GridKeyAction::LayoutKey { keycode } = spec.action {
            if keycode == KeyCode::GRAVE {
                normal = layout.grave.normal.clone();
                shift = layout.grave.shift.clone();
            } else if let Some(didx) = digit_index(keycode) {
                normal = digit_label(didx).to_owned();
                shift = layout
                    .digits_shift
                    .get(didx)
                    .cloned()
                    .unwrap_or_else(|| normal.clone());
            } else if let Some(kdef) = find_layout_char(layout, keycode) {
                normal = kdef.normal.clone();
                shift = kdef.shift.clone();
                let altgr_override = layout.altgr.get(&kdef.code);
                altgr = kdef
                    .altgr
                    .clone()
                    .or_else(|| altgr_override.map(|p| p.normal.clone()));
                altgr_shift = kdef
                    .altgr_shift
                    .clone()
                    .or_else(|| altgr_override.map(|p| p.shift.clone()));
                has_altgr = altgr.is_some();
                is_alphabetic = normal.chars().next().is_some_and(char::is_alphabetic);
            }
        }

        Self {
            grid: spec,
            normal,
            shift,
            altgr,
            altgr_shift,
            is_alphabetic,
            has_altgr,
        }
    }

    /// Compute `(primary_label, optional_corner_hint)` for the active modifier state.
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
}

fn digit_index(kc: KeyCode) -> Option<usize> {
    Some(match kc {
        KeyCode::KEY_1 => 0,
        KeyCode::KEY_2 => 1,
        KeyCode::KEY_3 => 2,
        KeyCode::KEY_4 => 3,
        KeyCode::KEY_5 => 4,
        KeyCode::KEY_6 => 5,
        KeyCode::KEY_7 => 6,
        KeyCode::KEY_8 => 7,
        KeyCode::KEY_9 => 8,
        KeyCode::KEY_0 => 9,
        KeyCode::MINUS => 10,
        KeyCode::EQUAL => 11,
        _ => return None,
    })
}

fn digit_label(idx: usize) -> &'static str {
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

fn find_layout_char(layout: &LayoutDefinition, kc: KeyCode) -> Option<&CharKeyDef> {
    layout
        .top_row
        .iter()
        .chain(layout.home_row.iter())
        .chain(layout.bottom_row.iter())
        .find(|def| KeyCode::from_name(&def.code) == Some(kc))
}

fn layout_key(id: &str, x: f32, y: f32, w: f32, h: f32, kc: KeyCode) -> GridKeySpec {
    GridKeySpec {
        id: id.to_owned(),
        x,
        y,
        w,
        h,
        action: GridKeyAction::LayoutKey { keycode: kc },
        role: KeyRole::Character,
        label: kc.short_name().to_owned(),
        shift_label: String::new(),
        icon: None,
        symbol_icon: None,
        color_override: None,
    }
}

#[allow(clippy::too_many_arguments)]
fn fixed_action(
    id: &str,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    kc: KeyCode,
    label: &str,
    icon: Option<IconId>,
    symbol_icon: Option<IconId>,
) -> GridKeySpec {
    GridKeySpec {
        id: id.to_owned(),
        x,
        y,
        w,
        h,
        action: GridKeyAction::FixedKey { keycode: kc },
        role: KeyRole::Action,
        label: label.to_owned(),
        shift_label: String::new(),
        icon,
        symbol_icon,
        color_override: None,
    }
}

#[allow(clippy::too_many_arguments)]
fn mod_key(
    id: &str,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    kind: ModifierKind,
    label: &str,
    symbol_icon: Option<IconId>,
) -> GridKeySpec {
    GridKeySpec {
        id: id.to_owned(),
        x,
        y,
        w,
        h,
        action: GridKeyAction::FixedKey {
            keycode: kind.keycode(),
        },
        role: KeyRole::Modifier(kind),
        label: label.to_owned(),
        shift_label: String::new(),
        icon: None,
        symbol_icon,
        color_override: None,
    }
}

fn space_key(x: f32, y: f32, w: f32, h: f32) -> GridKeySpec {
    GridKeySpec {
        id: "KEY_SPACE".to_owned(),
        x,
        y,
        w,
        h,
        action: GridKeyAction::FixedKey {
            keycode: KeyCode::SPACE,
        },
        role: KeyRole::Space,
        label: String::new(),
        shift_label: String::new(),
        icon: None,
        symbol_icon: None,
        color_override: None,
    }
}

#[allow(clippy::too_many_arguments)]
fn shortcut_key(
    id: &str,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    label: &str,
    sub: &str,
    modifiers: Vec<ModifierKind>,
    keycode: KeyCode,
) -> GridKeySpec {
    GridKeySpec {
        id: id.to_owned(),
        x,
        y,
        w,
        h,
        action: GridKeyAction::Shortcut { modifiers, keycode },
        role: KeyRole::Action,
        label: label.to_owned(),
        shift_label: sub.to_owned(),
        icon: None,
        symbol_icon: None,
        color_override: None,
    }
}

#[allow(clippy::too_many_arguments)]
fn media_key(
    id: &str,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    kc: KeyCode,
    label: &str,
    icon: IconId,
) -> GridKeySpec {
    GridKeySpec {
        id: id.to_owned(),
        x,
        y,
        w,
        h,
        action: GridKeyAction::FixedKey { keycode: kc },
        role: KeyRole::Media,
        label: label.to_owned(),
        shift_label: String::new(),
        icon: Some(icon),
        symbol_icon: Some(icon),
        color_override: None,
    }
}

/// Push a sleek half-height Inverted-T arrow cluster occupying `3.0 × 1.0` at `(base_x, base_y)`.
fn push_inverted_t_arrows(keys: &mut Vec<GridKeySpec>, base_x: f32, base_y: f32) {
    keys.push(fixed_action(
        "KEY_LEFT",
        base_x,
        base_y + 0.5,
        1.0,
        0.5,
        KeyCode::LEFT,
        "Left",
        Some(IconId::ArrowLeft),
        Some(IconId::ArrowLeft),
    ));
    keys.push(fixed_action(
        "KEY_UP",
        base_x + 1.0,
        base_y,
        1.0,
        0.5,
        KeyCode::UP,
        "Up",
        Some(IconId::ArrowUp),
        Some(IconId::ArrowUp),
    ));
    keys.push(fixed_action(
        "KEY_DOWN",
        base_x + 1.0,
        base_y + 0.5,
        1.0,
        0.5,
        KeyCode::DOWN,
        "Down",
        Some(IconId::ArrowDown),
        Some(IconId::ArrowDown),
    ));
    keys.push(fixed_action(
        "KEY_RIGHT",
        base_x + 2.0,
        base_y + 0.5,
        1.0,
        0.5,
        KeyCode::RIGHT,
        "Right",
        Some(IconId::ArrowRight),
        Some(IconId::ArrowRight),
    ));
}

/// Built-in Starter Panel 1: **Standard** (`15.5 × 5.0` grid) with dedicated `Del` next to `Backspace`
/// and a sleek half-height **Inverted-T arrow cluster** (`↑` stacked directly above `↓`, flanked by `←`/`→`).
#[must_use]
pub fn build_standard_panel() -> PanelDefinition {
    let mut keys = Vec::with_capacity(68);

    // Row 0 (y=0.0, total 15.5): ` 1..0 - = Bksp(1.5) Del(1.0)
    let row0_codes = [
        ("KEY_GRAVE", KeyCode::GRAVE),
        ("KEY_1", KeyCode::KEY_1),
        ("KEY_2", KeyCode::KEY_2),
        ("KEY_3", KeyCode::KEY_3),
        ("KEY_4", KeyCode::KEY_4),
        ("KEY_5", KeyCode::KEY_5),
        ("KEY_6", KeyCode::KEY_6),
        ("KEY_7", KeyCode::KEY_7),
        ("KEY_8", KeyCode::KEY_8),
        ("KEY_9", KeyCode::KEY_9),
        ("KEY_0", KeyCode::KEY_0),
        ("KEY_MINUS", KeyCode::MINUS),
        ("KEY_EQUAL", KeyCode::EQUAL),
    ];
    for (idx, (id, kc)) in row0_codes.iter().enumerate() {
        keys.push(layout_key(id, idx as f32, 0.0, 1.0, 1.0, *kc));
    }
    keys.push(fixed_action(
        "KEY_BACKSPACE",
        13.0,
        0.0,
        1.5,
        1.0,
        KeyCode::BACKSPACE,
        "Bksp",
        Some(IconId::Backspace),
        Some(IconId::Backspace),
    ));
    keys.push(fixed_action(
        "KEY_DELETE",
        14.5,
        0.0,
        1.0,
        1.0,
        KeyCode::DELETE,
        "Del",
        None,
        Some(IconId::ForwardDelete),
    ));

    // Row 1 (y=1.0, total 15.5): Tab(1.5) Q..P [ ] \(2.0)
    keys.push(fixed_action(
        "KEY_TAB",
        0.0,
        1.0,
        1.5,
        1.0,
        KeyCode::TAB,
        "Tab",
        None,
        Some(IconId::Tab),
    ));
    let row1_codes = [
        ("KEY_Q", KeyCode::Q),
        ("KEY_W", KeyCode::W),
        ("KEY_E", KeyCode::E),
        ("KEY_R", KeyCode::R),
        ("KEY_T", KeyCode::T),
        ("KEY_Y", KeyCode::Y),
        ("KEY_U", KeyCode::U),
        ("KEY_I", KeyCode::I),
        ("KEY_O", KeyCode::O),
        ("KEY_P", KeyCode::P),
        ("KEY_LEFTBRACE", KeyCode::LEFTBRACE),
        ("KEY_RIGHTBRACE", KeyCode::RIGHTBRACE),
    ];
    for (idx, (id, kc)) in row1_codes.iter().enumerate() {
        keys.push(layout_key(id, 1.5 + idx as f32, 1.0, 1.0, 1.0, *kc));
    }
    keys.push(layout_key(
        "KEY_BACKSLASH",
        13.5,
        1.0,
        2.0,
        1.0,
        KeyCode::BACKSLASH,
    ));

    // Row 2 (y=2.0, total 15.5): Caps(2.0) A..L ; ' Return(2.5)
    keys.push(mod_key(
        "KEY_CAPSLOCK",
        0.0,
        2.0,
        2.0,
        1.0,
        ModifierKind::CapsLock,
        "Caps",
        Some(IconId::CapsLock),
    ));
    let row2_codes = [
        ("KEY_A", KeyCode::A),
        ("KEY_S", KeyCode::S),
        ("KEY_D", KeyCode::D),
        ("KEY_F", KeyCode::F),
        ("KEY_G", KeyCode::G),
        ("KEY_H", KeyCode::H),
        ("KEY_J", KeyCode::J),
        ("KEY_K", KeyCode::K),
        ("KEY_L", KeyCode::L),
        ("KEY_SEMICOLON", KeyCode::SEMICOLON),
        ("KEY_APOSTROPHE", KeyCode::APOSTROPHE),
    ];
    for (idx, (id, kc)) in row2_codes.iter().enumerate() {
        keys.push(layout_key(id, 2.0 + idx as f32, 2.0, 1.0, 1.0, *kc));
    }
    keys.push(fixed_action(
        "KEY_ENTER",
        13.0,
        2.0,
        2.5,
        1.0,
        KeyCode::ENTER,
        "Return",
        None,
        Some(IconId::Return),
    ));

    // Row 3 (y=3.0, total 15.5): Shift(2.5) Z..M , . / Shift(3.0)
    keys.push(mod_key(
        "KEY_LEFTSHIFT",
        0.0,
        3.0,
        2.5,
        1.0,
        ModifierKind::Shift,
        "Shift",
        Some(IconId::Shift),
    ));
    let row3_codes = [
        ("KEY_Z", KeyCode::Z),
        ("KEY_X", KeyCode::X),
        ("KEY_C", KeyCode::C),
        ("KEY_V", KeyCode::V),
        ("KEY_B", KeyCode::B),
        ("KEY_N", KeyCode::N),
        ("KEY_M", KeyCode::M),
        ("KEY_COMMA", KeyCode::COMMA),
        ("KEY_DOT", KeyCode::DOT),
        ("KEY_SLASH", KeyCode::SLASH),
    ];
    for (idx, (id, kc)) in row3_codes.iter().enumerate() {
        keys.push(layout_key(id, 2.5 + idx as f32, 3.0, 1.0, 1.0, *kc));
    }
    keys.push(mod_key(
        "KEY_RIGHTSHIFT",
        12.5,
        3.0,
        3.0,
        1.0,
        ModifierKind::Shift,
        "Shift",
        Some(IconId::Shift),
    ));

    // Row 4 (y=4.0, total 15.5): Ctrl(1.5) Super(1.5) Alt(1.5) Space(6.5) AltGr(1.5) + Inverted-T(3.0)
    keys.push(mod_key(
        "KEY_LEFTCTRL",
        0.0,
        4.0,
        1.5,
        1.0,
        ModifierKind::Ctrl,
        "Ctrl",
        None,
    ));
    keys.push(mod_key(
        "KEY_LEFTMETA",
        1.5,
        4.0,
        1.5,
        1.0,
        ModifierKind::Super,
        "Super",
        Some(IconId::SuperKey),
    ));
    keys.push(mod_key(
        "KEY_LEFTALT",
        3.0,
        4.0,
        1.5,
        1.0,
        ModifierKind::Alt,
        "Alt",
        None,
    ));
    keys.push(space_key(4.5, 4.0, 6.5, 1.0));
    keys.push(mod_key(
        "KEY_RIGHTALT",
        11.0,
        4.0,
        1.5,
        1.0,
        ModifierKind::AltGr,
        "AltGr",
        None,
    ));
    push_inverted_t_arrows(&mut keys, 12.5, 4.0);

    PanelDefinition {
        id: PANEL_STANDARD.to_owned(),
        name: "Standard".to_owned(),
        cols: 15.5,
        rows: 5.0,
        keys,
        built_in: true,
    }
}

/// Built-in Starter Panel 2: **Compact TV** (`ABC` or `123` sub-page) with half-height Inverted-T arrows.
#[must_use]
pub fn build_compact_tv_panel(layout: &LayoutDefinition, page: CompactPage) -> PanelDefinition {
    match page {
        CompactPage::Letters => build_compact_tv_alpha(layout),
        CompactPage::SymbolsNav => build_compact_tv_symbols(),
    }
}

fn build_compact_tv_alpha(layout: &LayoutDefinition) -> PanelDefinition {
    let mut keys = Vec::new();
    let r0_len = layout.compact_rows.first().map_or(10, Vec::len);
    let cols = (r0_len as f32 + 2.0).max(12.0);

    // Row 0: top letters + Bksp(1.0) + Del(1.0) or Bksp(2.0)
    if let Some(r0) = layout.compact_rows.first() {
        for (i, code) in r0.iter().enumerate() {
            if let Some(kc) = KeyCode::from_name(code) {
                keys.push(layout_key(code, i as f32, 0.0, 1.0, 1.0, kc));
            }
        }
    }
    let bksp_x = r0_len as f32;
    keys.push(fixed_action(
        "KEY_BACKSPACE",
        bksp_x,
        0.0,
        (cols - bksp_x).max(1.5),
        1.0,
        KeyCode::BACKSPACE,
        "Bksp",
        Some(IconId::Backspace),
        Some(IconId::Backspace),
    ));

    // Row 1: home letters + Return
    let r1_len = layout.compact_rows.get(1).map_or(9, Vec::len);
    if let Some(r1) = layout.compact_rows.get(1) {
        for (i, code) in r1.iter().enumerate() {
            if let Some(kc) = KeyCode::from_name(code) {
                keys.push(layout_key(code, i as f32, 1.0, 1.0, 1.0, kc));
            }
        }
    }
    keys.push(fixed_action(
        "KEY_ENTER",
        r1_len as f32,
        1.0,
        (cols - r1_len as f32).max(1.5),
        1.0,
        KeyCode::ENTER,
        "Return",
        None,
        Some(IconId::Return),
    ));

    // Row 2: Shift(1.5) + bottom letters + Del(remaining)
    keys.push(mod_key(
        "KEY_LEFTSHIFT",
        0.0,
        2.0,
        1.5,
        1.0,
        ModifierKind::Shift,
        "Shift",
        Some(IconId::Shift),
    ));
    let r2_len = layout.compact_rows.get(2).map_or(9, Vec::len);
    if let Some(r2) = layout.compact_rows.get(2) {
        for (i, code) in r2.iter().enumerate() {
            if let Some(kc) = KeyCode::from_name(code) {
                keys.push(layout_key(code, 1.5 + i as f32, 2.0, 1.0, 1.0, kc));
            }
        }
    }
    let del_x = 1.5 + r2_len as f32;
    if cols - del_x >= 0.5 {
        keys.push(fixed_action(
            "KEY_DELETE",
            del_x,
            2.0,
            cols - del_x,
            1.0,
            KeyCode::DELETE,
            "Del",
            None,
            Some(IconId::ForwardDelete),
        ));
    }

    // Row 3: 123(1.5) + AltGr(1.5) + Space + Inverted-T arrows(3.0)
    keys.push(GridKeySpec {
        id: "LAYER_TOGGLE".to_owned(),
        x: 0.0,
        y: 3.0,
        w: 1.5,
        h: 1.0,
        action: GridKeyAction::ToggleCompactPage,
        role: KeyRole::LayerToggle,
        label: "123".to_owned(),
        shift_label: String::new(),
        icon: None,
        symbol_icon: None,
        color_override: None,
    });
    keys.push(mod_key(
        "KEY_RIGHTALT",
        1.5,
        3.0,
        1.5,
        1.0,
        ModifierKind::AltGr,
        "AltGr",
        None,
    ));
    let arrow_x = cols - 3.0;
    keys.push(space_key(3.0, 3.0, (arrow_x - 3.0).max(2.0), 1.0));
    push_inverted_t_arrows(&mut keys, arrow_x, 3.0);

    PanelDefinition {
        id: PANEL_COMPACT_TV.to_owned(),
        name: "Compact TV".to_owned(),
        cols,
        rows: 4.0,
        keys,
        built_in: true,
    }
}

fn build_compact_tv_symbols() -> PanelDefinition {
    let mut keys = Vec::new();
    let cols = 14.0;

    // Row 0: F1..F12 + Del(2.0)
    let fkeys = [
        KeyCode::F1,
        KeyCode::F2,
        KeyCode::F3,
        KeyCode::F4,
        KeyCode::F5,
        KeyCode::F6,
        KeyCode::F7,
        KeyCode::F8,
        KeyCode::F9,
        KeyCode::F10,
        KeyCode::F11,
        KeyCode::F12,
    ];
    for (i, kc) in fkeys.iter().enumerate() {
        keys.push(fixed_action(
            &format!("KEY_F{}", i + 1),
            i as f32,
            0.0,
            1.0,
            1.0,
            *kc,
            kc.short_name(),
            None,
            None,
        ));
    }
    keys.push(fixed_action(
        "KEY_DELETE",
        12.0,
        0.0,
        2.0,
        1.0,
        KeyCode::DELETE,
        "Del",
        None,
        Some(IconId::ForwardDelete),
    ));

    // Row 1: 1..0 - = Bksp(2.0)
    let nums = [
        KeyCode::KEY_1,
        KeyCode::KEY_2,
        KeyCode::KEY_3,
        KeyCode::KEY_4,
        KeyCode::KEY_5,
        KeyCode::KEY_6,
        KeyCode::KEY_7,
        KeyCode::KEY_8,
        KeyCode::KEY_9,
        KeyCode::KEY_0,
        KeyCode::MINUS,
        KeyCode::EQUAL,
    ];
    for (i, kc) in nums.iter().enumerate() {
        keys.push(layout_key(
            &format!("num_{i}"),
            i as f32,
            1.0,
            1.0,
            1.0,
            *kc,
        ));
    }
    keys.push(fixed_action(
        "KEY_BACKSPACE",
        12.0,
        1.0,
        2.0,
        1.0,
        KeyCode::BACKSPACE,
        "Bksp",
        Some(IconId::Backspace),
        Some(IconId::Backspace),
    ));

    // Row 2: Punctuation + Home/End/PgUp/PgDn/Ins
    let syms = [
        ("KEY_GRAVE", KeyCode::GRAVE),
        ("KEY_LEFTBRACE", KeyCode::LEFTBRACE),
        ("KEY_RIGHTBRACE", KeyCode::RIGHTBRACE),
        ("KEY_BACKSLASH", KeyCode::BACKSLASH),
        ("KEY_SEMICOLON", KeyCode::SEMICOLON),
        ("KEY_APOSTROPHE", KeyCode::APOSTROPHE),
        ("KEY_COMMA", KeyCode::COMMA),
        ("KEY_DOT", KeyCode::DOT),
        ("KEY_SLASH", KeyCode::SLASH),
    ];
    for (i, (id, kc)) in syms.iter().enumerate() {
        keys.push(layout_key(id, i as f32, 2.0, 1.0, 1.0, *kc));
    }
    keys.push(fixed_action(
        "KEY_HOME",
        9.0,
        2.0,
        1.0,
        1.0,
        KeyCode::HOME,
        "Home",
        None,
        None,
    ));
    keys.push(fixed_action(
        "KEY_END",
        10.0,
        2.0,
        1.0,
        1.0,
        KeyCode::END,
        "End",
        None,
        None,
    ));
    keys.push(fixed_action(
        "KEY_PAGEUP",
        11.0,
        2.0,
        1.0,
        1.0,
        KeyCode::PAGEUP,
        "PgUp",
        None,
        None,
    ));
    keys.push(fixed_action(
        "KEY_PAGEDOWN",
        12.0,
        2.0,
        1.0,
        1.0,
        KeyCode::PAGEDOWN,
        "PgDn",
        None,
        None,
    ));
    keys.push(fixed_action(
        "KEY_INSERT",
        13.0,
        2.0,
        1.0,
        1.0,
        KeyCode::INSERT,
        "Ins",
        None,
        None,
    ));

    // Row 3: ABC(1.5) Tab(1.0) Ctrl(1.5) Super(1.5) Alt(1.0) Space(3.0) Inverted-T(3.0) Return(1.5)
    keys.push(GridKeySpec {
        id: "LAYER_TOGGLE".to_owned(),
        x: 0.0,
        y: 3.0,
        w: 1.5,
        h: 1.0,
        action: GridKeyAction::ToggleCompactPage,
        role: KeyRole::LayerToggle,
        label: "ABC".to_owned(),
        shift_label: String::new(),
        icon: None,
        symbol_icon: None,
        color_override: None,
    });
    keys.push(fixed_action(
        "KEY_TAB",
        1.5,
        3.0,
        1.0,
        1.0,
        KeyCode::TAB,
        "Tab",
        None,
        Some(IconId::Tab),
    ));
    keys.push(mod_key(
        "KEY_LEFTCTRL",
        2.5,
        3.0,
        1.5,
        1.0,
        ModifierKind::Ctrl,
        "Ctrl",
        None,
    ));
    keys.push(mod_key(
        "KEY_LEFTMETA",
        4.0,
        3.0,
        1.5,
        1.0,
        ModifierKind::Super,
        "Super",
        Some(IconId::SuperKey),
    ));
    keys.push(mod_key(
        "KEY_LEFTALT",
        5.5,
        3.0,
        1.0,
        1.0,
        ModifierKind::Alt,
        "Alt",
        None,
    ));
    keys.push(space_key(6.5, 3.0, 3.0, 1.0));
    push_inverted_t_arrows(&mut keys, 9.5, 3.0);
    keys.push(fixed_action(
        "KEY_ENTER",
        12.5,
        3.0,
        1.5,
        1.0,
        KeyCode::ENTER,
        "Return",
        None,
        Some(IconId::Return),
    ));

    PanelDefinition {
        id: PANEL_COMPACT_TV.to_owned(),
        name: "Compact TV".to_owned(),
        cols,
        rows: 4.0,
        keys,
        built_in: true,
    }
}

/// Built-in Starter Panel 3: **Numpad & Nav** (`9.5 × 5.0` grid).
#[must_use]
pub fn build_numpad_nav_panel() -> PanelDefinition {
    let mut keys = vec![
        // Left navigation & editing cluster (cols 0..4.5)
        fixed_action(
            "np_ins",
            0.0,
            0.0,
            1.5,
            1.0,
            KeyCode::INSERT,
            "Ins",
            None,
            None,
        ),
        fixed_action(
            "np_home",
            1.5,
            0.0,
            1.5,
            1.0,
            KeyCode::HOME,
            "Home",
            None,
            None,
        ),
        fixed_action(
            "np_pgup",
            3.0,
            0.0,
            1.5,
            1.0,
            KeyCode::PAGEUP,
            "PgUp",
            None,
            None,
        ),
        fixed_action(
            "np_del",
            0.0,
            1.0,
            1.5,
            1.0,
            KeyCode::DELETE,
            "Del",
            None,
            Some(IconId::ForwardDelete),
        ),
        fixed_action(
            "np_end",
            1.5,
            1.0,
            1.5,
            1.0,
            KeyCode::END,
            "End",
            None,
            None,
        ),
        fixed_action(
            "np_pgdn",
            3.0,
            1.0,
            1.5,
            1.0,
            KeyCode::PAGEDOWN,
            "PgDn",
            None,
            None,
        ),
        shortcut_key(
            "np_copy",
            0.0,
            2.0,
            1.5,
            1.0,
            "Copy",
            "Ctrl+C",
            vec![ModifierKind::Ctrl],
            KeyCode::C,
        ),
        shortcut_key(
            "np_paste",
            1.5,
            2.0,
            1.5,
            1.0,
            "Paste",
            "Ctrl+V",
            vec![ModifierKind::Ctrl],
            KeyCode::V,
        ),
        fixed_action(
            "np_bksp",
            3.0,
            2.0,
            1.5,
            1.0,
            KeyCode::BACKSPACE,
            "Bksp",
            Some(IconId::Backspace),
            Some(IconId::Backspace),
        ),
        shortcut_key(
            "np_undo",
            0.0,
            3.0,
            1.5,
            1.0,
            "Undo",
            "Ctrl+Z",
            vec![ModifierKind::Ctrl],
            KeyCode::Z,
        ),
        shortcut_key(
            "np_redo",
            1.5,
            3.0,
            1.5,
            1.0,
            "Redo",
            "Ctrl+Y",
            vec![ModifierKind::Ctrl],
            KeyCode::Y,
        ),
        shortcut_key(
            "np_cut",
            3.0,
            3.0,
            1.5,
            1.0,
            "Cut",
            "Ctrl+X",
            vec![ModifierKind::Ctrl],
            KeyCode::X,
        ),
        // Return to Standard keyboard button
        GridKeySpec {
            id: "np_back_std".to_owned(),
            x: 0.0,
            y: 4.0,
            w: 1.5,
            h: 1.0,
            action: GridKeyAction::SwitchPanel {
                panel_id: PANEL_STANDARD.to_owned(),
            },
            role: KeyRole::LayerToggle,
            label: "ABC".to_owned(),
            shift_label: String::new(),
            icon: None,
            symbol_icon: None,
            color_override: None,
        },
        // Right Numpad (cols 5.0..9.5)
        fixed_action(
            "kp_num",
            5.0,
            0.0,
            1.0,
            1.0,
            KeyCode::NUMLOCK,
            "Num",
            None,
            None,
        ),
        fixed_action(
            "kp_div",
            6.0,
            0.0,
            1.0,
            1.0,
            KeyCode::KPSLASH,
            "/",
            None,
            None,
        ),
        fixed_action(
            "kp_mul",
            7.0,
            0.0,
            1.0,
            1.0,
            KeyCode::KPASTERISK,
            "*",
            None,
            None,
        ),
        fixed_action(
            "kp_sub",
            8.0,
            0.0,
            1.5,
            1.0,
            KeyCode::KPMINUS,
            "-",
            None,
            None,
        ),
        fixed_action("kp_7", 5.0, 1.0, 1.0, 1.0, KeyCode::KP7, "7", None, None),
        fixed_action("kp_8", 6.0, 1.0, 1.0, 1.0, KeyCode::KP8, "8", None, None),
        fixed_action("kp_9", 7.0, 1.0, 1.0, 1.0, KeyCode::KP9, "9", None, None),
        fixed_action(
            "kp_add",
            8.0,
            1.0,
            1.5,
            2.0,
            KeyCode::KPPLUS,
            "+",
            None,
            None,
        ),
        fixed_action("kp_4", 5.0, 2.0, 1.0, 1.0, KeyCode::KP4, "4", None, None),
        fixed_action("kp_5", 6.0, 2.0, 1.0, 1.0, KeyCode::KP5, "5", None, None),
        fixed_action("kp_6", 7.0, 2.0, 1.0, 1.0, KeyCode::KP6, "6", None, None),
        fixed_action("kp_1", 5.0, 3.0, 1.0, 1.0, KeyCode::KP1, "1", None, None),
        fixed_action("kp_2", 6.0, 3.0, 1.0, 1.0, KeyCode::KP2, "2", None, None),
        fixed_action("kp_3", 7.0, 3.0, 1.0, 1.0, KeyCode::KP3, "3", None, None),
        fixed_action(
            "kp_enter",
            8.0,
            3.0,
            1.5,
            2.0,
            KeyCode::KPENTER,
            "Enter",
            None,
            Some(IconId::Return),
        ),
        fixed_action("kp_0", 5.0, 4.0, 2.0, 1.0, KeyCode::KP0, "0", None, None),
        fixed_action(
            "kp_dot",
            7.0,
            4.0,
            1.0,
            1.0,
            KeyCode::KPDOT,
            ".",
            None,
            None,
        ),
    ];

    // Half-height Inverted-T arrow cluster flush with bottom row (cols 1.5..4.5, rows 4.0..5.0)
    push_inverted_t_arrows(&mut keys, 1.5, 4.0);

    PanelDefinition {
        id: PANEL_NUMPAD_NAV.to_owned(),
        name: "Numpad & Nav".to_owned(),
        cols: 9.5,
        rows: 5.0,
        keys,
        built_in: true,
    }
}

/// Built-in Starter Panel 4: **Shortcuts & Media** (`10.0 × 4.0` grid).
#[must_use]
pub fn build_shortcuts_media_panel() -> PanelDefinition {
    let keys = vec![
        // Row 0: Core editing shortcuts
        shortcut_key(
            "sc_undo",
            0.0,
            0.0,
            2.0,
            1.0,
            "Undo",
            "Ctrl+Z",
            vec![ModifierKind::Ctrl],
            KeyCode::Z,
        ),
        shortcut_key(
            "sc_redo",
            2.0,
            0.0,
            2.0,
            1.0,
            "Redo",
            "Ctrl+Y",
            vec![ModifierKind::Ctrl],
            KeyCode::Y,
        ),
        shortcut_key(
            "sc_cut",
            4.0,
            0.0,
            2.0,
            1.0,
            "Cut",
            "Ctrl+X",
            vec![ModifierKind::Ctrl],
            KeyCode::X,
        ),
        shortcut_key(
            "sc_copy",
            6.0,
            0.0,
            2.0,
            1.0,
            "Copy",
            "Ctrl+C",
            vec![ModifierKind::Ctrl],
            KeyCode::C,
        ),
        shortcut_key(
            "sc_paste",
            8.0,
            0.0,
            2.0,
            1.0,
            "Paste",
            "Ctrl+V",
            vec![ModifierKind::Ctrl],
            KeyCode::V,
        ),
        // Row 1: Browser & Document shortcuts
        shortcut_key(
            "sc_all",
            0.0,
            1.0,
            2.0,
            1.0,
            "Select All",
            "Ctrl+A",
            vec![ModifierKind::Ctrl],
            KeyCode::A,
        ),
        shortcut_key(
            "sc_find",
            2.0,
            1.0,
            2.0,
            1.0,
            "Find",
            "Ctrl+F",
            vec![ModifierKind::Ctrl],
            KeyCode::F,
        ),
        shortcut_key(
            "sc_save",
            4.0,
            1.0,
            2.0,
            1.0,
            "Save",
            "Ctrl+S",
            vec![ModifierKind::Ctrl],
            KeyCode::S,
        ),
        shortcut_key(
            "sc_newtab",
            6.0,
            1.0,
            2.0,
            1.0,
            "New Tab",
            "Ctrl+T",
            vec![ModifierKind::Ctrl],
            KeyCode::T,
        ),
        shortcut_key(
            "sc_closetab",
            8.0,
            1.0,
            2.0,
            1.0,
            "Close Tab",
            "Ctrl+W",
            vec![ModifierKind::Ctrl],
            KeyCode::W,
        ),
        // Row 2: Window & Editing actions
        shortcut_key(
            "sc_alttab",
            0.0,
            2.0,
            2.5,
            1.0,
            "Switch App",
            "Alt+Tab",
            vec![ModifierKind::Alt],
            KeyCode::TAB,
        ),
        shortcut_key(
            "sc_desktop",
            2.5,
            2.0,
            2.5,
            1.0,
            "Desktop",
            "Super+D",
            vec![ModifierKind::Super],
            KeyCode::D,
        ),
        fixed_action(
            "sc_del",
            5.0,
            2.0,
            1.5,
            1.0,
            KeyCode::DELETE,
            "Del",
            None,
            Some(IconId::ForwardDelete),
        ),
        fixed_action(
            "sc_bksp",
            6.5,
            2.0,
            1.5,
            1.0,
            KeyCode::BACKSPACE,
            "Bksp",
            Some(IconId::Backspace),
            Some(IconId::Backspace),
        ),
        fixed_action(
            "sc_enter",
            8.0,
            2.0,
            2.0,
            1.0,
            KeyCode::ENTER,
            "Return",
            None,
            Some(IconId::Return),
        ),
        // Row 3: Uniform 1.0u Back-to-ABC + 9 Media & System controls
        GridKeySpec {
            id: "sc_back_std".to_owned(),
            x: 0.0,
            y: 3.0,
            w: 1.0,
            h: 1.0,
            action: GridKeyAction::SwitchPanel {
                panel_id: PANEL_STANDARD.to_owned(),
            },
            role: KeyRole::LayerToggle,
            label: "ABC".to_owned(),
            shift_label: String::new(),
            icon: None,
            symbol_icon: None,
            color_override: None,
        },
        media_key(
            "sc_bri_dn",
            1.0,
            3.0,
            1.0,
            1.0,
            KeyCode::BRIGHTNESSDOWN,
            "Bri-",
            IconId::BrightnessDown,
        ),
        media_key(
            "sc_bri_up",
            2.0,
            3.0,
            1.0,
            1.0,
            KeyCode::BRIGHTNESSUP,
            "Bri+",
            IconId::BrightnessUp,
        ),
        media_key(
            "sc_prev",
            3.0,
            3.0,
            1.0,
            1.0,
            KeyCode::PREVIOUSSONG,
            "Prev",
            IconId::MediaPrev,
        ),
        media_key(
            "sc_play",
            4.0,
            3.0,
            1.0,
            1.0,
            KeyCode::PLAYPAUSE,
            "Play",
            IconId::MediaPlayPause,
        ),
        media_key(
            "sc_next",
            5.0,
            3.0,
            1.0,
            1.0,
            KeyCode::NEXTSONG,
            "Next",
            IconId::MediaNext,
        ),
        media_key(
            "sc_mute",
            6.0,
            3.0,
            1.0,
            1.0,
            KeyCode::MUTE,
            "Mute",
            IconId::AudioMute,
        ),
        media_key(
            "sc_voldn",
            7.0,
            3.0,
            1.0,
            1.0,
            KeyCode::VOLUMEDOWN,
            "Vol-",
            IconId::AudioLow,
        ),
        media_key(
            "sc_volup",
            8.0,
            3.0,
            1.0,
            1.0,
            KeyCode::VOLUMEUP,
            "Vol+",
            IconId::AudioHigh,
        ),
        media_key(
            "sc_shot",
            9.0,
            3.0,
            1.0,
            1.0,
            KeyCode::SYSRQ,
            "Shot",
            IconId::Screenshot,
        ),
    ];

    PanelDefinition {
        id: PANEL_SHORTCUTS_MEDIA.to_owned(),
        name: "Shortcuts & Media".to_owned(),
        cols: 10.0,
        rows: 4.0,
        keys,
        built_in: true,
    }
}

/// Manages built-in starter panels and user-customized JSON panels in `~/.config/wlboard/panels/`.
#[derive(Debug, Clone)]
pub struct PanelStore {
    dir: PathBuf,
}

impl PanelStore {
    #[must_use]
    pub fn default_store() -> Self {
        Self {
            dir: AppConfig::panels_dir(),
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

    /// Return all 4 built-in starter panels in default order.
    #[must_use]
    pub fn builtin_panels(default_layout: &LayoutDefinition) -> Vec<PanelDefinition> {
        vec![
            build_standard_panel(),
            build_compact_tv_panel(default_layout, CompactPage::Letters),
            build_numpad_nav_panel(),
            build_shortcuts_media_panel(),
        ]
    }

    /// Load all panels: built-in defaults (with user disk overrides merged by `id`) + custom user panels.
    #[must_use]
    pub fn load_all(&self, default_layout: &LayoutDefinition) -> Vec<PanelDefinition> {
        let mut by_id: HashMap<String, PanelDefinition> = HashMap::new();
        let mut order: Vec<String> = Vec::new();

        for panel in Self::builtin_panels(default_layout) {
            order.push(panel.id.clone());
            by_id.insert(panel.id.clone(), panel);
        }

        if self.dir.exists()
            && let Ok(entries) = fs::read_dir(&self.dir)
        {
            let mut custom: Vec<PanelDefinition> = Vec::new();
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("json")
                    && let Ok(raw) = fs::read_to_string(&path)
                    && let Ok(mut panel) = serde_json::from_str::<PanelDefinition>(&raw)
                {
                    panel.sanitize();
                    if by_id.contains_key(&panel.id) {
                        panel.built_in = true;
                        by_id.insert(panel.id.clone(), panel);
                    } else {
                        custom.push(panel);
                    }
                }
            }
            custom.sort_by_key(|p| p.name.to_lowercase());
            for p in custom {
                order.push(p.id.clone());
                by_id.insert(p.id.clone(), p);
            }
        }

        order
            .into_iter()
            .filter_map(|id| by_id.remove(&id))
            .collect()
    }

    /// Save (create or overwrite) a panel JSON atomically.
    pub fn save(&self, panel: &PanelDefinition) -> Result<PanelDefinition, PanelError> {
        let mut clean = panel.clone();
        clean.name = clean.name.trim().to_owned();
        if clean.name.is_empty() {
            return Err(PanelError::EmptyName);
        }
        if clean.id.trim().is_empty() {
            clean.id = PresetStore::sanitize_slug(&clean.name);
        }
        let slug = PresetStore::sanitize_slug(&clean.id);
        if slug.is_empty() {
            return Err(PanelError::EmptyName);
        }
        clean.id = slug.clone();
        clean.sanitize();

        fs::create_dir_all(&self.dir)?;
        let path = self.dir.join(format!("{slug}.json"));
        let serialized = serde_json::to_string_pretty(&clean)?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, serialized)?;
        fs::rename(&tmp, &path)?;
        Ok(clean)
    }

    /// Delete a saved panel JSON override or custom panel from disk.
    pub fn delete(&self, id: &str) -> Result<(), PanelError> {
        let slug = PresetStore::sanitize_slug(id);
        let path = self.dir.join(format!("{slug}.json"));
        if path.exists() {
            fs::remove_file(path)?;
            Ok(())
        } else {
            Err(PanelError::NotFound(id.to_owned()))
        }
    }

    /// True if a built-in panel has a saved user override on disk.
    #[must_use]
    pub fn has_override(&self, id: &str) -> bool {
        let slug = PresetStore::sanitize_slug(id);
        self.dir.join(format!("{slug}.json")).exists()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::LayoutRegistry;

    #[test]
    fn standard_panel_has_del_and_inverted_t_half_height_arrows() {
        let std = build_standard_panel();
        assert!((std.cols - 15.5).abs() < f32::EPSILON);
        assert!((std.rows - 5.0).abs() < f32::EPSILON);

        let del = std
            .keys
            .iter()
            .find(|k| k.id == "KEY_DELETE")
            .expect("Standard panel must include Delete key");
        assert!((del.y - 0.0).abs() < f32::EPSILON);

        let up = std
            .keys
            .iter()
            .find(|k| k.id == "KEY_UP")
            .expect("Up arrow");
        let down = std
            .keys
            .iter()
            .find(|k| k.id == "KEY_DOWN")
            .expect("Down arrow");
        let left = std
            .keys
            .iter()
            .find(|k| k.id == "KEY_LEFT")
            .expect("Left arrow");
        let right = std
            .keys
            .iter()
            .find(|k| k.id == "KEY_RIGHT")
            .expect("Right arrow");

        // Up sits directly above Down with identical x and half-height (0.5u)
        assert!((up.x - down.x).abs() < f32::EPSILON);
        assert!((up.h - 0.5).abs() < f32::EPSILON);
        assert!((down.h - 0.5).abs() < f32::EPSILON);
        assert!((down.y - (up.y + 0.5)).abs() < f32::EPSILON);
        assert!(left.x < down.x && right.x > down.x);
    }

    #[test]
    fn panel_store_saves_loads_and_resets_custom_panels() {
        let dir = std::env::temp_dir().join(format!("wlboard_panels_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let store = PanelStore::with_dir(&dir);
        let reg = LayoutRegistry::new();

        let initial = store.load_all(reg.get("en"));
        assert_eq!(initial.len(), 4);

        let custom = PanelDefinition {
            id: "my-macros".to_owned(),
            name: "My Macros".to_owned(),
            cols: 6.0,
            rows: 2.0,
            keys: vec![GridKeySpec {
                id: "hello".to_owned(),
                x: 0.0,
                y: 0.0,
                w: 3.0,
                h: 1.0,
                action: GridKeyAction::TextSnippet {
                    text: "Hello!".to_owned(),
                },
                role: KeyRole::Action,
                label: "Greet".to_owned(),
                shift_label: String::new(),
                icon: None,
                symbol_icon: None,
                color_override: None,
            }],
            built_in: false,
        };
        store.save(&custom).expect("save custom panel");
        let loaded = store.load_all(reg.get("en"));
        assert_eq!(loaded.len(), 5);
        assert!(loaded.iter().any(|p| p.id == "my-macros"));

        store.delete("my-macros").expect("delete custom panel");
        assert_eq!(store.load_all(reg.get("en")).len(), 4);
        let _ = fs::remove_dir_all(&dir);
    }
}
