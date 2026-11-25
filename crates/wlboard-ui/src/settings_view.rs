//! Built-in themed Wayland `xdg_toplevel` **Customize wlboard** Settings & Presets window
//! rendered with the exact same `tiny-skia` + `cosmic-text` + `resvg` engine.

use tiny_skia::{Pixmap, Transform};
use wlboard_core::{
    ACCENT_SWATCHES, AppearanceSettings, BUILTIN_LAYOUT_CODES, Color, ColorSlot, IconId, KeyCode,
    KeyboardState, KeycapStyle, Point, PresetStore, Rect, ThemeBase,
};

use crate::canvas::{TextAlign, UiRenderer};

pub const SETTINGS_DEFAULT_WIDTH: u32 = 720;
pub const SETTINGS_DEFAULT_HEIGHT: u32 = 680;

/// Active tab inside the built-in Settings window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsTab {
    #[default]
    Appearance,
    Layout,
    Behavior,
}

/// Which inline text input currently owns keyboard focus inside Settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusedField {
    PresetName,
    HexColor,
}

/// Interactive slider identifiers inside Tab 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SliderId {
    OutlineWidth,
    KeyBorderWidth,
    Opacity,
    FontScale,
    CornerRadius,
    KeySpacing,
}

/// Interactive toggle identifiers across all Settings tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToggleId {
    KeycapShadow,
    SystemLayoutSync,
    AutoRegisterKdeLayouts,
    FastSwitcher,
    PanelSwitcher,
    MediaRow,
    AltGrAmberHints,
    StickyModifierLock,
    StartMinimized,
}

/// Action dispatched when the user interacts with a Settings control.
#[derive(Debug, Clone, PartialEq)]
pub enum SettingsAction {
    None,
    Redraw,
    ConfigChanged,
    LayoutChanged(String),
    OpenPanelEditor,
    CloseSettings,
    QuitApp,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ActiveDrag {
    SvSquare(Rect),
    HueBar(Rect),
    Slider(SliderId, Rect),
}

#[derive(Debug, Clone)]
enum ControlHit {
    Tab(SettingsTab),
    CloseButton,
    SelectPreset(String),
    PresetNameInput,
    SavePreset,
    DeletePreset,
    ThemeBase(ThemeBase),
    AccentSwatch(Color),
    ColorSlotChip(ColorSlot),
    SvSquare(Rect),
    HueBar(Rect),
    HexInput,
    Slider(SliderId, Rect),
    FontWeight(u16),
    Toggle(ToggleId),
    ResetDefaults,
    PanelPill(String),
    OpenPanelEditor,
    KeycapStyle(KeycapStyle),
    LayoutCard(String),
    ToggleVisibleLang(String),
    MoveVisibleLang(String, isize),
    QuitButton,
}

/// Complete state for the built-in `Customize wlboard` Settings & Presets window.
#[derive(Debug, Clone)]
pub struct SettingsView {
    pub tab: SettingsTab,
    pub selected_slot: ColorSlot,
    pub hsv: (f32, f32, f32),
    pub hex_input: String,
    pub preset_name_input: String,
    pub focused_field: Option<FocusedField>,
    pub status_message: Option<String>,
    pub width: f32,
    pub height: f32,
    active_drag: Option<ActiveDrag>,
    hits: Vec<(Rect, ControlHit)>,
}

impl Default for SettingsView {
    fn default() -> Self {
        Self::new(&AppearanceSettings::default())
    }
}

impl SettingsView {
    #[must_use]
    pub fn new(appearance: &AppearanceSettings) -> Self {
        let initial_color = appearance.colors.get(ColorSlot::Accent);
        Self {
            tab: SettingsTab::Appearance,
            selected_slot: ColorSlot::Accent,
            hsv: initial_color.to_hsv(),
            hex_input: initial_color.to_hex(),
            preset_name_input: String::new(),
            focused_field: None,
            status_message: None,
            width: SETTINGS_DEFAULT_WIDTH as f32,
            height: SETTINGS_DEFAULT_HEIGHT as f32,
            active_drag: None,
            hits: Vec::new(),
        }
    }

    pub fn sync_from_appearance(&mut self, appearance: &AppearanceSettings) {
        let color = appearance.colors.get(self.selected_slot);
        self.hsv = color.to_hsv();
        self.hex_input = color.to_hex();
    }

    /// Handle pointer press inside the Settings window.
    pub fn pointer_down(
        &mut self,
        p: Point,
        state: &mut KeyboardState,
        preset_store: &PresetStore,
    ) -> SettingsAction {
        let hit = self
            .hits
            .iter()
            .rev()
            .find(|(r, _)| r.contains(p))
            .map(|(_, h)| h.clone());

        let Some(hit) = hit else {
            self.focused_field = None;
            return SettingsAction::Redraw;
        };

        self.status_message = None;

        match hit {
            ControlHit::Tab(t) => {
                self.tab = t;
                self.focused_field = None;
                SettingsAction::Redraw
            }
            ControlHit::CloseButton => SettingsAction::CloseSettings,
            ControlHit::SelectPreset(name) => {
                if let Ok(preset) = preset_store.load(&name) {
                    state.config.appearance = preset.appearance;
                    state.config.active_preset = Some(preset.name.clone());
                    self.preset_name_input = preset.name.clone();
                    self.sync_from_appearance(&state.config.appearance);
                    self.status_message = Some(format!("Loaded preset \"{}\"", preset.name));
                    return SettingsAction::ConfigChanged;
                }
                SettingsAction::Redraw
            }
            ControlHit::PresetNameInput => {
                self.focused_field = Some(FocusedField::PresetName);
                SettingsAction::Redraw
            }
            ControlHit::SavePreset => {
                let name = self.preset_name_input.trim().to_owned();
                if name.is_empty() {
                    self.status_message = Some("Enter a preset name first".to_owned());
                    self.focused_field = Some(FocusedField::PresetName);
                    return SettingsAction::Redraw;
                }
                match preset_store.save(&name, &state.config.appearance) {
                    Ok(saved) => {
                        state.config.active_preset = Some(saved.name.clone());
                        state.reload_presets(preset_store);
                        self.status_message = Some(format!("Saved preset \"{}\"", saved.name));
                        SettingsAction::ConfigChanged
                    }
                    Err(err) => {
                        self.status_message = Some(format!("Save error: {err}"));
                        SettingsAction::Redraw
                    }
                }
            }
            ControlHit::DeletePreset => {
                let target = if !self.preset_name_input.trim().is_empty() {
                    self.preset_name_input.trim().to_owned()
                } else if let Some(ref active) = state.config.active_preset {
                    active.clone()
                } else {
                    String::new()
                };
                if target.is_empty() {
                    self.status_message = Some("Select a preset to delete".to_owned());
                    return SettingsAction::Redraw;
                }
                match preset_store.delete(&target) {
                    Ok(()) => {
                        if state.config.active_preset.as_deref() == Some(target.as_str()) {
                            state.config.active_preset = None;
                        }
                        self.preset_name_input.clear();
                        state.reload_presets(preset_store);
                        self.status_message = Some(format!("Deleted preset \"{target}\""));
                        SettingsAction::ConfigChanged
                    }
                    Err(_) => {
                        self.status_message = Some(format!("Preset \"{target}\" not found"));
                        SettingsAction::Redraw
                    }
                }
            }
            ControlHit::ThemeBase(base) => {
                state.config.appearance.apply_base(base);
                state.config.active_preset = None;
                self.sync_from_appearance(&state.config.appearance);
                SettingsAction::ConfigChanged
            }
            ControlHit::AccentSwatch(color) => {
                state.config.appearance.colors.accent = color;
                state.config.active_preset = None;
                if self.selected_slot == ColorSlot::Accent {
                    self.sync_from_appearance(&state.config.appearance);
                }
                SettingsAction::ConfigChanged
            }
            ControlHit::ColorSlotChip(slot) => {
                self.selected_slot = slot;
                self.focused_field = None;
                self.sync_from_appearance(&state.config.appearance);
                SettingsAction::Redraw
            }
            ControlHit::SvSquare(rect) => {
                self.focused_field = None;
                self.active_drag = Some(ActiveDrag::SvSquare(rect));
                self.apply_sv_drag(p, rect, state);
                SettingsAction::ConfigChanged
            }
            ControlHit::HueBar(rect) => {
                self.focused_field = None;
                self.active_drag = Some(ActiveDrag::HueBar(rect));
                self.apply_hue_drag(p, rect, state);
                SettingsAction::ConfigChanged
            }
            ControlHit::HexInput => {
                self.focused_field = Some(FocusedField::HexColor);
                SettingsAction::Redraw
            }
            ControlHit::Slider(id, rect) => {
                self.focused_field = None;
                self.active_drag = Some(ActiveDrag::Slider(id, rect));
                self.apply_slider_drag(id, p, rect, state);
                SettingsAction::ConfigChanged
            }
            ControlHit::FontWeight(w) => {
                state.config.appearance.font_weight = w;
                state.config.active_preset = None;
                SettingsAction::ConfigChanged
            }
            ControlHit::Toggle(id) => {
                match id {
                    ToggleId::KeycapShadow => {
                        state.config.appearance.keycap_shadow =
                            !state.config.appearance.keycap_shadow;
                        state.config.active_preset = None;
                    }
                    ToggleId::SystemLayoutSync => {
                        state.config.layout.system_layout_sync =
                            !state.config.layout.system_layout_sync;
                    }
                    ToggleId::AutoRegisterKdeLayouts => {
                        state.config.layout.auto_register_kde_layouts =
                            !state.config.layout.auto_register_kde_layouts;
                    }
                    ToggleId::FastSwitcher => {
                        state.config.layout.show_fast_switcher =
                            !state.config.layout.show_fast_switcher;
                    }
                    ToggleId::PanelSwitcher => {
                        state.config.layout.show_panel_switcher =
                            !state.config.layout.show_panel_switcher;
                    }
                    ToggleId::MediaRow => {
                        state.config.layout.show_media_row = !state.config.layout.show_media_row;
                    }
                    ToggleId::AltGrAmberHints => {
                        state.config.layout.altgr_amber_hints =
                            !state.config.layout.altgr_amber_hints;
                    }
                    ToggleId::StickyModifierLock => {
                        state.config.behavior.sticky_modifier_lock =
                            !state.config.behavior.sticky_modifier_lock;
                    }
                    ToggleId::StartMinimized => {
                        state.config.behavior.start_minimized =
                            !state.config.behavior.start_minimized;
                    }
                }
                SettingsAction::ConfigChanged
            }
            ControlHit::ResetDefaults => {
                state.config.appearance = AppearanceSettings::default();
                state.config.active_preset = None;
                self.sync_from_appearance(&state.config.appearance);
                self.status_message = Some("Reset to Graphite Dark defaults".to_owned());
                SettingsAction::ConfigChanged
            }
            ControlHit::PanelPill(pid) => {
                state.set_panel(&pid);
                SettingsAction::ConfigChanged
            }
            ControlHit::OpenPanelEditor => SettingsAction::OpenPanelEditor,
            ControlHit::KeycapStyle(style) => {
                state.config.layout.keycap_style = style;
                SettingsAction::ConfigChanged
            }
            ControlHit::LayoutCard(code) => {
                if state.set_layout(&code) {
                    SettingsAction::LayoutChanged(code)
                } else {
                    SettingsAction::Redraw
                }
            }
            ControlHit::ToggleVisibleLang(code) => {
                if state.toggle_visible_language(&code) {
                    SettingsAction::ConfigChanged
                } else {
                    self.status_message =
                        Some("At least one language must remain visible".to_owned());
                    SettingsAction::Redraw
                }
            }
            ControlHit::MoveVisibleLang(code, delta) => {
                if state.move_visible_language(&code, delta) {
                    SettingsAction::ConfigChanged
                } else {
                    SettingsAction::Redraw
                }
            }
            ControlHit::QuitButton => SettingsAction::QuitApp,
        }
    }

