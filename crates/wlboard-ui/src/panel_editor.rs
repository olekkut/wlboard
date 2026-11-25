//! Dedicated 3-pane visual **Panel Editor** (`960×590` resizable `xdg_toplevel` window)
//! with a 0.5-unit snap-grid canvas, drag-to-move, drag-to-resize, and full Key Inspector.

use std::path::PathBuf;

use tiny_skia::Pixmap;
use wlboard_core::{
    ACCENT_SWATCHES, Color, CompactPage, GridKeyAction, GridKeySpec, IconId, KeyCode, KeyRole,
    KeyboardState, KeycapStyle, ModifierKind, PANEL_COMPACT_TV, PANEL_NUMPAD_NAV,
    PANEL_SHORTCUTS_MEDIA, PANEL_STANDARD, PanelDefinition, PanelStore, Point, Rect,
    ResolvedGridKey, build_compact_tv_panel, build_numpad_nav_panel, build_shortcuts_media_panel,
    build_standard_panel,
};

use crate::canvas::{TextAlign, UiRenderer};
use crate::settings_view::evdev_to_ascii;

pub const PANEL_EDITOR_DEFAULT_WIDTH: u32 = 960;
pub const PANEL_EDITOR_DEFAULT_HEIGHT: u32 = 590;

/// Ordered picker keycode table cycled in the Key Inspector.
const INSPECTOR_KEYCODES: &[KeyCode] = &[
    KeyCode::A,
    KeyCode::B,
    KeyCode::C,
    KeyCode::D,
    KeyCode::E,
    KeyCode::F,
    KeyCode::G,
    KeyCode::H,
    KeyCode::I,
    KeyCode::J,
    KeyCode::K,
    KeyCode::L,
    KeyCode::M,
    KeyCode::N,
    KeyCode::O,
    KeyCode::P,
    KeyCode::Q,
    KeyCode::R,
    KeyCode::S,
    KeyCode::T,
    KeyCode::U,
    KeyCode::V,
    KeyCode::W,
    KeyCode::X,
    KeyCode::Y,
    KeyCode::Z,
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
    KeyCode::SPACE,
    KeyCode::ENTER,
    KeyCode::BACKSPACE,
    KeyCode::DELETE,
    KeyCode::TAB,
    KeyCode::ESC,
    KeyCode::INSERT,
    KeyCode::HOME,
    KeyCode::END,
    KeyCode::PAGEUP,
    KeyCode::PAGEDOWN,
    KeyCode::UP,
    KeyCode::DOWN,
    KeyCode::LEFT,
    KeyCode::RIGHT,
    KeyCode::LEFTSHIFT,
    KeyCode::CAPSLOCK,
    KeyCode::LEFTCTRL,
    KeyCode::LEFTALT,
    KeyCode::RIGHTALT,
    KeyCode::LEFTMETA,
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
    KeyCode::BRIGHTNESSDOWN,
    KeyCode::BRIGHTNESSUP,
    KeyCode::PREVIOUSSONG,
    KeyCode::PLAYPAUSE,
    KeyCode::NEXTSONG,
    KeyCode::MUTE,
    KeyCode::VOLUMEDOWN,
    KeyCode::VOLUMEUP,
    KeyCode::SYSRQ,
];

/// Inline text inputs inside the Panel Editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorField {
    PanelName,
    KeyLabel,
    KeyShiftLabel,
    SnippetText,
}

/// Actions emitted by `PanelEditorView` back to the runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanelEditorAction {
    None,
    Redraw,
    /// User pressed an empty header or chrome region — initiate native compositor window move.
    StartWindowMove,
    PanelsChanged,
    CopyText(String),
    OpenFolder(PathBuf),
    CloseEditor,
}

#[derive(Debug, Clone, Copy)]
enum CanvasDrag {
    Move {
        key_idx: usize,
        offset_cols: f32,
        offset_rows: f32,
    },
    Resize {
        key_idx: usize,
    },
}

#[derive(Debug, Clone)]
enum EditorHit {
    CloseButton,
    AddKey,
    DeleteKey,
    ResetBuiltin,
    OpenPanelsFolder,
    NewPanel,
    DuplicatePanel,
    DeletePanel,
    SelectPanel(String),
    PanelNameInput,
    GridColsStep(f32),
    GridRowsStep(f32),
    CanvasKey(usize),
    CanvasKeyResize(usize),
    KeyLabelInput,
    KeyShiftInput,
    SnippetInput,
    ActionKind(u8),
    KeycodeStep(isize),
    ShortcutModToggle(ModifierKind),
    TargetPanelSelect(String),
    GeomStep { field: u8, delta: f32 },
    RoleSelect(KeyRole),
    ColorOverride(Option<Color>),
}

/// State & renderer for the built-in 3-pane visual **Panel Editor** window.
#[derive(Debug, Clone)]
pub struct PanelEditorView {
    pub width: f32,
    pub height: f32,
    pub editing_panel: PanelDefinition,
    pub selected_key_idx: Option<usize>,
    pub focused_field: Option<EditorField>,
    pub selection_all: bool,
    pub status_message: Option<String>,
    canvas_rect: Rect,
    drag: Option<CanvasDrag>,
    hits: Vec<(Rect, EditorHit)>,
}

impl PanelEditorView {
    #[must_use]
    pub fn new(state: &KeyboardState) -> Self {
        let panel = state.active_panel_def();
        let selected = if panel.keys.is_empty() { None } else { Some(0) };
        Self {
            width: PANEL_EDITOR_DEFAULT_WIDTH as f32,
            height: PANEL_EDITOR_DEFAULT_HEIGHT as f32,
            editing_panel: panel,
            selected_key_idx: selected,
            focused_field: None,
            selection_all: false,
            status_message: None,
            canvas_rect: Rect::default(),
            drag: None,
            hits: Vec::new(),
        }
    }

    pub fn load_panel(&mut self, panel: PanelDefinition) {
        self.selected_key_idx = if panel.keys.is_empty() { None } else { Some(0) };
        self.editing_panel = panel;
        self.focused_field = None;
        self.selection_all = false;
        self.drag = None;
    }

    fn save_current(
        &mut self,
        state: &mut KeyboardState,
        panel_store: &PanelStore,
    ) -> PanelEditorAction {
        self.editing_panel.sanitize();
        match panel_store.save(&self.editing_panel) {
            Ok(saved) => {
                self.editing_panel = saved.clone();
                state.reload_panels(panel_store);
                state.set_panel(&saved.id);
                PanelEditorAction::PanelsChanged
            }
            Err(err) => {
                self.status_message = Some(format!("Save error: {err}"));
                PanelEditorAction::Redraw
            }
        }
    }

    /// Returns true if `p` hovers an empty header area suitable for showing a grab cursor.
    #[must_use]
    pub fn is_header_drag_area(&self, p: Point) -> bool {
        p.y <= 52.0 && !self.hits.iter().any(|(r, _)| r.contains(p))
    }

