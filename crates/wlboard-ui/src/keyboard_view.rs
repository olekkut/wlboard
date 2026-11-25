//! Floating keyboard overlay 2D snap-grid solver, hit-testing, and `tiny-skia` renderer.

use tiny_skia::Pixmap;
use wlboard_core::{
    GridKeyAction, GridKeySpec, IconId, KeyCode, KeyRole, KeyboardState, KeyboardStyle,
    KeycapStyle, ModifierKind, Point, Rect, ResizeEdge, ResolvedGridKey, detect_resize_edge,
    media_control_row,
};

use crate::canvas::{TextAlign, UiRenderer};

/// Logical hit target produced by pointer or touch hit-testing on the keyboard surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyboardHit {
    Resize(ResizeEdge),
    ToolbarEsc,
    ToolbarLang(String),
    ToolbarPanelCycle,
    ToolbarOptions,
    ToolbarHide,
    DragHeader,
    Key(usize),
}

/// Positioned keycap inside the keyboard overlay.
#[derive(Debug, Clone)]
pub struct PositionedKey {
    pub rect: Rect,
    pub resolved: ResolvedGridKey,
}

/// Positioned toolbar pill button.
#[derive(Debug, Clone)]
pub struct ToolbarButton {
    pub rect: Rect,
    pub label: String,
    pub code: String,
    pub active: bool,
}

/// Complete laid-out geometry for a single frame of the floating keyboard overlay.
#[derive(Debug, Clone)]
pub struct KeyboardViewLayout {
    pub width: f32,
    pub height: f32,
    pub header_rect: Rect,
    pub esc_rect: Rect,
    pub lang_buttons: Vec<ToolbarButton>,
    pub panel_switcher_rect: Option<Rect>,
    pub panel_switcher_label: String,
    pub options_rect: Rect,
    pub hide_rect: Rect,
    pub keys: Vec<PositionedKey>,
    pub resize_grip_rect: Rect,
    pub outer_radius: f32,
}