    pub fn pointer_motion(&mut self, p: Point, state: &mut KeyboardState) -> SettingsAction {
        let Some(drag) = self.active_drag else {
            return SettingsAction::None;
        };
        match drag {
            ActiveDrag::SvSquare(rect) => self.apply_sv_drag(p, rect, state),
            ActiveDrag::HueBar(rect) => self.apply_hue_drag(p, rect, state),
            ActiveDrag::Slider(id, rect) => self.apply_slider_drag(id, p, rect, state),
        }
        SettingsAction::ConfigChanged
    }

    pub fn pointer_up(&mut self) {
        self.active_drag = None;
    }

    pub fn handle_key_input(
        &mut self,
        code: KeyCode,
        shift: bool,
        state: &mut KeyboardState,
        preset_store: &PresetStore,
    ) -> SettingsAction {
        let Some(field) = self.focused_field else {
            return SettingsAction::None;
        };

        if code == KeyCode::ESC {
            self.focused_field = None;
            return SettingsAction::Redraw;
        }
        if code == KeyCode::BACKSPACE {
            match field {
                FocusedField::PresetName => {
                    self.preset_name_input.pop();
                }
                FocusedField::HexColor => {
                    if self.hex_input.len() > 1 {
                        self.hex_input.pop();
                    }
                }
            }
            return SettingsAction::Redraw;
        }
        if code == KeyCode::ENTER {
            match field {
                FocusedField::PresetName => {
                    let name = self.preset_name_input.trim().to_owned();
                    if !name.is_empty()
                        && let Ok(saved) = preset_store.save(&name, &state.config.appearance)
                    {
                        state.config.active_preset = Some(saved.name.clone());
                        state.reload_presets(preset_store);
                        self.status_message = Some(format!("Saved preset \"{}\"", saved.name));
                        self.focused_field = None;
                        return SettingsAction::ConfigChanged;
                    }
                }
                FocusedField::HexColor => {
                    if let Some(col) = Color::from_hex(&self.hex_input) {
                        state.config.appearance.colors.set(self.selected_slot, col);
                        state.config.active_preset = None;
                        self.hsv = col.to_hsv();
                        self.hex_input = col.to_hex();
                        self.focused_field = None;
                        return SettingsAction::ConfigChanged;
                    }
                }
            }
            return SettingsAction::Redraw;
        }

        let Some(ch) = evdev_to_ascii(code, shift) else {
            return SettingsAction::None;
        };

        match field {
            FocusedField::PresetName => {
                if self.preset_name_input.len() < 28
                    && (ch.is_ascii_alphanumeric() || ch == ' ' || ch == '-' || ch == '_')
                {
                    self.preset_name_input.push(ch);
                    return SettingsAction::Redraw;
                }
            }
            FocusedField::HexColor => {
                let upper = ch.to_ascii_uppercase();
                if upper.is_ascii_hexdigit() && self.hex_input.len() < 7 {
                    if !self.hex_input.starts_with('#') {
                        self.hex_input.insert(0, '#');
                    }
                    self.hex_input.push(upper);
                    if self.hex_input.len() == 7
                        && let Some(col) = Color::from_hex(&self.hex_input)
                    {
                        state.config.appearance.colors.set(self.selected_slot, col);
                        state.config.active_preset = None;
                        self.hsv = col.to_hsv();
                        return SettingsAction::ConfigChanged;
                    }
                    return SettingsAction::Redraw;
                }
            }
        }
        SettingsAction::None
    }

    fn apply_sv_drag(&mut self, p: Point, rect: Rect, state: &mut KeyboardState) {
        let s = ((p.x - rect.x) / rect.w.max(1.0)).clamp(0.0, 1.0);
        let v = (1.0 - (p.y - rect.y) / rect.h.max(1.0)).clamp(0.0, 1.0);
        self.hsv.1 = s;
        self.hsv.2 = v;
        let col = Color::from_hsv(self.hsv.0, s, v);
        state.config.appearance.colors.set(self.selected_slot, col);
        state.config.active_preset = None;
        self.hex_input = col.to_hex();
    }

    fn apply_hue_drag(&mut self, p: Point, rect: Rect, state: &mut KeyboardState) {
        let h = ((p.x - rect.x) / rect.w.max(1.0)).clamp(0.0, 0.999) * 360.0;
        self.hsv.0 = h;
        let col = Color::from_hsv(h, self.hsv.1, self.hsv.2);
        state.config.appearance.colors.set(self.selected_slot, col);
        state.config.active_preset = None;
        self.hex_input = col.to_hex();
    }