    /// Handle pointer press inside the Panel Editor window.
    pub fn pointer_down(
        &mut self,
        p: Point,
        state: &mut KeyboardState,
        panel_store: &PanelStore,
    ) -> PanelEditorAction {
        let hit = self
            .hits
            .iter()
            .rev()
            .find(|(r, _)| r.contains(p))
            .map(|(_, h)| h.clone());

        let Some(hit) = hit else {
            self.focused_field = None;
            self.selection_all = false;
            if !self.canvas_rect.contains(p) {
                return PanelEditorAction::StartWindowMove;
            }
            return PanelEditorAction::Redraw;
        };
        self.status_message = None;
        self.selection_all = false;

        match hit {
            EditorHit::CloseButton => PanelEditorAction::CloseEditor,
            EditorHit::OpenPanelsFolder => {
                self.status_message = Some(format!("Opened {}", panel_store.dir().display()));
                PanelEditorAction::OpenFolder(panel_store.dir().to_path_buf())
            }
            EditorHit::SelectPanel(id) => {
                if let Some(found) = state.panels.iter().find(|p| p.id == id).cloned() {
                    let resolved =
                        if found.id == PANEL_COMPACT_TV && !panel_store.has_override(&found.id) {
                            build_compact_tv_panel(state.active_layout(), CompactPage::Letters)
                        } else {
                            found
                        };
                    self.load_panel(resolved);
                    state.set_panel(&id);
                    return PanelEditorAction::PanelsChanged;
                }
                PanelEditorAction::Redraw
            }
            EditorHit::NewPanel => {
                let mut idx = 1;
                let (id, name) = loop {
                    let cand_id = format!("custom-panel-{idx}");
                    if !state.panels.iter().any(|p| p.id == cand_id) {
                        break (cand_id, format!("Custom Panel {idx}"));
                    }
                    idx += 1;
                };
                let mut panel = PanelDefinition {
                    id,
                    name,
                    cols: 10.0,
                    rows: 4.0,
                    keys: vec![
                        GridKeySpec {
                            id: "custom_1".to_owned(),
                            x: 0.0,
                            y: 0.0,
                            w: 2.0,
                            h: 1.0,
                            action: GridKeyAction::Shortcut {
                                modifiers: vec![ModifierKind::Ctrl],
                                keycode: KeyCode::C,
                            },
                            role: KeyRole::Action,
                            label: "Copy".to_owned(),
                            shift_label: "Ctrl+C".to_owned(),
                            icon: None,
                            symbol_icon: None,
                            color_override: None,
                        },
                        GridKeySpec {
                            id: "custom_2".to_owned(),
                            x: 2.0,
                            y: 0.0,
                            w: 2.0,
                            h: 1.0,
                            action: GridKeyAction::Shortcut {
                                modifiers: vec![ModifierKind::Ctrl],
                                keycode: KeyCode::V,
                            },
                            role: KeyRole::Action,
                            label: "Paste".to_owned(),
                            shift_label: "Ctrl+V".to_owned(),
                            icon: None,
                            symbol_icon: None,
                            color_override: None,
                        },
                        GridKeySpec {
                            id: "custom_back".to_owned(),
                            x: 0.0,
                            y: 3.0,
                            w: 2.0,
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
                    ],
                    built_in: false,
                };
                panel.sanitize();
                self.load_panel(panel);
                self.status_message = Some("Created new custom panel".to_owned());
                self.save_current(state, panel_store)
            }
            EditorHit::DuplicatePanel => {
                let mut dup = self.editing_panel.clone();
                dup.built_in = false;
                dup.name = format!("{} Copy", dup.name);
                dup.id = format!("{}-copy-{}", dup.id, state.panels.len() + 1);
                self.load_panel(dup);
                self.status_message = Some("Duplicated panel".to_owned());
                self.save_current(state, panel_store)
            }
            EditorHit::DeletePanel => {
                if self.editing_panel.built_in {
                    self.status_message =
                        Some("Built-in panels can be reset instead of deleted".to_owned());
                    return PanelEditorAction::Redraw;
                }
                let del_id = self.editing_panel.id.clone();
                let _ = panel_store.delete(&del_id);
                state.reload_panels(panel_store);
                state.set_panel(PANEL_STANDARD);
                self.load_panel(state.active_panel_def());
                self.status_message = Some("Deleted custom panel".to_owned());
                PanelEditorAction::PanelsChanged
            }
            EditorHit::ResetBuiltin => {
                let id = self.editing_panel.id.clone();
                let _ = panel_store.delete(&id);
                let fresh = match id.as_str() {
                    PANEL_COMPACT_TV => {
                        build_compact_tv_panel(state.active_layout(), CompactPage::Letters)
                    }
                    PANEL_NUMPAD_NAV => build_numpad_nav_panel(),
                    PANEL_SHORTCUTS_MEDIA => build_shortcuts_media_panel(),
                    _ => build_standard_panel(),
                };
                state.reload_panels(panel_store);
                self.load_panel(fresh);
                self.status_message = Some("Restored built-in panel default".to_owned());
                PanelEditorAction::PanelsChanged
            }
            EditorHit::AddKey => {
                let (fx, fy) = self.editing_panel.find_free_slot(1.0, 1.0);
                let new_id = self.editing_panel.next_key_id();
                self.editing_panel.keys.push(GridKeySpec {
                    id: new_id,
                    x: fx,
                    y: fy,
                    w: 1.0,
                    h: 1.0,
                    action: GridKeyAction::FixedKey {
                        keycode: KeyCode::DELETE,
                    },
                    role: KeyRole::Action,
                    label: "Del".to_owned(),
                    shift_label: String::new(),
                    icon: None,
                    symbol_icon: Some(IconId::ForwardDelete),
                    color_override: None,
                });
                self.selected_key_idx = Some(self.editing_panel.keys.len() - 1);
                self.status_message = Some("Added new key on 0.5u grid".to_owned());
                self.save_current(state, panel_store)
            }
            EditorHit::DeleteKey => {
                if let Some(idx) = self.selected_key_idx
                    && idx < self.editing_panel.keys.len()
                {
                    self.editing_panel.keys.remove(idx);
                    self.selected_key_idx = if self.editing_panel.keys.is_empty() {
                        None
                    } else {
                        Some(idx.min(self.editing_panel.keys.len() - 1))
                    };
                    self.status_message = Some("Removed selected key".to_owned());
                    return self.save_current(state, panel_store);
                }
                PanelEditorAction::Redraw
            }
            EditorHit::PanelNameInput => {
                self.focused_field = Some(EditorField::PanelName);
                PanelEditorAction::Redraw
            }
            EditorHit::GridColsStep(delta) => {
                self.editing_panel.cols =
                    PanelDefinition::snap_half(self.editing_panel.cols + delta).clamp(4.0, 24.0);
                self.save_current(state, panel_store)
            }
            EditorHit::GridRowsStep(delta) => {
                self.editing_panel.rows =
                    PanelDefinition::snap_half(self.editing_panel.rows + delta).clamp(2.0, 10.0);
                self.save_current(state, panel_store)
            }
            EditorHit::CanvasKeyResize(idx) => {
                self.selected_key_idx = Some(idx);
                self.focused_field = None;
                self.drag = Some(CanvasDrag::Resize { key_idx: idx });
                PanelEditorAction::Redraw
            }
            EditorHit::CanvasKey(idx) => {
                self.selected_key_idx = Some(idx);
                self.focused_field = None;
                if let Some(k) = self.editing_panel.keys.get(idx) {
                    let gx = ((p.x - self.canvas_rect.x) / self.canvas_rect.w.max(1.0))
                        * self.editing_panel.cols;
                    let gy = ((p.y - self.canvas_rect.y) / self.canvas_rect.h.max(1.0))
                        * self.editing_panel.rows;
                    self.drag = Some(CanvasDrag::Move {
                        key_idx: idx,
                        offset_cols: gx - k.x,
                        offset_rows: gy - k.y,
                    });
                }
                PanelEditorAction::Redraw
            }
            EditorHit::KeyLabelInput => {
                self.focused_field = Some(EditorField::KeyLabel);
                PanelEditorAction::Redraw
            }
            EditorHit::KeyShiftInput => {
                self.focused_field = Some(EditorField::KeyShiftLabel);
                PanelEditorAction::Redraw
            }
            EditorHit::SnippetInput => {
                self.focused_field = Some(EditorField::SnippetText);
                PanelEditorAction::Redraw
            }
            EditorHit::ActionKind(kind) => {
                if let Some(idx) = self.selected_key_idx
                    && let Some(k) = self.editing_panel.keys.get_mut(idx)
                {
                    k.action = match kind {
                        0 => GridKeyAction::LayoutKey {
                            keycode: KeyCode::A,
                        },
                        1 => GridKeyAction::FixedKey {
                            keycode: KeyCode::DELETE,
                        },
                        2 => GridKeyAction::Shortcut {
                            modifiers: vec![ModifierKind::Ctrl],
                            keycode: KeyCode::C,
                        },
                        3 => GridKeyAction::TextSnippet {
                            text: "Hello!".to_owned(),
                        },
                        _ => GridKeyAction::SwitchPanel {
                            panel_id: PANEL_STANDARD.to_owned(),
                        },
                    };
                    if kind == 2 && k.shift_label.is_empty() {
                        k.shift_label = "Ctrl+C".to_owned();
                    }
                    return self.save_current(state, panel_store);
                }
                PanelEditorAction::Redraw
            }
            EditorHit::KeycodeStep(delta) => {
                if let Some(idx) = self.selected_key_idx
                    && let Some(k) = self.editing_panel.keys.get_mut(idx)
                {
                    match &mut k.action {
                        GridKeyAction::LayoutKey { keycode }
                        | GridKeyAction::FixedKey { keycode } => {
                            let cur = INSPECTOR_KEYCODES
                                .iter()
                                .position(|c| c == keycode)
                                .unwrap_or(0) as isize;
                            let len = INSPECTOR_KEYCODES.len() as isize;
                            let next = ((cur + delta).rem_euclid(len)) as usize;
                            let next_code = INSPECTOR_KEYCODES[next];
                            *keycode = next_code;
                            k.label = next_code.short_name().to_owned();
                            return self.save_current(state, panel_store);
                        }
                        GridKeyAction::Shortcut { modifiers, keycode } => {
                            let cur = INSPECTOR_KEYCODES
                                .iter()
                                .position(|c| c == keycode)
                                .unwrap_or(0) as isize;
                            let len = INSPECTOR_KEYCODES.len() as isize;
                            let next = ((cur + delta).rem_euclid(len)) as usize;
                            let next_code = INSPECTOR_KEYCODES[next];
                            *keycode = next_code;
                            k.shift_label = format_shortcut_hint(modifiers, next_code);
                            return self.save_current(state, panel_store);
                        }
                        _ => {}
                    }
                }
                PanelEditorAction::Redraw
            }
            EditorHit::ShortcutModToggle(m) => {
                if let Some(idx) = self.selected_key_idx
                    && let Some(k) = self.editing_panel.keys.get_mut(idx)
                    && let GridKeyAction::Shortcut { modifiers, keycode } = &mut k.action
                {
                    if let Some(pos) = modifiers.iter().position(|x| *x == m) {
                        modifiers.remove(pos);
                    } else {
                        modifiers.push(m);
                    }
                    k.shift_label = format_shortcut_hint(modifiers, *keycode);
                    return self.save_current(state, panel_store);
                }
                PanelEditorAction::Redraw
            }
            EditorHit::TargetPanelSelect(pid) => {
                if let Some(idx) = self.selected_key_idx
                    && let Some(k) = self.editing_panel.keys.get_mut(idx)
                {
                    k.action = GridKeyAction::SwitchPanel { panel_id: pid };
                    return self.save_current(state, panel_store);
                }
                PanelEditorAction::Redraw
            }
            EditorHit::GeomStep { field, delta } => {
                if let Some(idx) = self.selected_key_idx
                    && let Some(k) = self.editing_panel.keys.get_mut(idx)
                {
                    match field {
                        0 => k.x = PanelDefinition::snap_half(k.x + delta),
                        1 => k.y = PanelDefinition::snap_half(k.y + delta),
                        2 => k.w = PanelDefinition::snap_half(k.w + delta),
                        _ => k.h = PanelDefinition::snap_half(k.h + delta),
                    }
                    return self.save_current(state, panel_store);
                }
                PanelEditorAction::Redraw
            }
            EditorHit::RoleSelect(role) => {
                if let Some(idx) = self.selected_key_idx
                    && let Some(k) = self.editing_panel.keys.get_mut(idx)
                {
                    k.role = role;
                    return self.save_current(state, panel_store);
                }
                PanelEditorAction::Redraw
            }
            EditorHit::ColorOverride(color) => {
                if let Some(idx) = self.selected_key_idx
                    && let Some(k) = self.editing_panel.keys.get_mut(idx)
                {
                    k.color_override = color;
                    return self.save_current(state, panel_store);
                }
                PanelEditorAction::Redraw
            }
        }
    }

    /// Handle pointer motion for direct drag-to-move and drag-to-resize on the 0.5u snap grid!
    pub fn pointer_motion(&mut self, p: Point) -> PanelEditorAction {
        let Some(drag) = self.drag else {
            return PanelEditorAction::None;
        };
        let gx =
            ((p.x - self.canvas_rect.x) / self.canvas_rect.w.max(1.0)) * self.editing_panel.cols;
        let gy =
            ((p.y - self.canvas_rect.y) / self.canvas_rect.h.max(1.0)) * self.editing_panel.rows;

        match drag {
            CanvasDrag::Move {
                key_idx,
                offset_cols,
                offset_rows,
            } => {
                if let Some(k) = self.editing_panel.keys.get_mut(key_idx) {
                    let nx = PanelDefinition::snap_half(gx - offset_cols)
                        .clamp(0.0, (self.editing_panel.cols - k.w).max(0.0));
                    let ny = PanelDefinition::snap_half(gy - offset_rows)
                        .clamp(0.0, (self.editing_panel.rows - k.h).max(0.0));
                    if (nx - k.x).abs() > 0.01 || (ny - k.y).abs() > 0.01 {
                        k.x = nx;
                        k.y = ny;
                        return PanelEditorAction::Redraw;
                    }
                }
            }
            CanvasDrag::Resize { key_idx } => {
                if let Some(k) = self.editing_panel.keys.get_mut(key_idx) {
                    let nw = PanelDefinition::snap_half(gx - k.x)
                        .clamp(0.5, (self.editing_panel.cols - k.x).max(0.5));
                    let nh = PanelDefinition::snap_half(gy - k.y)
                        .clamp(0.5, (self.editing_panel.rows - k.y).max(0.5));
                    if (nw - k.w).abs() > 0.01 || (nh - k.h).abs() > 0.01 {
                        k.w = nw;
                        k.h = nh;
                        return PanelEditorAction::Redraw;
                    }
                }
            }
        }
        PanelEditorAction::None
    }

    pub fn pointer_up(
        &mut self,
        state: &mut KeyboardState,
        panel_store: &PanelStore,
    ) -> PanelEditorAction {
        if self.drag.take().is_some() {
            return self.save_current(state, panel_store);
        }
        PanelEditorAction::None
    }

    /// Feed raw evdev keyboard input into the focused inline field (`PanelName`, `KeyLabel`, `SnippetText`).
    /// Supports `Ctrl+A` (Select All), `Ctrl+C` (Copy), `Ctrl+X` (Cut), and `Ctrl+V` (Paste).
    pub fn handle_key_input(
        &mut self,
        code: KeyCode,
        shift: bool,
        ctrl: bool,
        clipboard_text: Option<&str>,
        state: &mut KeyboardState,
        panel_store: &PanelStore,
    ) -> PanelEditorAction {
        let Some(field) = self.focused_field else {
            // Allow Delete key on physical keyboard to delete selected key on canvas!
            if code == KeyCode::DELETE
                && let Some(idx) = self.selected_key_idx
                && idx < self.editing_panel.keys.len()
            {
                self.editing_panel.keys.remove(idx);
                self.selected_key_idx = if self.editing_panel.keys.is_empty() {
                    None
                } else {
                    Some(idx.min(self.editing_panel.keys.len() - 1))
                };
                return self.save_current(state, panel_store);
            }
            return PanelEditorAction::None;
        };

        if ctrl {
            match code {
                KeyCode::A => {
                    self.selection_all = true;
                    return PanelEditorAction::Redraw;
                }
                KeyCode::C => {
                    let current = self.focused_field_text(field).map(ToOwned::to_owned);
                    if let Some(txt) = current
                        && !txt.is_empty()
                    {
                        self.status_message = Some(format!("Copied \"{txt}\" to clipboard"));
                        return PanelEditorAction::CopyText(txt);
                    }
                    return PanelEditorAction::None;
                }
                KeyCode::X => {
                    let copied = self
                        .focused_field_text(field)
                        .unwrap_or_default()
                        .to_owned();
                    if let Some(buf) = self.focused_field_mut(field) {
                        buf.clear();
                    }
                    self.selection_all = false;
                    let _ = self.save_current(state, panel_store);
                    if !copied.is_empty() {
                        self.status_message = Some(format!("Cut \"{copied}\" to clipboard"));
                        return PanelEditorAction::CopyText(copied);
                    }
                    return PanelEditorAction::PanelsChanged;
                }
                KeyCode::V => {
                    if let Some(clip) = clipboard_text {
                        let clear_first = self.selection_all;
                        if let Some(buf) = self.focused_field_mut(field) {
                            if clear_first {
                                buf.clear();
                            }
                            for ch in clip.chars() {
                                if buf.len() >= 48 {
                                    break;
                                }
                                if !ch.is_control() {
                                    buf.push(ch);
                                }
                            }
                        }
                        self.selection_all = false;
                        return self.save_current(state, panel_store);
                    }
                    return PanelEditorAction::None;
                }
                _ => return PanelEditorAction::None,
            }
        }

        if code == KeyCode::ESC || code == KeyCode::ENTER {
            self.focused_field = None;
            self.selection_all = false;
            return self.save_current(state, panel_store);
        }

        let clear_first = self.selection_all;
        let Some(buf) = self.focused_field_mut(field) else {
            return PanelEditorAction::None;
        };

        if code == KeyCode::BACKSPACE {
            if clear_first {
                buf.clear();
            } else {
                buf.pop();
            }
            self.selection_all = false;
            return self.save_current(state, panel_store);
        }
        if let Some(ch) = evdev_to_ascii(code, shift) {
            if clear_first {
                buf.clear();
            }
            if buf.len() < 48 {
                buf.push(ch);
            }
            self.selection_all = false;
            return self.save_current(state, panel_store);
        }
        PanelEditorAction::None
    }

    fn focused_field_text(&self, field: EditorField) -> Option<&str> {
        match field {
            EditorField::PanelName => Some(self.editing_panel.name.as_str()),
            EditorField::KeyLabel => self
                .selected_key_idx
                .and_then(|i| self.editing_panel.keys.get(i))
                .map(|k| k.label.as_str()),
            EditorField::KeyShiftLabel => self
                .selected_key_idx
                .and_then(|i| self.editing_panel.keys.get(i))
                .map(|k| k.shift_label.as_str()),
            EditorField::SnippetText => self
                .selected_key_idx
                .and_then(|i| self.editing_panel.keys.get(i))
                .and_then(|k| match &k.action {
                    GridKeyAction::TextSnippet { text } => Some(text.as_str()),
                    _ => None,
                }),
        }
    }

    fn focused_field_mut(&mut self, field: EditorField) -> Option<&mut String> {
        match field {
            EditorField::PanelName => Some(&mut self.editing_panel.name),
            EditorField::KeyLabel => self
                .selected_key_idx
                .and_then(|i| self.editing_panel.keys.get_mut(i))
                .map(|k| &mut k.label),
            EditorField::KeyShiftLabel => self
                .selected_key_idx
                .and_then(|i| self.editing_panel.keys.get_mut(i))
                .map(|k| &mut k.shift_label),
            EditorField::SnippetText => self
                .selected_key_idx
                .and_then(|i| self.editing_panel.keys.get_mut(i))
                .and_then(|k| match &mut k.action {
                    GridKeyAction::TextSnippet { text } => Some(text),
                    _ => None,
                }),
        }
    }

    /// Render the complete 3-pane **Panel Editor** window into `pixmap` at HiDPI `scale`.
    pub fn render(
        &mut self,
        renderer: &mut UiRenderer,
        pixmap: &mut Pixmap,
        state: &KeyboardState,
        panel_store: &PanelStore,
        scale: f32,
    ) {
        self.hits.clear();

        let pal = state.config.appearance.resolved_palette();
        let w = self.width;
        let h = self.height;
        let card_bg = if pal.is_light {
            pal.colors.window_bg.adjust_brightness(0.35)
        } else {
            pal.colors.window_bg.adjust_brightness(0.05)
        };
        let border = pal.colors.key_border.with_alpha(0.65);

        UiRenderer::fill_rounded_rect(
            pixmap,
            Rect::new(0.0, 0.0, w, h),
            0.0,
            pal.colors.window_bg,
            scale,
        );

        // --- Top Toolbar (52 px) ---
        let header_h = 52.0;
        UiRenderer::fill_rounded_rect(
            pixmap,
            Rect::new(0.0, 0.0, w, header_h),
            0.0,
            pal.toolbar_bg,
            scale,
        );
        UiRenderer::draw_hline(pixmap, 0.0, w, header_h, 1.0, border, scale);

        // Subtle centered drag-handle pill indicating the header drags the window
        UiRenderer::fill_rounded_rect(
            pixmap,
            Rect::new((w - 40.0) * 0.5, 4.0, 40.0, 4.0),
            2.0,
            pal.muted_text.with_alpha(0.45),
            scale,
        );

        renderer.draw_icon(
            pixmap,
            IconId::Panels,
            Rect::new(14.0, 14.0, 24.0, 24.0),
            19.0,
            pal.colors.accent,
            scale,
        );
        renderer.draw_text(
            pixmap,
            "Panel Editor",
            Rect::new(44.0, 0.0, 128.0, header_h),
            16.0,
            700,
            pal.colors.text_icon,
            TextAlign::Left,
            scale,
        );

        // Close button top-right
        let close_rect = Rect::new(w - 40.0, 12.0, 28.0, 28.0);
        UiRenderer::fill_rounded_rect(pixmap, close_rect, 7.0, pal.colors.modifier_bg, scale);
        renderer.draw_icon(
            pixmap,
            IconId::Close,
            close_rect,
            14.0,
            pal.colors.text_icon,
            scale,
        );
        self.hits.push((close_rect, EditorHit::CloseButton));

        // Toolbar action buttons (+ Add Key, Delete Key, Reset Built-in)
        let mut bx = close_rect.x - 12.0;
        if self.editing_panel.built_in && panel_store.has_override(&self.editing_panel.id) {
            let r = Rect::new(bx - 102.0, 11.0, 102.0, 30.0);
            UiRenderer::fill_rounded_rect(pixmap, r, 7.0, pal.colors.modifier_bg, scale);
            UiRenderer::stroke_rounded_rect(pixmap, r, 7.0, 1.0, border, scale);
            renderer.draw_text(
                pixmap,
                "Reset Built-in",
                r,
                11.0,
                600,
                pal.colors.text_icon,
                TextAlign::Center,
                scale,
            );
            self.hits.push((r, EditorHit::ResetBuiltin));
            bx = r.x - 8.0;
        }
        let del_key_rect = Rect::new(bx - 110.0, 11.0, 110.0, 30.0);
        UiRenderer::fill_rounded_rect(pixmap, del_key_rect, 7.0, pal.colors.modifier_bg, scale);
        UiRenderer::stroke_rounded_rect(pixmap, del_key_rect, 7.0, 1.0, border, scale);
        renderer.draw_icon(
            pixmap,
            IconId::Trash,
            Rect::new(del_key_rect.x + 10.0, del_key_rect.y, 18.0, 30.0),
            13.5,
            pal.colors.text_icon,
            scale,
        );
        renderer.draw_text(
            pixmap,
            "Delete Key",
            Rect::new(
                del_key_rect.x + 28.0,
                del_key_rect.y,
                del_key_rect.w - 34.0,
                30.0,
            ),
            11.5,
            500,
            pal.colors.text_icon,
            TextAlign::Center,
            scale,
        );
        self.hits.push((del_key_rect, EditorHit::DeleteKey));

        let add_key_rect = Rect::new(del_key_rect.x - 114.0, 11.0, 106.0, 30.0);
        UiRenderer::fill_rounded_rect(pixmap, add_key_rect, 7.0, pal.colors.accent, scale);
        renderer.draw_icon(
            pixmap,
            IconId::Plus,
            Rect::new(add_key_rect.x + 10.0, add_key_rect.y, 18.0, 30.0),
            14.0,
            pal.accent_text,
            scale,
        );
        renderer.draw_text(
            pixmap,
            "Add Key",
            Rect::new(
                add_key_rect.x + 28.0,
                add_key_rect.y,
                add_key_rect.w - 34.0,
                30.0,
            ),
            11.5,
            700,
            pal.accent_text,
            TextAlign::Center,
            scale,
        );
        self.hits.push((add_key_rect, EditorHit::AddKey));

        // Adaptive subtitle sized strictly between title and Add Key button so it never overlaps
        let subtitle_x = 166.0;
        let subtitle_w = (add_key_rect.x - subtitle_x - 12.0).max(80.0);
        let subtitle = if subtitle_w >= 270.0 {
            format!(
                "{}  |  {:.1} x {:.1} grid (0.5u snap)",
                self.editing_panel.name, self.editing_panel.cols, self.editing_panel.rows
            )
        } else {
            format!(
                "{} ({:.1} x {:.1})",
                self.editing_panel.name, self.editing_panel.cols, self.editing_panel.rows
            )
        };
        renderer.draw_text(
            pixmap,
            &subtitle,
            Rect::new(subtitle_x, 0.0, subtitle_w, header_h),
            11.5,
            400,
            pal.muted_text,
            TextAlign::Left,
            scale,
        );

        // --- 3-Pane Body Layout (responsive for 960x590 and narrower widths) ---
        let body_y = header_h + 10.0;
        let body_h = (h - body_y - 12.0).max(200.0);
        let left_w = (w * 0.19).clamp(164.0, 200.0);
        let right_w = (w * 0.25).clamp(228.0, 268.0);
        let center_x = 12.0 + left_w + 10.0;
        let center_w = (w - center_x - right_w - 22.0).max(240.0);
        let right_x = center_x + center_w + 10.0;

        // === PANE 1 (Left): Panels Collection ===
        let left_rect = Rect::new(12.0, body_y, left_w, body_h);
        UiRenderer::fill_rounded_rect(pixmap, left_rect, 10.0, card_bg, scale);
        UiRenderer::stroke_rounded_rect(pixmap, left_rect, 10.0, 1.0, border, scale);

        renderer.draw_text(
            pixmap,
            "Panels",
            Rect::new(left_rect.x + 10.0, left_rect.y + 10.0, 56.0, 20.0),
            12.5,
            700,
            pal.colors.text_icon,
            TextAlign::Left,
            scale,
        );

        let folder_p_rect = Rect::new(
            left_rect.x + left_rect.w - 92.0,
            left_rect.y + 8.0,
            24.0,
            24.0,
        );
        UiRenderer::fill_rounded_rect(pixmap, folder_p_rect, 6.0, pal.colors.key_bg, scale);
        renderer.draw_icon(
            pixmap,
            IconId::Folder,
            folder_p_rect,
            13.0,
            pal.colors.text_icon,
            scale,
        );
        self.hits.push((folder_p_rect, EditorHit::OpenPanelsFolder));

        let new_p_rect = Rect::new(
            left_rect.x + left_rect.w - 64.0,
            left_rect.y + 8.0,
            24.0,
            24.0,
        );
        UiRenderer::fill_rounded_rect(pixmap, new_p_rect, 6.0, pal.colors.accent, scale);
        renderer.draw_icon(
            pixmap,
            IconId::Plus,
            new_p_rect,
            13.5,
            pal.accent_text,
            scale,
        );
        self.hits.push((new_p_rect, EditorHit::NewPanel));

        let dup_p_rect = Rect::new(
            left_rect.x + left_rect.w - 34.0,
            left_rect.y + 8.0,
            24.0,
            24.0,
        );
        UiRenderer::fill_rounded_rect(pixmap, dup_p_rect, 6.0, pal.colors.key_bg, scale);
        renderer.draw_icon(
            pixmap,
            IconId::Copy,
            dup_p_rect,
            13.0,
            pal.colors.text_icon,
            scale,
        );
        self.hits.push((dup_p_rect, EditorHit::DuplicatePanel));

        let mut py = left_rect.y + 40.0;
        for panel in &state.panels {
            if py + 30.0 > left_rect.y + left_rect.h - 148.0 {
                break;
            }
            let pr = Rect::new(left_rect.x + 10.0, py, left_rect.w - 20.0, 28.0);
            let active = self.editing_panel.id == panel.id;
            UiRenderer::fill_rounded_rect(
                pixmap,
                pr,
                6.0,
                if active {
                    pal.colors.accent
                } else {
                    pal.colors.key_bg
                },
                scale,
            );
            renderer.draw_text(
                pixmap,
                &panel.name,
                Rect::new(pr.x + 8.0, pr.y, pr.w - 16.0, pr.h),
                11.5,
                if active { 700 } else { 500 },
                if active {
                    pal.accent_text
                } else {
                    pal.colors.text_icon
                },
                TextAlign::Left,
                scale,
            );
            self.hits
                .push((pr, EditorHit::SelectPanel(panel.id.clone())));
            py += 33.0;
        }

        // Bottom of Left Pane: Panel Name + Grid Cols/Rows + Delete Panel
        let cfg_y = left_rect.y + left_rect.h - 140.0;
        UiRenderer::draw_hline(
            pixmap,
            left_rect.x + 10.0,
            left_rect.x + left_rect.w - 10.0,
            cfg_y,
            1.0,
            border,
            scale,
        );
        renderer.draw_text(
            pixmap,
            "Panel Name",
            Rect::new(left_rect.x + 10.0, cfg_y + 8.0, left_rect.w - 20.0, 16.0),
            10.5,
            600,
            pal.muted_text,
            TextAlign::Left,
            scale,
        );
        let pname_r = Rect::new(left_rect.x + 10.0, cfg_y + 26.0, left_rect.w - 20.0, 24.0);
        let pname_foc = self.focused_field == Some(EditorField::PanelName);
        UiRenderer::fill_rounded_rect(pixmap, pname_r, 5.0, pal.colors.window_bg, scale);
        if pname_foc && self.selection_all && !self.editing_panel.name.is_empty() {
            UiRenderer::fill_rounded_rect(
                pixmap,
                pname_r.inset(3.0),
                3.0,
                pal.colors.accent.with_alpha(0.32),
                scale,
            );
        }
        UiRenderer::stroke_rounded_rect(
            pixmap,
            pname_r,
            5.0,
            if pname_foc { 1.6 } else { 1.0 },
            if pname_foc { pal.colors.accent } else { border },
            scale,
        );
        let pname_txt = if pname_foc && !self.selection_all {
            format!("{}|", self.editing_panel.name)
        } else {
            self.editing_panel.name.clone()
        };
        renderer.draw_text(
            pixmap,
            &pname_txt,
            Rect::new(pname_r.x + 6.0, pname_r.y, pname_r.w - 10.0, pname_r.h),
            11.0,
            500,
            pal.colors.text_icon,
            TextAlign::Left,
            scale,
        );
        self.hits.push((pname_r, EditorHit::PanelNameInput));

        // Cols & Rows steppers
        self.draw_stepper(
            renderer,
            pixmap,
            &pal,
            Rect::new(left_rect.x + 10.0, cfg_y + 56.0, left_rect.w - 20.0, 22.0),
            "Cols",
            &format!("{:.1}", self.editing_panel.cols),
            EditorHit::GridColsStep(-0.5),
            EditorHit::GridColsStep(0.5),
            scale,
        );
        self.draw_stepper(
            renderer,
            pixmap,
            &pal,
            Rect::new(left_rect.x + 10.0, cfg_y + 82.0, left_rect.w - 20.0, 22.0),
            "Rows",
            &format!("{:.1}", self.editing_panel.rows),
            EditorHit::GridRowsStep(-0.5),
            EditorHit::GridRowsStep(0.5),
            scale,
        );

        let del_panel_r = Rect::new(left_rect.x + 10.0, cfg_y + 110.0, left_rect.w - 20.0, 24.0);
        UiRenderer::fill_rounded_rect(pixmap, del_panel_r, 6.0, pal.colors.modifier_bg, scale);
        UiRenderer::stroke_rounded_rect(pixmap, del_panel_r, 6.0, 1.0, border, scale);
        renderer.draw_text(
            pixmap,
            if self.editing_panel.built_in {
                "Built-in Panel"
            } else {
                "Delete Panel"
            },
            del_panel_r,
            10.5,
            500,
            if self.editing_panel.built_in {
                pal.muted_text
            } else {
                Color::rgb(0xEF, 0x44, 0x44)
            },
            TextAlign::Center,
            scale,
        );
        self.hits.push((del_panel_r, EditorHit::DeletePanel));

        // === PANE 2 (Center): Interactive 0.5u Snap-Grid Blueprint Canvas ===
        let center_outer = Rect::new(center_x, body_y, center_w, body_h);
        UiRenderer::fill_rounded_rect(pixmap, center_outer, 10.0, card_bg, scale);
        UiRenderer::stroke_rounded_rect(pixmap, center_outer, 10.0, 1.0, border, scale);

        // Fit panel aspect ratio comfortably inside center pane
        let pad = 18.0;
        let avail_w = (center_outer.w - pad * 2.0).max(120.0);
        let avail_h = (center_outer.h - pad * 2.0 - 28.0).max(120.0);
        let desired_aspect =
            (self.editing_panel.cols / self.editing_panel.rows.max(1.0)).clamp(1.2, 4.2);
        let (cw, ch) = if avail_w / avail_h > desired_aspect {
            (avail_h * desired_aspect, avail_h)
        } else {
            (avail_w, avail_w / desired_aspect)
        };
        let cx = center_outer.x + (center_outer.w - cw) * 0.5;
        let cy = center_outer.y + (center_outer.h - 24.0 - ch) * 0.5;
        self.canvas_rect = Rect::new(cx, cy, cw, ch);

        // Subtle canvas heading
        renderer.draw_text(
            pixmap,
            "Blueprint Canvas (0.5u snap grid)",
            Rect::new(
                center_outer.x + 14.0,
                center_outer.y + 10.0,
                center_outer.w - 28.0,
                18.0,
            ),
            11.0,
            600,
            pal.muted_text,
            TextAlign::Left,
            scale,
        );

        // Blueprint grid background + subtle 0.5u & 1.0u lines
        let frame_rect = Rect::new(cx - 8.0, cy - 8.0, cw + 16.0, ch + 16.0);
        UiRenderer::fill_rounded_rect(pixmap, frame_rect, 10.0, pal.colors.window_bg, scale);
        UiRenderer::stroke_rounded_rect(
            pixmap,
            frame_rect,
            10.0,
            1.8,
            pal.colors.window_outline.with_alpha(0.75),
            scale,
        );

        let cols = self.editing_panel.cols.max(1.0);
        let rows = self.editing_panel.rows.max(1.0);
        let steps_x = (cols * 2.0).round() as usize;
        let steps_y = (rows * 2.0).round() as usize;
        for sx in 0..=steps_x {
            let gx = cx + (sx as f32 * 0.5 / cols) * cw;
            let major = sx % 2 == 0;
            let col = pal
                .colors
                .key_border
                .with_alpha(if major { 0.45 } else { 0.22 });
            UiRenderer::fill_rounded_rect(pixmap, Rect::new(gx, cy, 1.0, ch), 0.0, col, scale);
        }
        for sy in 0..=steps_y {
            let gy = cy + (sy as f32 * 0.5 / rows) * ch;
            let major = sy % 2 == 0;
            let col = pal
                .colors
                .key_border
                .with_alpha(if major { 0.45 } else { 0.22 });
            UiRenderer::draw_hline(pixmap, cx, cx + cw, gy, 1.0, col, scale);
        }

        // Draw every key on the canvas
        let gap = 3.5;
        let unit_w = (cw + gap) / cols;
        let unit_h = (ch + gap) / rows;
        let layout = state.active_layout();
        let use_symbols = state.config.layout.keycap_style == KeycapStyle::Symbols;

        for (idx, k) in self.editing_panel.keys.iter().enumerate() {
            let kr = Rect::new(
                cx + k.x * unit_w,
                cy + k.y * unit_h,
                (k.w * unit_w - gap).max(10.0),
                (k.h * unit_h - gap).max(10.0),
            );
            let selected = self.selected_key_idx == Some(idx);
            let fill = if let Some(custom) = k.color_override {
                custom
            } else if matches!(k.role, KeyRole::Character | KeyRole::Space) {
                pal.colors.key_bg
            } else {
                pal.colors.modifier_bg
            };
            let fg = if let Some(custom) = k.color_override {
                custom.contrasting_text()
            } else {
                pal.colors.text_icon
            };

            let r = (state.config.appearance.corner_radius * 0.75).min(kr.h * 0.35);
            UiRenderer::fill_rounded_rect(pixmap, kr, r, fill, scale);
            UiRenderer::stroke_rounded_rect(
                pixmap,
                kr,
                r,
                if selected { 2.2 } else { 1.0 },
                if selected {
                    pal.colors.accent
                } else {
                    pal.colors.key_border
                },
                scale,
            );

            let resolved = ResolvedGridKey::resolve(k.clone(), layout);
            let chosen_icon = if use_symbols {
                k.symbol_icon.or(k.icon)
            } else {
                k.icon
            };
            if let Some(icon_id) = chosen_icon {
                renderer.draw_icon(
                    pixmap,
                    icon_id,
                    kr,
                    (kr.h * 0.52).clamp(9.0, 17.0),
                    fg,
                    scale,
                );
            } else {
                let lbl = if k.role == KeyRole::Space && resolved.normal.is_empty() {
                    "Space"
                } else {
                    resolved.normal.as_str()
                };
                renderer.draw_text(
                    pixmap,
                    lbl,
                    kr.inset(2.0),
                    (kr.h * 0.35).clamp(8.5, 16.0),
                    500,
                    fg,
                    TextAlign::Center,
                    scale,
                );
            }

            self.hits.push((kr, EditorHit::CanvasKey(idx)));

            if selected {
                let handle = Rect::new(kr.x + kr.w - 9.0, kr.y + kr.h - 9.0, 10.0, 10.0);
                UiRenderer::fill_rounded_rect(pixmap, handle, 2.5, pal.colors.accent, scale);
                UiRenderer::stroke_rounded_rect(
                    pixmap,
                    handle,
                    2.5,
                    1.2,
                    Color::rgb(255, 255, 255),
                    scale,
                );
                self.hits.push((
                    Rect::new(handle.x - 4.0, handle.y - 4.0, 18.0, 18.0),
                    EditorHit::CanvasKeyResize(idx),
                ));
            }
        }

        // Canvas tip footer
        let tip = self
            .status_message
            .as_deref()
            .unwrap_or("Click key to inspect  |  Drag to move (0.5u snap)  |  Drag corner to resize  |  Auto-saved");
        renderer.draw_text(
            pixmap,
            tip,
            Rect::new(
                center_outer.x + 12.0,
                center_outer.y + center_outer.h - 24.0,
                center_outer.w - 24.0,
                18.0,
            ),
            10.0,
            400,
            if self.status_message.is_some() {
                pal.colors.accent
            } else {
                pal.muted_text
            },
            TextAlign::Center,
            scale,
        );

        // === PANE 3 (Right): Key Inspector ===
        let right_rect = Rect::new(right_x, body_y, right_w, body_h);
        UiRenderer::fill_rounded_rect(pixmap, right_rect, 10.0, card_bg, scale);
        UiRenderer::stroke_rounded_rect(pixmap, right_rect, 10.0, 1.0, border, scale);

        renderer.draw_text(
            pixmap,
            "Key Inspector",
            Rect::new(
                right_rect.x + 12.0,
                right_rect.y + 10.0,
                right_rect.w - 24.0,
                20.0,
            ),
            12.5,
            700,
            pal.colors.text_icon,
            TextAlign::Left,
            scale,
        );

        let Some(sel_idx) = self.selected_key_idx else {
            renderer.draw_text(
                pixmap,
                "Select any key on the grid canvas to edit its label, size, action, or color.",
                Rect::new(
                    right_rect.x + 12.0,
                    right_rect.y + 42.0,
                    right_rect.w - 24.0,
                    60.0,
                ),
                11.0,
                400,
                pal.muted_text,
                TextAlign::Left,
                scale,
            );
            return;
        };
        let Some(k) = self.editing_panel.keys.get(sel_idx).cloned() else {
            return;
        };

        let ix = right_rect.x + 12.0;
        let iw = right_rect.w - 24.0;
        let mut iy = right_rect.y + 38.0;

        // Label & Sub-label inputs
        let label_placeholder = if k.role == KeyRole::Space {
            Some("Auto (active layout)")
        } else {
            None
        };
        self.draw_text_field(
            renderer,
            pixmap,
            &pal,
            border,
            ix,
            iy,
            iw,
            "Label",
            &k.label,
            label_placeholder,
            self.focused_field == Some(EditorField::KeyLabel),
            EditorHit::KeyLabelInput,
            scale,
        );
        iy += 46.0;

        self.draw_text_field(
            renderer,
            pixmap,
            &pal,
            border,
            ix,
            iy,
            iw,
            "Sub / Shift Label",
            &k.shift_label,
            None,
            self.focused_field == Some(EditorField::KeyShiftLabel),
            EditorHit::KeyShiftInput,
            scale,
        );
        iy += 48.0;

        // Action Type pills
        renderer.draw_text(
            pixmap,
            "Action Type",
            Rect::new(ix, iy, iw, 16.0),
            10.5,
            600,
            pal.muted_text,
            TextAlign::Left,
            scale,
        );
        iy += 18.0;
        let kinds = [
            (0u8, "Layout"),
            (1u8, "Fixed"),
            (2u8, "Combo"),
            (3u8, "Text"),
            (4u8, "Panel"),
        ];
        let active_kind: u8 = match k.action {
            GridKeyAction::LayoutKey { .. } => 0,
            GridKeyAction::FixedKey { .. } => 1,
            GridKeyAction::Shortcut { .. } => 2,
            GridKeyAction::TextSnippet { .. } => 3,
            GridKeyAction::SwitchPanel { .. } | GridKeyAction::ToggleCompactPage => 4,
        };
        let kw_pill = (iw - 12.0) / 5.0;
        for (i, (kid, lbl)) in kinds.iter().enumerate() {
            let kr = Rect::new(ix + i as f32 * (kw_pill + 3.0), iy, kw_pill, 24.0);
            let act = active_kind == *kid;
            UiRenderer::fill_rounded_rect(
                pixmap,
                kr,
                5.0,
                if act {
                    pal.colors.accent
                } else {
                    pal.colors.key_bg
                },
                scale,
            );
            renderer.draw_text(
                pixmap,
                lbl,
                kr,
                9.5,
                if act { 700 } else { 500 },
                if act {
                    pal.accent_text
                } else {
                    pal.colors.text_icon
                },
                TextAlign::Center,
                scale,
            );
            self.hits.push((kr, EditorHit::ActionKind(*kid)));
        }
        iy += 32.0;

        // Context-specific action editor
        match &k.action {
            GridKeyAction::LayoutKey { keycode } | GridKeyAction::FixedKey { keycode } => {
                self.draw_stepper(
                    renderer,
                    pixmap,
                    &pal,
                    Rect::new(ix, iy, iw, 26.0),
                    "Key",
                    keycode.short_name(),
                    EditorHit::KeycodeStep(-1),
                    EditorHit::KeycodeStep(1),
                    scale,
                );
                iy += 36.0;
            }
            GridKeyAction::Shortcut { modifiers, keycode } => {
                let mods = [
                    ModifierKind::Ctrl,
                    ModifierKind::Shift,
                    ModifierKind::Alt,
                    ModifierKind::Super,
                ];
                let mw = (iw - 9.0) / 4.0;
                for (i, m) in mods.iter().enumerate() {
                    let mr = Rect::new(ix + i as f32 * (mw + 3.0), iy, mw, 24.0);
                    let on = modifiers.contains(m);
                    UiRenderer::fill_rounded_rect(
                        pixmap,
                        mr,
                        5.0,
                        if on {
                            pal.colors.accent
                        } else {
                            pal.colors.key_bg
                        },
                        scale,
                    );
                    renderer.draw_text(
                        pixmap,
                        m.label(),
                        mr,
                        10.0,
                        if on { 700 } else { 500 },
                        if on {
                            pal.accent_text
                        } else {
                            pal.colors.text_icon
                        },
                        TextAlign::Center,
                        scale,
                    );
                    self.hits.push((mr, EditorHit::ShortcutModToggle(*m)));
                }
                iy += 30.0;
                self.draw_stepper(
                    renderer,
                    pixmap,
                    &pal,
                    Rect::new(ix, iy, iw, 24.0),
                    "Key",
                    keycode.short_name(),
                    EditorHit::KeycodeStep(-1),
                    EditorHit::KeycodeStep(1),
                    scale,
                );
                iy += 32.0;
            }
            GridKeyAction::TextSnippet { text } => {
                self.draw_text_field(
                    renderer,
                    pixmap,
                    &pal,
                    border,
                    ix,
                    iy,
                    iw,
                    "Text to Type",
                    text,
                    None,
                    self.focused_field == Some(EditorField::SnippetText),
                    EditorHit::SnippetInput,
                    scale,
                );
                iy += 48.0;
            }
            GridKeyAction::SwitchPanel { panel_id } => {
                renderer.draw_text(
                    pixmap,
                    "Target Panel",
                    Rect::new(ix, iy, iw, 16.0),
                    10.5,
                    600,
                    pal.muted_text,
                    TextAlign::Left,
                    scale,
                );
                iy += 18.0;
                let mut tx = ix;
                for p in state.panels.iter().take(4) {
                    let pw = (iw - 6.0) * 0.5;
                    let pr = Rect::new(tx, iy, pw, 22.0);
                    let on = panel_id == &p.id;
                    UiRenderer::fill_rounded_rect(
                        pixmap,
                        pr,
                        5.0,
                        if on {
                            pal.colors.accent
                        } else {
                            pal.colors.key_bg
                        },
                        scale,
                    );
                    renderer.draw_text(
                        pixmap,
                        &p.name,
                        pr.inset(2.0),
                        9.5,
                        if on { 700 } else { 500 },
                        if on {
                            pal.accent_text
                        } else {
                            pal.colors.text_icon
                        },
                        TextAlign::Center,
                        scale,
                    );
                    self.hits
                        .push((pr, EditorHit::TargetPanelSelect(p.id.clone())));
                    if (tx - ix).abs() < 1.0 {
                        tx = ix + pw + 6.0;
                    } else {
                        tx = ix;
                        iy += 26.0;
                    }
                }
                if (tx - ix).abs() > 1.0 {
                    iy += 26.0;
                }
                iy += 4.0;
            }
            GridKeyAction::ToggleCompactPage => {
                renderer.draw_text(
                    pixmap,
                    "Toggles Compact TV ABC / 123",
                    Rect::new(ix, iy, iw, 22.0),
                    10.5,
                    400,
                    pal.muted_text,
                    TextAlign::Left,
                    scale,
                );
                iy += 30.0;
            }
        }

        // Geometry steppers (X, Y, W, H on 0.5u grid)
        renderer.draw_text(
            pixmap,
            "Grid Position & Size (0.5u)",
            Rect::new(ix, iy, iw, 16.0),
            10.5,
            600,
            pal.muted_text,
            TextAlign::Left,
            scale,
        );
        iy += 20.0;
        let half_w = (iw - 8.0) * 0.5;
        self.draw_stepper(
            renderer,
            pixmap,
            &pal,
            Rect::new(ix, iy, half_w, 24.0),
            "X",
            &format!("{:.1}", k.x),
            EditorHit::GeomStep {
                field: 0,
                delta: -0.5,
            },
            EditorHit::GeomStep {
                field: 0,
                delta: 0.5,
            },
            scale,
        );
        self.draw_stepper(
            renderer,
            pixmap,
            &pal,
            Rect::new(ix + half_w + 8.0, iy, half_w, 24.0),
            "Y",
            &format!("{:.1}", k.y),
            EditorHit::GeomStep {
                field: 1,
                delta: -0.5,
            },
            EditorHit::GeomStep {
                field: 1,
                delta: 0.5,
            },
            scale,
        );
        iy += 28.0;
        self.draw_stepper(
            renderer,
            pixmap,
            &pal,
            Rect::new(ix, iy, half_w, 24.0),
            "W",
            &format!("{:.1}", k.w),
            EditorHit::GeomStep {
                field: 2,
                delta: -0.5,
            },
            EditorHit::GeomStep {
                field: 2,
                delta: 0.5,
            },
            scale,
        );
        self.draw_stepper(
            renderer,
            pixmap,
            &pal,
            Rect::new(ix + half_w + 8.0, iy, half_w, 24.0),
            "H",
            &format!("{:.1}", k.h),
            EditorHit::GeomStep {
                field: 3,
                delta: -0.5,
            },
            EditorHit::GeomStep {
                field: 3,
                delta: 0.5,
            },
            scale,
        );
        iy += 34.0;

        // Key Role selector
        renderer.draw_text(
            pixmap,
            "Key Style Role",
            Rect::new(ix, iy, iw, 16.0),
            10.5,
            600,
            pal.muted_text,
            TextAlign::Left,
            scale,
        );
        iy += 18.0;
        let roles = [
            (KeyRole::Character, "Key"),
            (KeyRole::Action, "Action"),
            (KeyRole::Modifier(ModifierKind::Shift), "Mod"),
            (KeyRole::Media, "Media"),
        ];
        let rw_p = (iw - 9.0) / 4.0;
        for (i, (r_val, lbl)) in roles.iter().enumerate() {
            let rr = Rect::new(ix + i as f32 * (rw_p + 3.0), iy, rw_p, 24.0);
            let on = std::mem::discriminant(&k.role) == std::mem::discriminant(r_val);
            UiRenderer::fill_rounded_rect(
                pixmap,
                rr,
                5.0,
                if on {
                    pal.colors.accent
                } else {
                    pal.colors.key_bg
                },
                scale,
            );
            renderer.draw_text(
                pixmap,
                lbl,
                rr,
                10.0,
                if on { 700 } else { 500 },
                if on {
                    pal.accent_text
                } else {
                    pal.colors.text_icon
                },
                TextAlign::Center,
                scale,
            );
            self.hits.push((rr, EditorHit::RoleSelect(*r_val)));
        }
        iy += 32.0;

        // Key Color Override swatches
        renderer.draw_text(
            pixmap,
            "Key Color Tint",
            Rect::new(ix, iy, iw, 16.0),
            10.5,
            600,
            pal.muted_text,
            TextAlign::Left,
            scale,
        );
        iy += 20.0;
        let auto_r = Rect::new(ix, iy, 40.0, 22.0);
        let auto_on = k.color_override.is_none();
        UiRenderer::fill_rounded_rect(
            pixmap,
            auto_r,
            5.0,
            if auto_on {
                pal.colors.accent
            } else {
                pal.colors.key_bg
            },
            scale,
        );
        renderer.draw_text(
            pixmap,
            "Auto",
            auto_r,
            9.5,
            600,
            if auto_on {
                pal.accent_text
            } else {
                pal.colors.text_icon
            },
            TextAlign::Center,
            scale,
        );
        self.hits.push((auto_r, EditorHit::ColorOverride(None)));

        let swatch_area = (iw - 46.0).max(112.0);
        let swatch_step = (swatch_area / 7.0).min(26.0);
        let swatch_radius = if swatch_step < 23.0 { 7.5 } else { 8.5 };
        for (i, sw) in ACCENT_SWATCHES.iter().take(7).enumerate() {
            let sx = ix + 48.0 + swatch_radius + i as f32 * swatch_step;
            let sy = iy + 11.0;
            UiRenderer::fill_circle(pixmap, sx, sy, swatch_radius, sw.color, scale);
            if k.color_override == Some(sw.color) {
                let ring = swatch_radius + 2.0;
                UiRenderer::stroke_rounded_rect(
                    pixmap,
                    Rect::new(sx - ring, sy - ring, ring * 2.0, ring * 2.0),
                    ring,
                    1.8,
                    pal.colors.text_icon,
                    scale,
                );
            }
            self.hits.push((
                Rect::new(sx - 10.0, sy - 10.0, 20.0, 20.0),
                EditorHit::ColorOverride(Some(sw.color)),
            ));
        }

        // Crisp 1.5px window outline
        UiRenderer::stroke_rounded_rect(
            pixmap,
            Rect::new(0.0, 0.0, w, h),
            0.0,
            1.5,
            pal.colors.window_outline.with_alpha(0.55),
            scale,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_text_field(
        &mut self,
        renderer: &mut UiRenderer,
        pixmap: &mut Pixmap,
        pal: &wlboard_core::ResolvedPalette,
        border: Color,
        x: f32,
        y: f32,
        w: f32,
        label: &str,
        value: &str,
        placeholder: Option<&str>,
        focused: bool,
        hit: EditorHit,
        scale: f32,
    ) {
        renderer.draw_text(
            pixmap,
            label,
            Rect::new(x, y, w, 16.0),
            10.5,
            600,
            pal.muted_text,
            TextAlign::Left,
            scale,
        );
        let r = Rect::new(x, y + 18.0, w, 24.0);
        UiRenderer::fill_rounded_rect(pixmap, r, 5.0, pal.colors.window_bg, scale);
        if focused && self.selection_all && !value.is_empty() {
            UiRenderer::fill_rounded_rect(
                pixmap,
                r.inset(3.0),
                3.0,
                pal.colors.accent.with_alpha(0.32),
                scale,
            );
        }
        UiRenderer::stroke_rounded_rect(
            pixmap,
            r,
            5.0,
            if focused { 1.8 } else { 1.0 },
            if focused { pal.colors.accent } else { border },
            scale,
        );
        let (disp, fg) = if focused && !self.selection_all {
            (format!("{value}|"), pal.colors.text_icon)
        } else if value.is_empty() {
            if let Some(ph) = placeholder {
                (ph.to_owned(), pal.muted_text)
            } else {
                (String::new(), pal.colors.text_icon)
            }
        } else {
            (value.to_owned(), pal.colors.text_icon)
        };
        renderer.draw_text(
            pixmap,
            &disp,
            Rect::new(r.x + 7.0, r.y, r.w - 12.0, r.h),
            11.0,
            if value.is_empty() && !focused {
                400
            } else {
                500
            },
            fg,
            TextAlign::Left,
            scale,
        );
        self.hits.push((r, hit));
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_stepper(
        &mut self,
        renderer: &mut UiRenderer,
        pixmap: &mut Pixmap,
        pal: &wlboard_core::ResolvedPalette,
        rect: Rect,
        label: &str,
        value: &str,
        dec_hit: EditorHit,
        inc_hit: EditorHit,
        scale: f32,
    ) {
        let lbl_w = if label.len() <= 1 {
            16.0
        } else {
            (rect.w * 0.34).clamp(24.0, 44.0)
        };
        renderer.draw_text(
            pixmap,
            label,
            Rect::new(rect.x, rect.y, lbl_w, rect.h),
            10.5,
            500,
            pal.muted_text,
            TextAlign::Left,
            scale,
        );
        let btn_w = 22.0;
        let dec_r = Rect::new(rect.x + lbl_w, rect.y, btn_w, rect.h);
        let inc_r = Rect::new(rect.x + rect.w - btn_w, rect.y, btn_w, rect.h);
        let val_r = Rect::new(
            dec_r.x + btn_w + 2.0,
            rect.y,
            (inc_r.x - (dec_r.x + btn_w) - 4.0).max(20.0),
            rect.h,
        );

        UiRenderer::fill_rounded_rect(pixmap, dec_r, 5.0, pal.colors.key_bg, scale);
        renderer.draw_text(
            pixmap,
            "-",
            dec_r,
            12.0,
            700,
            pal.colors.text_icon,
            TextAlign::Center,
            scale,
        );
        self.hits.push((dec_r, dec_hit));

        renderer.draw_text(
            pixmap,
            value,
            val_r,
            10.5,
            600,
            pal.colors.text_icon,
            TextAlign::Center,
            scale,
        );

        UiRenderer::fill_rounded_rect(pixmap, inc_r, 5.0, pal.colors.key_bg, scale);
        renderer.draw_text(
            pixmap,
            "+",
            inc_r,
            12.0,
            700,
            pal.colors.text_icon,
            TextAlign::Center,
            scale,
        );
        self.hits.push((inc_r, inc_hit));
    }
}

fn format_shortcut_hint(modifiers: &[ModifierKind], keycode: KeyCode) -> String {
    let mut parts: Vec<&str> = modifiers.iter().map(|m| m.label()).collect();
    parts.push(keycode.short_name());
    parts.join("+")
}