impl KeyboardViewLayout {
    /// Compute responsive 2D grid rects for the active panel, visible languages, and logical dimensions.
    #[must_use]
    pub fn compute(state: &KeyboardState, width: f32, height: f32) -> Self {
        let app = &state.config.appearance;
        let outer_pad = (app.outline_width + 7.0).max(8.0);
        let gap = app.key_spacing;
        let outer_radius = (app.corner_radius + 6.0).clamp(6.0, 20.0);

        let header_h = (height * 0.125).clamp(32.0, 42.0);
        let header_rect = Rect::new(
            outer_pad,
            outer_pad - 1.0,
            (width - outer_pad * 2.0).max(0.0),
            header_h,
        );

        let btn_h = (header_h - 6.0).max(22.0);
        let btn_y = header_rect.y + (header_h - btn_h) * 0.5;

        // Left: ESC button
        let esc_w = (btn_h * 1.65).clamp(42.0, 56.0);
        let esc_rect = Rect::new(header_rect.x, btn_y, esc_w, btn_h);

        // Right: Hide icon button + Options button
        let hide_w = btn_h;
        let hide_rect = Rect::new(header_rect.x + header_rect.w - hide_w, btn_y, hide_w, btn_h);
        let options_w = if width >= 540.0 { 92.0 } else { btn_h + 6.0 };
        let options_rect = Rect::new(hide_rect.x - options_w - 6.0, btn_y, options_w, btn_h);

        // Customizable Fast Language Switcher pills (respecting user's visible_languages + order!)
        let mut lang_buttons = Vec::new();
        let mut cursor_x = esc_rect.x + esc_rect.w + 8.0;
        let max_left_x = options_rect.x - 16.0;

        if state.config.layout.show_fast_switcher {
            let pill_w = (btn_h * 1.25).clamp(32.0, 44.0);
            for def in state.visible_layouts() {
                if cursor_x + pill_w > max_left_x {
                    break;
                }
                lang_buttons.push(ToolbarButton {
                    rect: Rect::new(cursor_x, btn_y, pill_w, btn_h),
                    label: def.short_label.clone(),
                    code: def.code.clone(),
                    active: state.config.layout.active == def.code,
                });
                cursor_x += pill_w + 4.0;
            }
        }

        // Optional Panel Switcher pill right after language buttons
        let (panel_def, resolved_keys) = state.active_resolved_keys();
        let panel_switcher_label = panel_def.name.clone();
        let mut panel_switcher_rect = None;
        if state.config.layout.show_panel_switcher {
            let pill_w = (panel_switcher_label.len() as f32 * 8.4 + 42.0).clamp(96.0, 196.0);
            let px = cursor_x + 6.0;
            if px + pill_w <= max_left_x {
                panel_switcher_rect = Some(Rect::new(px, btn_y, pill_w, btn_h));
            }
        }

        // Compute content grid region (optional Media Row + 2D Panel Grid)
        let mut grid_top = header_rect.y + header_rect.h + 6.0;
        let grid_bottom = height - outer_pad;
        let grid_w = (width - outer_pad * 2.0).max(100.0);
        let mut keys = Vec::with_capacity(resolved_keys.len() + 10);

        if state.config.layout.show_media_row {
            let media_specs = media_control_row();
            let total_avail_h = (grid_bottom - grid_top).max(60.0);
            let media_h = (total_avail_h / (panel_def.rows + 0.72) * 0.72).clamp(24.0, 44.0);
            let total_w: f32 = media_specs.iter().map(|k| k.weight).sum();
            let h_gaps = gap * (media_specs.len().saturating_sub(1) as f32);
            let avail_w = (grid_w - h_gaps).max(20.0);
            let mut mx = outer_pad;

            for spec in media_specs {
                let kw = avail_w * (spec.weight / total_w.max(1.0));
                let kc = match spec.action {
                    wlboard_core::KeyAction::Key(c) => c,
                    wlboard_core::KeyAction::ToggleCompactPage => KeyCode::ESC,
                };
                let grid_spec = GridKeySpec {
                    id: spec.id,
                    x: 0.0,
                    y: 0.0,
                    w: spec.weight,
                    h: 1.0,
                    action: GridKeyAction::FixedKey { keycode: kc },
                    role: KeyRole::Media,
                    label: spec.normal.clone(),
                    shift_label: String::new(),
                    icon: spec.icon,
                    symbol_icon: spec.icon,
                    color_override: None,
                };
                keys.push(PositionedKey {
                    rect: Rect::new(mx, grid_top, kw, media_h),
                    resolved: ResolvedGridKey {
                        grid: grid_spec,
                        normal: spec.normal,
                        shift: spec.shift,
                        altgr: None,
                        altgr_shift: None,
                        is_alphabetic: false,
                        has_altgr: false,
                    },
                });
                mx += kw + gap;
            }
            grid_top += media_h + gap;
        }

        // Unified 2D 0.5-unit coordinate projection (keeps half-height Inverted-T arrows flush with full-height keys!)
        let grid_h = (grid_bottom - grid_top).max(40.0);
        let cols = panel_def.cols.max(1.0);
        let rows = panel_def.rows.max(1.0);
        let unit_w = (grid_w + gap) / cols;
        let unit_h = (grid_h + gap) / rows;

        for resolved in resolved_keys {
            let g = &resolved.grid;
            let rx = outer_pad + g.x * unit_w;
            let ry = grid_top + g.y * unit_h;
            let rw = (g.w * unit_w - gap).max(12.0);
            let rh = (g.h * unit_h - gap).max(12.0);
            keys.push(PositionedKey {
                rect: Rect::new(rx, ry, rw, rh),
                resolved,
            });
        }

        let grip_size = 18.0;
        let resize_grip_rect = Rect::new(
            width - grip_size - 2.0,
            height - grip_size - 2.0,
            grip_size,
            grip_size,
        );

        Self {
            width,
            height,
            header_rect,
            esc_rect,
            lang_buttons,
            panel_switcher_rect,
            panel_switcher_label,
            options_rect,
            hide_rect,
            keys,
            resize_grip_rect,
            outer_radius,
        }
    }