    fn apply_slider_drag(&mut self, id: SliderId, p: Point, rect: Rect, state: &mut KeyboardState) {
        let t = ((p.x - rect.x) / rect.w.max(1.0)).clamp(0.0, 1.0);
        let app = &mut state.config.appearance;
        match id {
            SliderId::OutlineWidth => app.outline_width = (t * 4.0 * 2.0).round() / 2.0,
            SliderId::KeyBorderWidth => app.key_border_width = (t * 3.0 * 2.0).round() / 2.0,
            SliderId::Opacity => app.opacity = 0.30 + t * 0.70,
            SliderId::FontScale => app.font_scale = 0.75 + t * 0.85,
            SliderId::CornerRadius => app.corner_radius = (t * 16.0).round(),
            SliderId::KeySpacing => app.key_spacing = 1.0 + (t * 7.0).round(),
        }
        app.clamp();
        state.config.active_preset = None;
    }

    /// Render the complete themed `Customize wlboard` window and populate exact hit rects.
    pub fn render(
        &mut self,
        renderer: &mut UiRenderer,
        pixmap: &mut Pixmap,
        state: &KeyboardState,
        scale: f32,
    ) {
        self.hits.clear();

        let app = &state.config.appearance;
        let pal = app.resolved_palette();
        let w = self.width;
        let h = self.height;

        let card_bg = if pal.is_light {
            pal.colors.window_bg.adjust_brightness(0.35)
        } else {
            pal.colors.window_bg.adjust_brightness(0.05)
        };
        let card_border = pal.colors.key_border.with_alpha(0.65);

        UiRenderer::fill_rounded_rect(
            pixmap,
            Rect::new(0.0, 0.0, w, h),
            0.0,
            pal.colors.window_bg,
            scale,
        );

        let header_h = 54.0;
        UiRenderer::fill_rounded_rect(
            pixmap,
            Rect::new(0.0, 0.0, w, header_h),
            0.0,
            pal.toolbar_bg,
            scale,
        );
        UiRenderer::draw_hline(pixmap, 0.0, w, header_h, 1.0, card_border, scale);

        renderer.draw_icon(
            pixmap,
            IconId::Settings,
            Rect::new(14.0, 15.0, 24.0, 24.0),
            19.0,
            pal.colors.accent,
            scale,
        );
        renderer.draw_text(
            pixmap,
            "Customize wlboard",
            Rect::new(44.0, 0.0, 210.0, header_h),
            16.5,
            700,
            pal.colors.text_icon,
            TextAlign::Left,
            scale,
        );

        let close_rect = Rect::new(w - 40.0, 13.0, 28.0, 28.0);
        UiRenderer::fill_rounded_rect(pixmap, close_rect, 7.0, pal.colors.modifier_bg, scale);
        renderer.draw_icon(
            pixmap,
            IconId::Close,
            close_rect,
            14.0,
            pal.colors.text_icon,
            scale,
        );
        self.hits.push((close_rect, ControlHit::CloseButton));

        let tabs = [
            (SettingsTab::Appearance, "Appearance", 104.0),
            (SettingsTab::Layout, "Layout & Panels", 122.0),
            (SettingsTab::Behavior, "System", 78.0),
        ];
        let mut tx = close_rect.x - 14.0 - tabs.iter().map(|(_, _, tw)| tw + 6.0).sum::<f32>();
        for (tab_id, label, tw) in tabs {
            let r = Rect::new(tx, 12.0, tw, 30.0);
            let active = self.tab == tab_id;
            let bg = if active {
                pal.colors.accent
            } else {
                pal.colors.modifier_bg
            };
            let fg = if active {
                pal.accent_text
            } else {
                pal.colors.text_icon
            };
            UiRenderer::fill_rounded_rect(pixmap, r, 7.0, bg, scale);
            renderer.draw_text(
                pixmap,
                label,
                r,
                12.0,
                if active { 700 } else { 500 },
                fg,
                TextAlign::Center,
                scale,
            );
            self.hits.push((r, ControlHit::Tab(tab_id)));
            tx += tw + 6.0;
        }

        let content_x = 18.0;
        let content_w = (w - 36.0).max(200.0);
        let mut y = header_h + 14.0;

        match self.tab {
            SettingsTab::Appearance => {
                self.render_appearance_tab(
                    renderer,
                    pixmap,
                    state,
                    content_x,
                    content_w,
                    &mut y,
                    card_bg,
                    card_border,
                    scale,
                );
            }
            SettingsTab::Layout => {
                self.render_layout_tab(
                    renderer,
                    pixmap,
                    state,
                    content_x,
                    content_w,
                    &mut y,
                    card_bg,
                    card_border,
                    scale,
                );
            }
            SettingsTab::Behavior => {
                self.render_behavior_tab(
                    renderer,
                    pixmap,
                    state,
                    content_x,
                    content_w,
                    &mut y,
                    card_bg,
                    card_border,
                    scale,
                );
            }
        }

        if let Some(ref msg) = self.status_message {
            renderer.draw_text(
                pixmap,
                msg,
                Rect::new(content_x, h - 26.0, content_w, 20.0),
                11.5,
                500,
                pal.colors.accent,
                TextAlign::Left,
                scale,
            );
        }

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
    fn render_appearance_tab(
        &mut self,
        renderer: &mut UiRenderer,
        pixmap: &mut Pixmap,
        state: &KeyboardState,
        x: f32,
        w: f32,
        y: &mut f32,
        card_bg: Color,
        card_border: Color,
        scale: f32,
    ) {
        let app = &state.config.appearance;
        let pal = app.resolved_palette();

        // Card 1: My Presets
        let preset_card = Rect::new(x, *y, w, 92.0);
        UiRenderer::fill_rounded_rect(pixmap, preset_card, 10.0, card_bg, scale);
        UiRenderer::stroke_rounded_rect(pixmap, preset_card, 10.0, 1.0, card_border, scale);
        renderer.draw_text(
            pixmap,
            "My Presets",
            Rect::new(x + 12.0, *y + 8.0, 140.0, 18.0),
            12.5,
            700,
            pal.colors.text_icon,
            TextAlign::Left,
            scale,
        );

        let mut px = x + 12.0;
        let py = *y + 30.0;
        if state.presets.is_empty() {
            renderer.draw_text(
                pixmap,
                "No saved presets yet — customize colors below and save your own preset.",
                Rect::new(px, py, w - 24.0, 22.0),
                11.5,
                400,
                pal.muted_text,
                TextAlign::Left,
                scale,
            );
        } else {
            for preset in &state.presets {
                let pw = (preset.name.len() as f32 * 7.5 + 22.0).clamp(64.0, 150.0);
                if px + pw > x + w - 12.0 {
                    break;
                }
                let pr = Rect::new(px, py, pw, 22.0);
                let active = state.config.active_preset.as_deref() == Some(preset.name.as_str());
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
                    &preset.name,
                    pr.inset(2.0),
                    11.0,
                    if active { 700 } else { 500 },
                    if active {
                        pal.accent_text
                    } else {
                        pal.colors.text_icon
                    },
                    TextAlign::Center,
                    scale,
                );
                self.hits
                    .push((pr, ControlHit::SelectPreset(preset.name.clone())));
                px += pw + 6.0;
            }
        }

        let row_y = *y + 57.0;
        let save_w = 72.0;
        let del_w = 72.0;
        let input_w = (w - 24.0 - save_w - del_w - 14.0).max(120.0);
        let input_rect = Rect::new(x + 12.0, row_y, input_w, 26.0);
        let name_focused = self.focused_field == Some(FocusedField::PresetName);
        UiRenderer::fill_rounded_rect(pixmap, input_rect, 6.0, pal.colors.window_bg, scale);
        UiRenderer::stroke_rounded_rect(
            pixmap,
            input_rect,
            6.0,
            if name_focused { 1.8 } else { 1.0 },
            if name_focused {
                pal.colors.accent
            } else {
                card_border
            },
            scale,
        );
        let display_name = if self.preset_name_input.is_empty() && !name_focused {
            "Preset name (e.g. Midnight Cyan)".to_owned()
        } else if name_focused {
            format!("{}|", self.preset_name_input)
        } else {
            self.preset_name_input.clone()
        };
        let input_fg = if self.preset_name_input.is_empty() && !name_focused {
            pal.muted_text
        } else {
            pal.colors.text_icon
        };
        renderer.draw_text(
            pixmap,
            &display_name,
            Rect::new(
                input_rect.x + 8.0,
                input_rect.y,
                input_rect.w - 12.0,
                input_rect.h,
            ),
            11.5,
            400,
            input_fg,
            TextAlign::Left,
            scale,
        );
        self.hits.push((input_rect, ControlHit::PresetNameInput));

        let save_rect = Rect::new(input_rect.x + input_rect.w + 7.0, row_y, save_w, 26.0);
        UiRenderer::fill_rounded_rect(pixmap, save_rect, 6.0, pal.colors.accent, scale);
        renderer.draw_text(
            pixmap,
            "Save",
            save_rect,
            11.5,
            700,
            pal.accent_text,
            TextAlign::Center,
            scale,
        );
        self.hits.push((save_rect, ControlHit::SavePreset));

        let del_rect = Rect::new(save_rect.x + save_rect.w + 7.0, row_y, del_w, 26.0);
        UiRenderer::fill_rounded_rect(pixmap, del_rect, 6.0, pal.colors.modifier_bg, scale);
        UiRenderer::stroke_rounded_rect(pixmap, del_rect, 6.0, 1.0, card_border, scale);
        renderer.draw_text(
            pixmap,
            "Delete",
            del_rect,
            11.5,
            500,
            pal.colors.text_icon,
            TextAlign::Center,
            scale,
        );
        self.hits.push((del_rect, ControlHit::DeletePreset));

        *y += preset_card.h + 10.0;

        // Card 2: Theme Base & Quick Accents
        let base_card = Rect::new(x, *y, w, 92.0);
        UiRenderer::fill_rounded_rect(pixmap, base_card, 10.0, card_bg, scale);
        UiRenderer::stroke_rounded_rect(pixmap, base_card, 10.0, 1.0, card_border, scale);
        renderer.draw_text(
            pixmap,
            "Theme Base & Accent",
            Rect::new(x + 12.0, *y + 8.0, 200.0, 18.0),
            12.5,
            700,
            pal.colors.text_icon,
            TextAlign::Left,
            scale,
        );

        let base_w = (w - 24.0 - 18.0) / 4.0;
        for (idx, &base) in ThemeBase::ALL.iter().enumerate() {
            let br = Rect::new(
                x + 12.0 + idx as f32 * (base_w + 6.0),
                *y + 30.0,
                base_w,
                26.0,
            );
            let active = app.base == base;
            UiRenderer::fill_rounded_rect(
                pixmap,
                br,
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
                base.label(),
                br,
                11.0,
                if active { 700 } else { 500 },
                if active {
                    pal.accent_text
                } else {
                    pal.colors.text_icon
                },
                TextAlign::Center,
                scale,
            );
            self.hits.push((br, ControlHit::ThemeBase(base)));
        }

        renderer.draw_text(
            pixmap,
            "Accent:",
            Rect::new(x + 12.0, *y + 62.0, 56.0, 22.0),
            11.5,
            500,
            pal.muted_text,
            TextAlign::Left,
            scale,
        );
        for (idx, sw) in ACCENT_SWATCHES.iter().enumerate() {
            let cx = x + 82.0 + idx as f32 * 30.0;
            let cy = *y + 73.0;
            let sr = Rect::new(cx - 10.0, cy - 10.0, 20.0, 20.0);
            UiRenderer::fill_circle(pixmap, cx, cy, 9.5, sw.color, scale);
            if app.colors.accent == sw.color {
                UiRenderer::stroke_rounded_rect(
                    pixmap,
                    Rect::new(cx - 12.0, cy - 12.0, 24.0, 24.0),
                    12.0,
                    2.0,
                    pal.colors.text_icon,
                    scale,
                );
            }
            self.hits.push((sr, ControlHit::AccentSwatch(sw.color)));
        }

        *y += base_card.h + 10.0;

        // Card 3: 7 Custom Color Slots + Inline HSV/Hex Picker
        let color_card = Rect::new(x, *y, w, 174.0);
        UiRenderer::fill_rounded_rect(pixmap, color_card, 10.0, card_bg, scale);
        UiRenderer::stroke_rounded_rect(pixmap, color_card, 10.0, 1.0, card_border, scale);
        renderer.draw_text(
            pixmap,
            "Custom Palette (7 Color Slots)",
            Rect::new(x + 12.0, *y + 8.0, 260.0, 18.0),
            12.5,
            700,
            pal.colors.text_icon,
            TextAlign::Left,
            scale,
        );

        let slot_w = (w - 24.0 - 6.0 * 5.0) / 7.0;
        for (idx, &slot) in ColorSlot::ALL.iter().enumerate() {
            let sr = Rect::new(
                x + 12.0 + idx as f32 * (slot_w + 5.0),
                *y + 30.0,
                slot_w,
                36.0,
            );
            let selected = self.selected_slot == slot;
            UiRenderer::fill_rounded_rect(
                pixmap,
                sr,
                6.0,
                if selected {
                    pal.colors.key_bg
                } else {
                    pal.colors.modifier_bg
                },
                scale,
            );
            UiRenderer::stroke_rounded_rect(
                pixmap,
                sr,
                6.0,
                if selected { 1.8 } else { 1.0 },
                if selected {
                    pal.colors.accent
                } else {
                    card_border
                },
                scale,
            );
            let swatch = Rect::new(sr.x + 6.0, sr.y + 5.0, sr.w - 12.0, 12.0);
            UiRenderer::fill_rounded_rect(pixmap, swatch, 3.0, app.colors.get(slot), scale);
            UiRenderer::stroke_rounded_rect(pixmap, swatch, 3.0, 0.8, card_border, scale);
            renderer.draw_text(
                pixmap,
                slot.label(),
                Rect::new(sr.x + 2.0, sr.y + 19.0, sr.w - 4.0, 15.0),
                9.5,
                if selected { 700 } else { 500 },
                pal.colors.text_icon,
                TextAlign::Center,
                scale,
            );
            self.hits.push((sr, ControlHit::ColorSlotChip(slot)));
        }

        let sv_rect = Rect::new(x + 12.0, *y + 74.0, 240.0, 68.0);
        render_hsv_sv_square(pixmap, sv_rect, self.hsv.0, scale);
        UiRenderer::stroke_rounded_rect(pixmap, sv_rect, 4.0, 1.0, card_border, scale);
        let sv_hx = sv_rect.x + self.hsv.1 * sv_rect.w;
        let sv_hy = sv_rect.y + (1.0 - self.hsv.2) * sv_rect.h;
        UiRenderer::fill_circle(pixmap, sv_hx, sv_hy, 5.5, Color::rgb(255, 255, 255), scale);
        UiRenderer::fill_circle(
            pixmap,
            sv_hx,
            sv_hy,
            3.5,
            app.colors.get(self.selected_slot),
            scale,
        );
        self.hits.push((sv_rect, ControlHit::SvSquare(sv_rect)));

        let hue_rect = Rect::new(x + 12.0, *y + 148.0, 240.0, 16.0);
        render_hsv_hue_bar(pixmap, hue_rect, scale);
        UiRenderer::stroke_rounded_rect(pixmap, hue_rect, 4.0, 1.0, card_border, scale);
        let hue_hx = hue_rect.x + (self.hsv.0 / 360.0) * hue_rect.w;
        UiRenderer::fill_circle(
            pixmap,
            hue_hx,
            hue_rect.y + hue_rect.h * 0.5,
            7.0,
            Color::rgb(255, 255, 255),
            scale,
        );
        UiRenderer::fill_circle(
            pixmap,
            hue_hx,
            hue_rect.y + hue_rect.h * 0.5,
            4.5,
            Color::from_hsv(self.hsv.0, 1.0, 1.0),
            scale,
        );
        self.hits.push((hue_rect, ControlHit::HueBar(hue_rect)));

        let info_x = sv_rect.x + sv_rect.w + 18.0;
        let info_w = (x + w - 12.0 - info_x).max(120.0);
        let active_color = app.colors.get(self.selected_slot);
        renderer.draw_text(
            pixmap,
            &format!("Editing: {}", self.selected_slot.label()),
            Rect::new(info_x, *y + 76.0, info_w, 20.0),
            12.0,
            700,
            pal.colors.text_icon,
            TextAlign::Left,
            scale,
        );
        let preview_box = Rect::new(info_x, *y + 100.0, 56.0, 28.0);
        UiRenderer::fill_rounded_rect(pixmap, preview_box, 6.0, active_color, scale);
        UiRenderer::stroke_rounded_rect(pixmap, preview_box, 6.0, 1.2, card_border, scale);

        let hex_rect = Rect::new(info_x + 66.0, *y + 100.0, (info_w - 66.0).min(130.0), 28.0);
        let hex_focused = self.focused_field == Some(FocusedField::HexColor);
        UiRenderer::fill_rounded_rect(pixmap, hex_rect, 6.0, pal.colors.window_bg, scale);
        UiRenderer::stroke_rounded_rect(
            pixmap,
            hex_rect,
            6.0,
            if hex_focused { 1.8 } else { 1.0 },
            if hex_focused {
                pal.colors.accent
            } else {
                card_border
            },
            scale,
        );
        let hex_display = if hex_focused {
            format!("{}|", self.hex_input)
        } else {
            self.hex_input.clone()
        };
        renderer.draw_text(
            pixmap,
            &hex_display,
            Rect::new(hex_rect.x + 8.0, hex_rect.y, hex_rect.w - 12.0, hex_rect.h),
            12.0,
            600,
            pal.colors.text_icon,
            TextAlign::Left,
            scale,
        );
        self.hits.push((hex_rect, ControlHit::HexInput));

        renderer.draw_text(
            pixmap,
            "Drag square/hue or click hex to type #RRGGBB.",
            Rect::new(info_x, *y + 136.0, info_w, 26.0),
            10.5,
            400,
            pal.muted_text,
            TextAlign::Left,
            scale,
        );

        *y += color_card.h + 10.0;

        // Card 4: Borders, Geometry & Typography
        let geom_card = Rect::new(x, *y, w, 190.0);
        UiRenderer::fill_rounded_rect(pixmap, geom_card, 10.0, card_bg, scale);
        UiRenderer::stroke_rounded_rect(pixmap, geom_card, 10.0, 1.0, card_border, scale);
        renderer.draw_text(
            pixmap,
            "Borders, Geometry & Typography",
            Rect::new(x + 12.0, *y + 8.0, 260.0, 18.0),
            12.5,
            700,
            pal.colors.text_icon,
            TextAlign::Left,
            scale,
        );

        let col_w = (w - 36.0) * 0.5;
        let col1_x = x + 12.0;
        let col2_x = col1_x + col_w + 12.0;

        let sliders = [
            (
                col1_x,
                *y + 32.0,
                SliderId::OutlineWidth,
                "Outline Border",
                format!("{:.1} px", app.outline_width),
                app.outline_width / 4.0,
            ),
            (
                col2_x,
                *y + 32.0,
                SliderId::KeyBorderWidth,
                "Key Border",
                format!("{:.1} px", app.key_border_width),
                app.key_border_width / 3.0,
            ),
            (
                col1_x,
                *y + 72.0,
                SliderId::Opacity,
                "Window Opacity",
                format!("{}%", (app.opacity * 100.0).round() as u32),
                (app.opacity - 0.30) / 0.70,
            ),
            (
                col2_x,
                *y + 72.0,
                SliderId::FontScale,
                "Font Scale",
                format!("{}%", (app.font_scale * 100.0).round() as u32),
                (app.font_scale - 0.75) / 0.85,
            ),
            (
                col1_x,
                *y + 112.0,
                SliderId::CornerRadius,
                "Corner Radius",
                format!("{} px", app.corner_radius.round() as u32),
                app.corner_radius / 16.0,
            ),
            (
                col2_x,
                *y + 112.0,
                SliderId::KeySpacing,
                "Key Spacing",
                format!("{} px", app.key_spacing.round() as u32),
                (app.key_spacing - 1.0) / 7.0,
            ),
        ];

        for (sx, sy, id, label, value_str, frac) in sliders {
            self.draw_slider(
                renderer,
                pixmap,
                state,
                Rect::new(sx, sy, col_w, 34.0),
                id,
                label,
                &value_str,
                frac,
                scale,
            );
        }

        let bot_y = *y + 154.0;
        renderer.draw_text(
            pixmap,
            "Weight:",
            Rect::new(col1_x, bot_y, 50.0, 24.0),
            11.0,
            500,
            pal.muted_text,
            TextAlign::Left,
            scale,
        );
        for (idx, wt) in [400u16, 500, 600, 700].iter().enumerate() {
            let wr = Rect::new(col1_x + 54.0 + idx as f32 * 40.0, bot_y, 36.0, 24.0);
            let active = app.font_weight == *wt;
            UiRenderer::fill_rounded_rect(
                pixmap,
                wr,
                5.0,
                if active {
                    pal.colors.accent
                } else {
                    pal.colors.key_bg
                },
                scale,
            );
            renderer.draw_text(
                pixmap,
                &wt.to_string(),
                wr,
                10.5,
                *wt,
                if active {
                    pal.accent_text
                } else {
                    pal.colors.text_icon
                },
                TextAlign::Center,
                scale,
            );
            self.hits.push((wr, ControlHit::FontWeight(*wt)));
        }

        renderer.draw_text(
            pixmap,
            "Keycap Shadow",
            Rect::new(col2_x, bot_y, 108.0, 24.0),
            11.5,
            500,
            pal.colors.text_icon,
            TextAlign::Left,
            scale,
        );
        let sh_switch = Rect::new(col2_x + 108.0, bot_y + 2.0, 40.0, 20.0);
        draw_toggle_pill(
            pixmap,
            sh_switch,
            app.keycap_shadow,
            pal.colors.accent,
            pal.colors.modifier_bg,
            scale,
        );
        self.hits
            .push((sh_switch, ControlHit::Toggle(ToggleId::KeycapShadow)));

        let reset_rect = Rect::new(x + w - 122.0, bot_y, 110.0, 24.0);
        UiRenderer::fill_rounded_rect(pixmap, reset_rect, 6.0, pal.colors.modifier_bg, scale);
        UiRenderer::stroke_rounded_rect(pixmap, reset_rect, 6.0, 1.0, card_border, scale);
        renderer.draw_text(
            pixmap,
            "Reset Defaults",
            reset_rect,
            11.0,
            600,
            pal.colors.text_icon,
            TextAlign::Center,
            scale,
        );
        self.hits.push((reset_rect, ControlHit::ResetDefaults));

        *y += geom_card.h + 10.0;
    }

    #[allow(clippy::too_many_arguments)]
    fn render_layout_tab(
        &mut self,
        renderer: &mut UiRenderer,
        pixmap: &mut Pixmap,
        state: &KeyboardState,
        x: f32,
        w: f32,
        y: &mut f32,
        card_bg: Color,
        card_border: Color,
        scale: f32,
    ) {
        let pal = state.config.appearance.resolved_palette();

        // --- Card 1: Panels & Keycap Label Style ---
        let panel_card = Rect::new(x, *y, w, 128.0);
        UiRenderer::fill_rounded_rect(pixmap, panel_card, 10.0, card_bg, scale);
        UiRenderer::stroke_rounded_rect(pixmap, panel_card, 10.0, 1.0, card_border, scale);

        renderer.draw_text(
            pixmap,
            "Active Panel & Visual Layout Editor",
            Rect::new(x + 14.0, *y + 10.0, 280.0, 20.0),
            12.5,
            700,
            pal.colors.text_icon,
            TextAlign::Left,
            scale,
        );

        let editor_btn = Rect::new(x + w - 198.0, *y + 8.0, 184.0, 28.0);
        UiRenderer::fill_rounded_rect(pixmap, editor_btn, 7.0, pal.colors.accent, scale);
        renderer.draw_icon(
            pixmap,
            IconId::Panels,
            Rect::new(editor_btn.x + 10.0, editor_btn.y, 18.0, 28.0),
            14.0,
            pal.accent_text,
            scale,
        );
        renderer.draw_text(
            pixmap,
            "Open Panel Editor...",
            Rect::new(editor_btn.x + 28.0, editor_btn.y, editor_btn.w - 34.0, 28.0),
            11.5,
            700,
            pal.accent_text,
            TextAlign::Center,
            scale,
        );
        self.hits.push((editor_btn, ControlHit::OpenPanelEditor));

        // Panel selector pills
        let mut px = x + 14.0;
        let py = *y + 44.0;
        for panel in &state.panels {
            let pw = (panel.name.len() as f32 * 7.6 + 26.0).clamp(76.0, 158.0);
            if px + pw > x + w - 14.0 {
                break;
            }
            let pr = Rect::new(px, py, pw, 28.0);
            let active = state.config.layout.active_panel == panel.id;
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
                pr,
                11.0,
                if active { 700 } else { 500 },
                if active {
                    pal.accent_text
                } else {
                    pal.colors.text_icon
                },
                TextAlign::Center,
                scale,
            );
            self.hits
                .push((pr, ControlHit::PanelPill(panel.id.clone())));
            px += pw + 8.0;
        }