    /// Hit-test a surface-local logical point against resize edges, toolbar buttons, header drag, and keys.
    #[must_use]
    pub fn hit_test(&self, p: Point) -> Option<KeyboardHit> {
        if let Some(edge) = detect_resize_edge(p, self.width, self.height, 7.0, 18.0) {
            return Some(KeyboardHit::Resize(edge));
        }
        if self.esc_rect.contains(p) {
            return Some(KeyboardHit::ToolbarEsc);
        }
        for btn in &self.lang_buttons {
            if btn.rect.contains(p) {
                return Some(KeyboardHit::ToolbarLang(btn.code.clone()));
            }
        }
        if let Some(pr) = self.panel_switcher_rect
            && pr.contains(p)
        {
            return Some(KeyboardHit::ToolbarPanelCycle);
        }
        if self.options_rect.contains(p) {
            return Some(KeyboardHit::ToolbarOptions);
        }
        if self.hide_rect.contains(p) {
            return Some(KeyboardHit::ToolbarHide);
        }
        if self.header_rect.contains(p) {
            return Some(KeyboardHit::DragHeader);
        }
        for (idx, key) in self.keys.iter().enumerate() {
            if key.rect.contains(p) {
                return Some(KeyboardHit::Key(idx));
            }
        }
        None
    }