        // Keycap Style segmented toggle (Text vs Monochrome Symbols)
        let ks_y = *y + 86.0;
        renderer.draw_text(
            pixmap,
            "Modifier & Action Keycaps:",
            Rect::new(x + 14.0, ks_y, 185.0, 28.0),
            11.5,
            600,
            pal.colors.text_icon,
            TextAlign::Left,
            scale,
        );
        let styles = [
            (KeycapStyle::Text, "Text Labels (Tab, Shift, Del)"),
            (KeycapStyle::Symbols, "Monochrome SVG Symbols"),
        ];
        let sw_w = (w - 220.0) * 0.5;
        for (idx, (st, lbl)) in styles.iter().enumerate() {
            let r = Rect::new(x + 204.0 + idx as f32 * (sw_w + 8.0), ks_y, sw_w, 28.0);
            let active = state.config.layout.keycap_style == *st;
            UiRenderer::fill_rounded_rect(
                pixmap,
                r,
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
                lbl,
                r,
                11.0,
                if active { 700 } else { 500 },
                if active {
                    pal.accent_text
                } else {
                    pal.colors.text_icon
                },
                TextAlign::Center,
                scale,
            );
            self.hits.push((r, ControlHit::KeycapStyle(*st)));
        }

        *y += panel_card.h + 10.0;

        // --- Card 2: Customizable Fast Language Switcher (Show/Hide & Reorder) ---
        let lang_card = Rect::new(x, *y, w, 152.0);
        UiRenderer::fill_rounded_rect(pixmap, lang_card, 10.0, card_bg, scale);
        UiRenderer::stroke_rounded_rect(pixmap, lang_card, 10.0, 1.0, card_border, scale);
        renderer.draw_text(
            pixmap,
            "Fast Language Switcher (Select Active & Visible Languages)",
            Rect::new(x + 14.0, *y + 10.0, w - 28.0, 18.0),
            12.5,
            700,
            pal.colors.text_icon,
            TextAlign::Left,
            scale,
        );

        let lw = (w - 28.0 - 4.0 * 8.0) / 5.0;
        for (idx, code) in BUILTIN_LAYOUT_CODES.iter().enumerate() {
            let def = state.registry.get(code);
            let lx = x + 14.0 + idx as f32 * (lw + 8.0);
            let is_active = state.config.layout.active == def.code;
            let is_visible = state
                .config
                .layout
                .visible_languages
                .iter()
                .any(|c| c == &def.code);

            // Top card selects active language
            let top_r = Rect::new(lx, *y + 34.0, lw, 48.0);
            UiRenderer::fill_rounded_rect(
                pixmap,
                top_r,
                7.0,
                if is_active {
                    pal.colors.accent
                } else {
                    pal.colors.key_bg
                },
                scale,
            );
            let fg = if is_active {
                pal.accent_text
            } else {
                pal.colors.text_icon
            };
            renderer.draw_text(
                pixmap,
                &def.short_label,
                Rect::new(top_r.x, top_r.y + 6.0, top_r.w, 20.0),
                13.5,
                700,
                fg,
                TextAlign::Center,
                scale,
            );
            let short_name = def
                .display_name
                .split(' ')
                .next()
                .unwrap_or(&def.display_name);
            renderer.draw_text(
                pixmap,
                short_name,
                Rect::new(top_r.x, top_r.y + 26.0, top_r.w, 16.0),
                10.0,
                400,
                if is_active {
                    pal.accent_text.with_alpha(0.88)
                } else {
                    pal.muted_text
                },
                TextAlign::Center,
                scale,
            );
            self.hits
                .push((top_r, ControlHit::LayoutCard(def.code.clone())));

            // Bottom pill toggles visibility in top Fast Language Switcher (Shown vs Hidden)
            let vis_r = Rect::new(lx, *y + 88.0, lw, 24.0);
            UiRenderer::fill_rounded_rect(
                pixmap,
                vis_r,
                5.0,
                if is_visible {
                    pal.colors.modifier_bg
                } else {
                    pal.colors.window_bg
                },
                scale,
            );
            UiRenderer::stroke_rounded_rect(
                pixmap,
                vis_r,
                5.0,
                1.0,
                if is_visible {
                    pal.colors.accent
                } else {
                    card_border
                },
                scale,
            );
            renderer.draw_text(
                pixmap,
                if is_visible { "Shown" } else { "Hidden" },
                vis_r,
                10.0,
                if is_visible { 600 } else { 400 },
                if is_visible {
                    pal.colors.text_icon
                } else {
                    pal.muted_text
                },
                TextAlign::Center,
                scale,
            );
            self.hits
                .push((vis_r, ControlHit::ToggleVisibleLang(def.code.clone())));
        }

        // Reorder strip showing exact live top-bar order with left/right arrows
        let ord_y = *y + 120.0;
        renderer.draw_text(
            pixmap,
            "Top-bar order:",
            Rect::new(x + 14.0, ord_y, 92.0, 22.0),
            10.5,
            600,
            pal.muted_text,
            TextAlign::Left,
            scale,
        );
        let mut ox = x + 110.0;
        let vis_list = state.config.layout.visible_languages.clone();
        for (pos, code) in vis_list.iter().enumerate() {
            let short = &state.registry.get(code).short_label;
            let pill_w = 78.0;
            let pr = Rect::new(ox, ord_y, pill_w, 22.0);
            UiRenderer::fill_rounded_rect(pixmap, pr, 5.0, pal.colors.modifier_bg, scale);
            UiRenderer::stroke_rounded_rect(pixmap, pr, 5.0, 1.0, card_border, scale);

            let left_r = Rect::new(ox, ord_y, 22.0, 22.0);
            let right_r = Rect::new(ox + pill_w - 22.0, ord_y, 22.0, 22.0);
            let mid_r = Rect::new(ox + 22.0, ord_y, pill_w - 44.0, 22.0);

            if pos > 0 {
                renderer.draw_icon(
                    pixmap,
                    IconId::ArrowLeft,
                    left_r,
                    11.0,
                    pal.colors.text_icon,
                    scale,
                );
                self.hits
                    .push((left_r, ControlHit::MoveVisibleLang(code.clone(), -1)));
            }
            renderer.draw_text(
                pixmap,
                short,
                mid_r,
                10.5,
                700,
                pal.colors.text_icon,
                TextAlign::Center,
                scale,
            );
            if pos + 1 < vis_list.len() {
                renderer.draw_icon(
                    pixmap,
                    IconId::ArrowRight,
                    right_r,
                    11.0,
                    pal.colors.text_icon,
                    scale,
                );
                self.hits
                    .push((right_r, ControlHit::MoveVisibleLang(code.clone(), 1)));
            }
            ox += pill_w + 8.0;
        }

        *y += lang_card.h + 10.0;