    /// Render the complete floating keyboard overlay into `pixmap` at HiDPI `scale`.
    pub fn render(
        &self,
        renderer: &mut UiRenderer,
        pixmap: &mut Pixmap,
        state: &KeyboardState,
        hovered_toolbar: Option<&KeyboardHit>,
        scale: f32,
    ) {
        let app = &state.config.appearance;
        let pal = app.resolved_palette();
        let alpha = app.opacity;
        let full_rect = Rect::new(0.0, 0.0, self.width, self.height);

        pixmap.data_mut().fill(0);

        // 1. Unified rounded window background
        UiRenderer::fill_rounded_rect(
            pixmap,
            full_rect,
            self.outer_radius,
            pal.colors.window_bg.with_alpha(alpha),
            scale,
        );

        let div_y = self.header_rect.y + self.header_rect.h + 2.5;
        UiRenderer::draw_hline(
            pixmap,
            self.header_rect.x,
            self.header_rect.x + self.header_rect.w,
            div_y,
            1.0,
            pal.colors.key_border.with_alpha(0.35 * alpha),
            scale,
        );

        // 2. Centered subtle drag grip pill inside empty header space
        let drag_pill_w = 40.0;
        let drag_pill_h = 4.0;
        let left_occupied = self
            .panel_switcher_rect
            .map(|r| r.x + r.w)
            .or_else(|| self.lang_buttons.last().map(|b| b.rect.x + b.rect.w))
            .unwrap_or(self.esc_rect.x + self.esc_rect.w);
        let right_occupied = self.options_rect.x;
        if right_occupied - left_occupied > drag_pill_w + 20.0 {
            let cx = (left_occupied + right_occupied) * 0.5;
            let cy = self.header_rect.y + self.header_rect.h * 0.5;
            UiRenderer::fill_rounded_rect(
                pixmap,
                Rect::new(
                    cx - drag_pill_w * 0.5,
                    cy - drag_pill_h * 0.5,
                    drag_pill_w,
                    drag_pill_h,
                ),
                2.0,
                pal.muted_text.with_alpha(0.45 * alpha),
                scale,
            );
        }

        // 3. Top toolbar buttons: ESC, Fast Language Switcher, Panel Switcher, Options, Hide
        let tb_radius = (app.corner_radius * 0.8).clamp(4.0, 10.0);
        let tb_font = (self.esc_rect.h * 0.44 * app.font_scale).clamp(10.0, 15.0);

        // ESC pill
        let esc_pressed = state.touches.is_key_pressed("TOOLBAR_ESC");
        let esc_hovered = matches!(hovered_toolbar, Some(KeyboardHit::ToolbarEsc));
        let esc_bg = if esc_pressed {
            pal.colors.accent
        } else if esc_hovered {
            pal.modifier_hover
        } else {
            pal.colors.modifier_bg
        };
        let esc_fg = if esc_pressed {
            pal.accent_text
        } else {
            pal.colors.text_icon
        };
        UiRenderer::fill_rounded_rect(
            pixmap,
            self.esc_rect,
            tb_radius,
            esc_bg.with_alpha(alpha),
            scale,
        );
        UiRenderer::stroke_rounded_rect(
            pixmap,
            self.esc_rect,
            tb_radius,
            app.key_border_width.min(1.5),
            pal.colors.key_border.with_alpha(0.75 * alpha),
            scale,
        );
        renderer.draw_text(
            pixmap,
            "ESC",
            self.esc_rect,
            tb_font,
            600,
            esc_fg,
            TextAlign::Center,
            scale,
        );

        // Fast Language Switcher pills (only user's visible_languages!)
        for btn in &self.lang_buttons {
            let is_hovered = matches!(
                hovered_toolbar,
                Some(KeyboardHit::ToolbarLang(code)) if code == &btn.code
            );
            let fill = if btn.active {
                pal.colors.accent
            } else if is_hovered {
                pal.modifier_hover
            } else {
                pal.colors.modifier_bg
            };
            let fg = if btn.active {
                pal.accent_text
            } else {
                pal.colors.text_icon
            };
            UiRenderer::fill_rounded_rect(
                pixmap,
                btn.rect,
                tb_radius,
                fill.with_alpha(alpha),
                scale,
            );
            if !btn.active {
                UiRenderer::stroke_rounded_rect(
                    pixmap,
                    btn.rect,
                    tb_radius,
                    app.key_border_width.min(1.2),
                    pal.colors.key_border.with_alpha(0.65 * alpha),
                    scale,
                );
            }
            renderer.draw_text(
                pixmap,
                &btn.label,
                btn.rect,
                tb_font,
                if btn.active { 700 } else { 500 },
                fg,
                TextAlign::Center,
                scale,
            );
        }

        // Panel Switcher pill
        if let Some(pr) = self.panel_switcher_rect {
            let hovered = matches!(hovered_toolbar, Some(KeyboardHit::ToolbarPanelCycle));
            let fill = if state.panel_editor_open {
                pal.colors.accent
            } else if hovered {
                pal.modifier_hover
            } else {
                pal.colors.modifier_bg
            };
            let fg = if state.panel_editor_open {
                pal.accent_text
            } else {
                pal.colors.text_icon
            };
            UiRenderer::fill_rounded_rect(pixmap, pr, tb_radius, fill.with_alpha(alpha), scale);
            UiRenderer::stroke_rounded_rect(
                pixmap,
                pr,
                tb_radius,
                app.key_border_width.min(1.2),
                pal.colors.key_border.with_alpha(0.75 * alpha),
                scale,
            );
            let icon_box = Rect::new(pr.x + 6.0, pr.y, 16.0, pr.h);
            renderer.draw_icon(pixmap, IconId::Panels, icon_box, 13.5, fg, scale);
            let lbl_box = Rect::new(pr.x + 23.0, pr.y, pr.w - 28.0, pr.h);
            renderer.draw_text(
                pixmap,
                &self.panel_switcher_label,
                lbl_box,
                (tb_font * 0.94).clamp(10.0, 14.0),
                500,
                fg,
                TextAlign::Center,
                scale,
            );
        }

        // Options button
        let opt_hovered = matches!(hovered_toolbar, Some(KeyboardHit::ToolbarOptions));
        let opt_bg = if state.settings_open {
            pal.colors.accent
        } else if opt_hovered {
            pal.modifier_hover
        } else {
            pal.colors.modifier_bg
        };
        let opt_fg = if state.settings_open {
            pal.accent_text
        } else {
            pal.colors.text_icon
        };
        UiRenderer::fill_rounded_rect(
            pixmap,
            self.options_rect,
            tb_radius,
            opt_bg.with_alpha(alpha),
            scale,
        );
        UiRenderer::stroke_rounded_rect(
            pixmap,
            self.options_rect,
            tb_radius,
            app.key_border_width.min(1.5),
            pal.colors.key_border.with_alpha(0.75 * alpha),
            scale,
        );
        if self.options_rect.w >= 70.0 {
            let icon_box = Rect::new(
                self.options_rect.x + 6.0,
                self.options_rect.y,
                18.0,
                self.options_rect.h,
            );
            renderer.draw_icon(pixmap, IconId::Settings, icon_box, 14.5, opt_fg, scale);
            let label_box = Rect::new(
                self.options_rect.x + 24.0,
                self.options_rect.y,
                self.options_rect.w - 30.0,
                self.options_rect.h,
            );
            renderer.draw_text(
                pixmap,
                "Options",
                label_box,
                tb_font,
                500,
                opt_fg,
                TextAlign::Center,
                scale,
            );
        } else {
            renderer.draw_icon(
                pixmap,
                IconId::Settings,
                self.options_rect,
                15.0,
                opt_fg,
                scale,
            );
        }

        // Hide button
        let hide_hovered = matches!(hovered_toolbar, Some(KeyboardHit::ToolbarHide));
        let hide_bg = if hide_hovered {
            pal.modifier_hover
        } else {
            pal.colors.modifier_bg
        };
        UiRenderer::fill_rounded_rect(
            pixmap,
            self.hide_rect,
            tb_radius,
            hide_bg.with_alpha(alpha),
            scale,
        );
        UiRenderer::stroke_rounded_rect(
            pixmap,
            self.hide_rect,
            tb_radius,
            app.key_border_width.min(1.5),
            pal.colors.key_border.with_alpha(0.75 * alpha),
            scale,
        );
        renderer.draw_icon(
            pixmap,
            IconId::Close,
            self.hide_rect,
            14.5,
            pal.colors.text_icon,
            scale,
        );

        // 4. Keycaps
        let shift = state.modifiers.effective_shift();
        let uppercase = state.modifiers.effective_uppercase();
        let altgr = state.modifiers.effective_altgr();
        let is_compact = state.config.layout.style == KeyboardStyle::CompactTv;
        let is_rtl = state.active_layout().is_rtl;
        let use_symbols = state.config.layout.keycap_style == KeycapStyle::Symbols;

        for pos_key in &self.keys {
            let resolved = &pos_key.resolved;
            let spec = &resolved.grid;
            let rect = pos_key.rect;

            let is_pressed = state.touches.is_key_pressed(&spec.id);
            let is_hovered = state.hovered_key_id.as_deref() == Some(spec.id.as_str());
            let (mod_active, mod_locked) = match spec.role {
                KeyRole::Modifier(kind) => (
                    state.modifiers.is_active(kind),
                    state.modifiers.is_locked(kind),
                ),
                _ => (false, false),
            };
            let altgr_highlighted =
                altgr && resolved.has_altgr && state.config.layout.altgr_amber_hints;

            let key_radius = app.corner_radius.min(rect.h * 0.38);

            if app.keycap_shadow {
                let shadow_rect = Rect::new(rect.x, rect.y + 1.5, rect.w, rect.h);
                UiRenderer::fill_rounded_rect(
                    pixmap,
                    shadow_rect,
                    key_radius,
                    pal.shadow_color,
                    scale,
                );
            }

            let fill = if is_pressed || mod_locked {
                pal.colors.accent
            } else if mod_active {
                pal.colors.accent.mix(pal.colors.modifier_bg, 0.25)
            } else if let Some(custom) = spec.color_override {
                if is_hovered {
                    custom.adjust_brightness(if pal.is_light { -0.07 } else { 0.12 })
                } else {
                    custom
                }
            } else if matches!(
                spec.role,
                KeyRole::Modifier(_) | KeyRole::Action | KeyRole::Media | KeyRole::LayerToggle
            ) {
                if is_hovered {
                    pal.modifier_hover
                } else {
                    pal.colors.modifier_bg
                }
            } else if is_hovered {
                pal.key_hover
            } else {
                pal.colors.key_bg
            };

            let fg = if is_pressed || mod_active || mod_locked {
                pal.accent_text
            } else if let Some(custom) = spec.color_override {
                custom.contrasting_text()
            } else {
                pal.colors.text_icon
            };

            UiRenderer::fill_rounded_rect(pixmap, rect, key_radius, fill.with_alpha(alpha), scale);

            let (border_w, border_col) = if altgr_highlighted {
                (app.key_border_width.max(1.8), pal.altgr_amber)
            } else if is_pressed || mod_active || mod_locked {
                (
                    app.key_border_width.max(1.2),
                    pal.colors.accent.adjust_brightness(0.18),
                )
            } else {
                (app.key_border_width, pal.colors.key_border)
            };
            if border_w > 0.05 {
                UiRenderer::stroke_rounded_rect(
                    pixmap,
                    rect,
                    key_radius,
                    border_w,
                    border_col.with_alpha(alpha),
                    scale,
                );
            }

            if mod_locked {
                UiRenderer::fill_circle(
                    pixmap,
                    rect.x + rect.w - 7.5,
                    rect.y + 7.5,
                    2.6,
                    pal.accent_text,
                    scale,
                );
            }

            // Decide whether to render a monochrome SVG icon (native icon OR Symbol-mode icon)
            let chosen_icon = if use_symbols {
                spec.symbol_icon.or(spec.icon)
            } else {
                spec.icon
            };
            if let Some(icon_id) = chosen_icon {
                let icon_size = (rect.h * 0.56 * app.font_scale).clamp(11.0, 22.0);
                renderer.draw_icon(pixmap, icon_id, rect, icon_size, fg, scale);
                continue;
            }

            let base_ratio = match spec.role {
                KeyRole::Character => {
                    let base = if is_compact { 0.44 } else { 0.38 };
                    if is_rtl && resolved.is_alphabetic {
                        base * 1.24
                    } else {
                        base
                    }
                }
                KeyRole::Space => 0.28,
                KeyRole::Modifier(_) | KeyRole::Action | KeyRole::LayerToggle | KeyRole::Media => {
                    if resolved.normal.chars().count() <= 2 {
                        0.36
                    } else {
                        0.28
                    }
                }
            };
            let font_size = (rect.h * base_ratio * app.font_scale).clamp(9.5, 32.0);
            let weight = if spec.role == KeyRole::Space {
                400
            } else {
                app.font_weight
            };

            if spec.role == KeyRole::Character {
                let (primary, secondary) = resolved.display_labels(shift, uppercase, altgr);
                let primary_rect = if secondary.is_some() && rect.h >= 34.0 {
                    Rect::new(
                        rect.x + 2.0,
                        rect.y + rect.h * 0.12,
                        rect.w - 4.0,
                        rect.h * 0.84,
                    )
                } else {
                    rect
                };
                let primary_color = if altgr_highlighted && !is_pressed {
                    pal.altgr_amber
                } else {
                    fg
                };
                renderer.draw_text(
                    pixmap,
                    primary,
                    primary_rect,
                    font_size,
                    weight,
                    primary_color,
                    TextAlign::Center,
                    scale,
                );

                if let Some(hint) = secondary
                    && rect.h >= 34.0
                    && rect.w >= 28.0
                {
                    let is_altgr_hint =
                        resolved.has_altgr && !altgr && state.config.layout.altgr_amber_hints;
                    if is_altgr_hint || !resolved.is_alphabetic {
                        let hint_size = (font_size * 0.56).clamp(8.5, 13.5);
                        let hint_color = if is_altgr_hint {
                            pal.altgr_amber
                        } else {
                            pal.muted_text
                        };
                        let hint_rect = Rect::new(
                            rect.x + rect.w * 0.45,
                            rect.y + 2.5,
                            rect.w * 0.50 - 3.0,
                            rect.h * 0.36,
                        );
                        renderer.draw_text(
                            pixmap,
                            hint,
                            hint_rect,
                            hint_size,
                            if is_altgr_hint { 600 } else { 400 },
                            hint_color,
                            TextAlign::Right,
                            scale,
                        );
                    }
                }
            } else if matches!(spec.action, GridKeyAction::Shortcut { .. })
                && !spec.shift_label.is_empty()
                && rect.h >= 36.0
            {
                // Two-line shortcut tile: e.g. "Copy" + subtle "Ctrl+C"
                renderer.draw_text(
                    pixmap,
                    &resolved.normal,
                    Rect::new(
                        rect.x + 3.0,
                        rect.y + rect.h * 0.14,
                        rect.w - 6.0,
                        rect.h * 0.46,
                    ),
                    (font_size * 1.02).clamp(10.5, 16.0),
                    600,
                    fg,
                    TextAlign::Center,
                    scale,
                );
                renderer.draw_text(
                    pixmap,
                    &spec.shift_label,
                    Rect::new(
                        rect.x + 3.0,
                        rect.y + rect.h * 0.56,
                        rect.w - 6.0,
                        rect.h * 0.34,
                    ),
                    (font_size * 0.72).clamp(8.5, 12.0),
                    400,
                    if is_pressed {
                        pal.accent_text.with_alpha(0.85)
                    } else {
                        pal.muted_text
                    },
                    TextAlign::Center,
                    scale,
                );
            } else {
                let text_color = if spec.role == KeyRole::Space && !is_pressed {
                    pal.muted_text
                } else {
                    fg
                };
                renderer.draw_text(
                    pixmap,
                    &resolved.normal,
                    rect.inset(2.0),
                    font_size,
                    weight,
                    text_color,
                    TextAlign::Center,
                    scale,
                );
            }
        }

        // 5. Bottom-right resize grip icon
        renderer.draw_icon(
            pixmap,
            IconId::ResizeGrip,
            self.resize_grip_rect,
            13.0,
            pal.muted_text.with_alpha(0.75 * alpha),
            scale,
        );

        // 6. Crisp unified outer border wrapping BOTH headerbar and keyboard seamlessly
        if app.outline_width > 0.05 {
            UiRenderer::stroke_rounded_rect(
                pixmap,
                full_rect,
                self.outer_radius,
                app.outline_width,
                pal.colors.window_outline,
                scale,
            );
        }
    }

    /// Build a synthetic `ResolvedGridKey` for the top-bar `ESC` button.
    #[must_use]
    pub fn esc_key_spec() -> ResolvedGridKey {
        ResolvedGridKey {
            grid: GridKeySpec {
                id: "TOOLBAR_ESC".to_owned(),
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 1.0,
                action: GridKeyAction::FixedKey {
                    keycode: KeyCode::ESC,
                },
                role: KeyRole::Action,
                label: "ESC".to_owned(),
                shift_label: String::new(),
                icon: None,
                symbol_icon: None,
                color_override: None,
            },
            normal: "ESC".to_owned(),
            shift: "ESC".to_owned(),
            altgr: None,
            altgr_shift: None,
            is_alphabetic: false,
            has_altgr: false,
        }
    }
}

/// Helper to check whether a key is a modifier kind.
#[must_use]
pub fn key_modifier_kind(spec: &GridKeySpec) -> Option<ModifierKind> {
    match spec.role {
        KeyRole::Modifier(kind) => Some(kind),
        _ => None,
    }
}