        // --- Card 3: Toolbars & KDE Integration ---
        let tb_card = Rect::new(x, *y, w, 256.0);
        UiRenderer::fill_rounded_rect(pixmap, tb_card, 10.0, card_bg, scale);
        UiRenderer::stroke_rounded_rect(pixmap, tb_card, 10.0, 1.0, card_border, scale);
        renderer.draw_text(
            pixmap,
            "Toolbars & KDE Plasma Integration",
            Rect::new(x + 14.0, *y + 10.0, 280.0, 18.0),
            12.5,
            700,
            pal.colors.text_icon,
            TextAlign::Left,
            scale,
        );

        let toggles = [
            (
                ToggleId::SystemLayoutSync,
                "KDE Plasma Two-Way Layout Sync",
                "Keep active language synchronized with Plasma desktop keyboard indicator",
                state.config.layout.system_layout_sync,
            ),
            (
                ToggleId::AutoRegisterKdeLayouts,
                "Auto-Add Missing Languages to KDE Settings",
                "Automatically register unconfigured layouts in kxkbrc when switching",
                state.config.layout.auto_register_kde_layouts,
            ),
            (
                ToggleId::FastSwitcher,
                "Show Fast Language Switcher Bar",
                "Display your selected languages in the top control bar",
                state.config.layout.show_fast_switcher,
            ),
            (
                ToggleId::PanelSwitcher,
                "Show Panel Switcher Pill",
                "Quick 1-click button to cycle between Standard, Compact TV, Numpad & custom panels",
                state.config.layout.show_panel_switcher,
            ),
            (
                ToggleId::MediaRow,
                "System & Media Controls Row",
                "Display Brightness, Media Playback, Volume & Screenshot row",
                state.config.layout.show_media_row,
            ),
            (
                ToggleId::AltGrAmberHints,
                "AltGr Diacritic Amber Hints",
                "Highlight Polish & alternate diacritic keys in warm amber",
                state.config.layout.altgr_amber_hints,
            ),
        ];

        for (idx, (id, title, subtitle, value)) in toggles.into_iter().enumerate() {
            let ry = *y + 34.0 + idx as f32 * 36.0;
            self.draw_toggle_row(
                renderer,
                pixmap,
                state,
                Rect::new(x + 14.0, ry, w - 28.0, 34.0),
                id,
                title,
                subtitle,
                value,
                scale,
            );
        }

        *y += tb_card.h + 10.0;
    }

    #[allow(clippy::too_many_arguments)]
    fn render_behavior_tab(
        &mut self,
        renderer: &mut UiRenderer,
        pixmap: &mut Pixmap,
        state: &KeyboardState,
        x: f32,
        w: f32,
        y: &mut f32,
        card_bg: Color,
        card_border: Color,
        scale: f32,
    ) {
        let pal = state.config.appearance.resolved_palette();

        // Card 1: Typing Behavior
        let beh_card = Rect::new(x, *y, w, 124.0);
        UiRenderer::fill_rounded_rect(pixmap, beh_card, 10.0, card_bg, scale);
        UiRenderer::stroke_rounded_rect(pixmap, beh_card, 10.0, 1.0, card_border, scale);
        renderer.draw_text(
            pixmap,
            "Input & Startup Behavior",
            Rect::new(x + 14.0, *y + 10.0, 240.0, 20.0),
            13.0,
            700,
            pal.colors.text_icon,
            TextAlign::Left,
            scale,
        );
        self.draw_toggle_row(
            renderer,
            pixmap,
            state,
            Rect::new(x + 14.0, *y + 36.0, w - 28.0, 38.0),
            ToggleId::StickyModifierLock,
            "Sticky Modifier Lock (Double-Tap)",
            "Double-tap Shift, Ctrl, Alt, or AltGr to lock active until tapped again",
            state.config.behavior.sticky_modifier_lock,
            scale,
        );
        self.draw_toggle_row(
            renderer,
            pixmap,
            state,
            Rect::new(x + 14.0, *y + 78.0, w - 28.0, 38.0),
            ToggleId::StartMinimized,
            "Start Minimized to System Tray",
            "Launch quietly into the StatusNotifier tray until toggled",
            state.config.behavior.start_minimized,
            scale,
        );
        *y += beh_card.h + 12.0;

        // Card 2: Wayland & Lockscreen Diagnostics
        let diag_card = Rect::new(x, *y, w, 178.0);
        UiRenderer::fill_rounded_rect(pixmap, diag_card, 10.0, card_bg, scale);
        UiRenderer::stroke_rounded_rect(pixmap, diag_card, 10.0, 1.0, card_border, scale);
        renderer.draw_text(
            pixmap,
            "Wayland, Input & Lockscreen Integration",
            Rect::new(x + 14.0, *y + 10.0, 340.0, 20.0),
            13.0,
            700,
            pal.colors.text_icon,
            TextAlign::Left,
            scale,
        );

        let rows = [
            ("Input Backend:", state.input_backend_status.as_str()),
            ("Layout Sync:", state.layout_sync_status.as_str()),
            (
                "Wayland Layer:",
                if state.input_method_mode {
                    "zwp_input_panel_v1 (Lockscreen / Input Method Mode)"
                } else {
                    "zwlr_layer_shell_v1 (Floating Non-Focus-Stealing Overlay)"
                },
            ),
            (
                "KDE Lockscreen:",
                "System Settings -> Keyboard -> Virtual Keyboard -> wlboard",
            ),
            (
                "SDDM Login:",
                "Run /usr/share/wlboard/scripts/setup-sddm.sh (optional)",
            ),
        ];
        for (idx, (k, v)) in rows.iter().enumerate() {
            let ry = *y + 38.0 + idx as f32 * 26.0;
            renderer.draw_text(
                pixmap,
                k,
                Rect::new(x + 14.0, ry, 126.0, 22.0),
                11.5,
                600,
                pal.muted_text,
                TextAlign::Left,
                scale,
            );
            renderer.draw_text(
                pixmap,
                v,
                Rect::new(x + 142.0, ry, w - 156.0, 22.0),
                11.5,
                500,
                pal.colors.text_icon,
                TextAlign::Left,
                scale,
            );
        }
        *y += diag_card.h + 12.0;

        // Card 3: About & Quit
        let about_card = Rect::new(x, *y, w, 124.0);
        UiRenderer::fill_rounded_rect(pixmap, about_card, 10.0, card_bg, scale);
        UiRenderer::stroke_rounded_rect(pixmap, about_card, 10.0, 1.0, card_border, scale);
        renderer.draw_text(
            pixmap,
            "About wlboard",
            Rect::new(x + 14.0, *y + 10.0, 200.0, 20.0),
            13.0,
            700,
            pal.colors.text_icon,
            TextAlign::Left,
            scale,
        );
        renderer.draw_text(
            pixmap,
            "wlboard v0.2.0 — Native Wayland floating virtual keyboard & Panel Editor in Rust",
            Rect::new(x + 14.0, *y + 36.0, w - 28.0, 20.0),
            11.5,
            500,
            pal.colors.text_icon,
            TextAlign::Left,
            scale,
        );
        renderer.draw_text(
            pixmap,
            "Author: Aleksander Kutycki (olekkut)   |   License: GPL-3.0-or-later   |   github.com/olekkut/wlboard",
            Rect::new(x + 14.0, *y + 58.0, w - 28.0, 20.0),
            11.0,
            400,
            pal.muted_text,
            TextAlign::Left,
            scale,
        );

        let quit_rect = Rect::new(x + w - 134.0, *y + 84.0, 120.0, 28.0);
        let danger = Color::rgb(0xDC, 0x26, 0x26);
        UiRenderer::fill_rounded_rect(pixmap, quit_rect, 7.0, danger, scale);
        renderer.draw_text(
            pixmap,
            "Quit wlboard",
            quit_rect,
            11.5,
            700,
            Color::rgb(255, 255, 255),
            TextAlign::Center,
            scale,
        );
        self.hits.push((quit_rect, ControlHit::QuitButton));
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_slider(
        &mut self,
        renderer: &mut UiRenderer,
        pixmap: &mut Pixmap,
        state: &KeyboardState,
        rect: Rect,
        id: SliderId,
        label: &str,
        value_str: &str,
        fraction: f32,
        scale: f32,
    ) {
        let pal = state.config.appearance.resolved_palette();
        let frac = fraction.clamp(0.0, 1.0);

        renderer.draw_text(
            pixmap,
            label,
            Rect::new(rect.x, rect.y, rect.w * 0.65, 16.0),
            11.0,
            500,
            pal.colors.text_icon,
            TextAlign::Left,
            scale,
        );
        renderer.draw_text(
            pixmap,
            value_str,
            Rect::new(rect.x + rect.w * 0.65, rect.y, rect.w * 0.35, 16.0),
            11.0,
            600,
            pal.colors.accent,
            TextAlign::Right,
            scale,
        );

        let track = Rect::new(rect.x, rect.y + 21.0, rect.w, 6.0);
        UiRenderer::fill_rounded_rect(pixmap, track, 3.0, pal.colors.modifier_bg, scale);
        let filled = Rect::new(track.x, track.y, (track.w * frac).max(6.0), track.h);
        UiRenderer::fill_rounded_rect(pixmap, filled, 3.0, pal.colors.accent, scale);

        let knob_x = track.x + track.w * frac;
        let knob_y = track.y + track.h * 0.5;
        UiRenderer::fill_circle(pixmap, knob_x, knob_y, 7.0, pal.colors.text_icon, scale);
        UiRenderer::fill_circle(pixmap, knob_x, knob_y, 4.5, pal.colors.accent, scale);

        let hit_rect = Rect::new(track.x - 4.0, track.y - 8.0, track.w + 8.0, 22.0);
        self.hits.push((hit_rect, ControlHit::Slider(id, track)));
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_toggle_row(
        &mut self,
        renderer: &mut UiRenderer,
        pixmap: &mut Pixmap,
        state: &KeyboardState,
        rect: Rect,
        id: ToggleId,
        title: &str,
        subtitle: &str,
        value: bool,
        scale: f32,
    ) {
        let pal = state.config.appearance.resolved_palette();
        renderer.draw_text(
            pixmap,
            title,
            Rect::new(rect.x, rect.y, rect.w - 58.0, 17.0),
            11.5,
            600,
            pal.colors.text_icon,
            TextAlign::Left,
            scale,
        );
        renderer.draw_text(
            pixmap,
            subtitle,
            Rect::new(rect.x, rect.y + 17.0, rect.w - 58.0, 15.0),
            10.0,
            400,
            pal.muted_text,
            TextAlign::Left,
            scale,
        );

        let switch_rect = Rect::new(rect.x + rect.w - 44.0, rect.y + 6.0, 42.0, 20.0);
        draw_toggle_pill(
            pixmap,
            switch_rect,
            value,
            pal.colors.accent,
            pal.colors.modifier_bg,
            scale,
        );
        self.hits.push((rect, ControlHit::Toggle(id)));
    }
}

fn draw_toggle_pill(
    pixmap: &mut Pixmap,
    rect: Rect,
    enabled: bool,
    accent: Color,
    off_bg: Color,
    scale: f32,
) {
    let r = rect.h * 0.5;
    UiRenderer::fill_rounded_rect(
        pixmap,
        rect,
        r,
        if enabled { accent } else { off_bg },
        scale,
    );
    let knob_x = if enabled {
        rect.x + rect.w - r
    } else {
        rect.x + r
    };
    let knob_y = rect.y + r;
    UiRenderer::fill_circle(
        pixmap,
        knob_x,
        knob_y,
        (r - 2.5).max(4.0),
        Color::rgb(255, 255, 255),
        scale,
    );
}

fn render_hsv_sv_square(pixmap: &mut Pixmap, rect: Rect, hue: f32, scale: f32) {
    let w = (rect.w * scale).round().max(1.0) as u32;
    let h = (rect.h * scale).round().max(1.0) as u32;
    let Some(mut sub) = Pixmap::new(w, h) else {
        return;
    };
    let data = sub.data_mut();
    for py in 0..h {
        let v = 1.0 - (py as f32 / (h.saturating_sub(1).max(1)) as f32);
        for px in 0..w {
            let s = px as f32 / (w.saturating_sub(1).max(1)) as f32;
            let c = Color::from_hsv(hue, s, v);
            let idx = ((py * w + px) * 4) as usize;
            data[idx] = c.r;
            data[idx + 1] = c.g;
            data[idx + 2] = c.b;
            data[idx + 3] = 255;
        }
    }
    pixmap.draw_pixmap(
        (rect.x * scale).round() as i32,
        (rect.y * scale).round() as i32,
        sub.as_ref(),
        &tiny_skia::PixmapPaint::default(),
        Transform::identity(),
        None,
    );
}

fn render_hsv_hue_bar(pixmap: &mut Pixmap, rect: Rect, scale: f32) {
    let w = (rect.w * scale).round().max(1.0) as u32;
    let h = (rect.h * scale).round().max(1.0) as u32;
    let Some(mut sub) = Pixmap::new(w, h) else {
        return;
    };
    let data = sub.data_mut();
    for px in 0..w {
        let hue = (px as f32 / (w.saturating_sub(1).max(1)) as f32) * 360.0;
        let c = Color::from_hsv(hue, 1.0, 1.0);
        for py in 0..h {
            let idx = ((py * w + px) * 4) as usize;
            data[idx] = c.r;
            data[idx + 1] = c.g;
            data[idx + 2] = c.b;
            data[idx + 3] = 255;
        }
    }
    pixmap.draw_pixmap(
        (rect.x * scale).round() as i32,
        (rect.y * scale).round() as i32,
        sub.as_ref(),
        &tiny_skia::PixmapPaint::default(),
        Transform::identity(),
        None,
    );
}

/// Map standard ASCII evdev keycodes for inline text inputs across Settings and Panel Editor.
pub(crate) fn evdev_to_ascii(code: KeyCode, shift: bool) -> Option<char> {
    let ch = match code {
        KeyCode::A => 'a',
        KeyCode::B => 'b',
        KeyCode::C => 'c',
        KeyCode::D => 'd',
        KeyCode::E => 'e',
        KeyCode::F => 'f',
        KeyCode::G => 'g',
        KeyCode::H => 'h',
        KeyCode::I => 'i',
        KeyCode::J => 'j',
        KeyCode::K => 'k',
        KeyCode::L => 'l',
        KeyCode::M => 'm',
        KeyCode::N => 'n',
        KeyCode::O => 'o',
        KeyCode::P => 'p',
        KeyCode::Q => 'q',
        KeyCode::R => 'r',
        KeyCode::S => 's',
        KeyCode::T => 't',
        KeyCode::U => 'u',
        KeyCode::V => 'v',
        KeyCode::W => 'w',
        KeyCode::X => 'x',
        KeyCode::Y => 'y',
        KeyCode::Z => 'z',
        KeyCode::KEY_0 => return Some(if shift { ')' } else { '0' }),
        KeyCode::KEY_1 => return Some(if shift { '!' } else { '1' }),
        KeyCode::KEY_2 => return Some(if shift { '@' } else { '2' }),
        KeyCode::KEY_3 => return Some(if shift { '#' } else { '3' }),
        KeyCode::KEY_4 => return Some(if shift { '$' } else { '4' }),
        KeyCode::KEY_5 => return Some(if shift { '%' } else { '5' }),
        KeyCode::KEY_6 => return Some(if shift { '^' } else { '6' }),
        KeyCode::KEY_7 => return Some(if shift { '&' } else { '7' }),
        KeyCode::KEY_8 => return Some(if shift { '*' } else { '8' }),
        KeyCode::KEY_9 => return Some(if shift { '(' } else { '9' }),
        KeyCode::SPACE => ' ',
        KeyCode::MINUS => return Some(if shift { '_' } else { '-' }),
        KeyCode::EQUAL => return Some(if shift { '+' } else { '=' }),
        KeyCode::COMMA => return Some(if shift { '<' } else { ',' }),
        KeyCode::DOT => return Some(if shift { '>' } else { '.' }),
        KeyCode::SLASH => return Some(if shift { '?' } else { '/' }),
        KeyCode::SEMICOLON => return Some(if shift { ':' } else { ';' }),
        _ => return None,
    };
    Some(if shift && ch.is_ascii_alphabetic() {
        ch.to_ascii_uppercase()
    } else {
        ch
    })
}
