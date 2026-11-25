//! Floating keyboard overlay geometry solver, hit-testing, and `tiny-skia` renderer.

use tiny_skia::Pixmap;
use wlboard_core::{
    IconId, KeyCode, KeyRole, KeySpec, KeyboardState, KeyboardStyle, ModifierKind, Point, Rect,
    ResizeEdge, detect_resize_edge, media_control_row,
};

use crate::canvas::{TextAlign, UiRenderer};

/// Logical hit target produced by pointer or touch hit-testing on the keyboard surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyboardHit {
    Resize(ResizeEdge),
    ToolbarEsc,
    ToolbarLang(String),
    ToolbarOptions,
    ToolbarHide,
    DragHeader,
    Key(usize),
}

/// Positioned keycap inside the keyboard overlay.
#[derive(Debug, Clone)]
pub struct PositionedKey {
    pub rect: Rect,
    pub spec: KeySpec,
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
    pub options_rect: Rect,
    pub hide_rect: Rect,
    pub keys: Vec<PositionedKey>,
    pub resize_grip_rect: Rect,
    pub outer_radius: f32,
}

impl KeyboardViewLayout {
    /// Compute responsive layout rects for the current keyboard state and logical dimensions.
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

        // Fast Language Switcher pills next to ESC
        let mut lang_buttons = Vec::new();
        if state.config.layout.show_fast_switcher {
            let mut lx = esc_rect.x + esc_rect.w + 8.0;
            let pill_w = (btn_h * 1.25).clamp(32.0, 44.0);
            for def in state.registry.available() {
                if lx + pill_w > header_rect.x + header_rect.w - 140.0 {
                    break;
                }
                lang_buttons.push(ToolbarButton {
                    rect: Rect::new(lx, btn_y, pill_w, btn_h),
                    label: def.short_label.clone(),
                    code: def.code.clone(),
                    active: state.config.layout.active == def.code,
                });
                lx += pill_w + 4.0;
            }
        }

        // Right: Hide icon button + Options button
        let hide_w = btn_h;
        let hide_rect = Rect::new(header_rect.x + header_rect.w - hide_w, btn_y, hide_w, btn_h);
        let options_w = if width >= 540.0 { 92.0 } else { btn_h + 6.0 };
        let options_rect = Rect::new(hide_rect.x - options_w - 6.0, btn_y, options_w, btn_h);

        // Build rows (optional Media Row + main key grid)
        let mut all_rows: Vec<(f32, Vec<KeySpec>)> = Vec::new();
        if state.config.layout.show_media_row {
            all_rows.push((0.72, media_control_row()));
        }
        for row in state.current_rows() {
            all_rows.push((1.0, row));
        }

        let grid_top = header_rect.y + header_rect.h + 6.0;
        let grid_bottom = height - outer_pad;
        let grid_h = (grid_bottom - grid_top).max(40.0);
        let grid_w = (width - outer_pad * 2.0).max(100.0);

        let total_row_weight: f32 = all_rows.iter().map(|(w, _)| *w).sum();
        let total_v_gaps = gap * (all_rows.len().saturating_sub(1) as f32);
        let unit_h = ((grid_h - total_v_gaps) / total_row_weight.max(1.0)).max(16.0);

        let mut keys = Vec::with_capacity(80);
        let mut cur_y = grid_top;

        for (row_weight, row) in all_rows {
            let row_h = unit_h * row_weight;
            let total_w: f32 = row.iter().map(|k| k.weight).sum();
            let h_gaps = gap * (row.len().saturating_sub(1) as f32);
            let avail_w = (grid_w - h_gaps).max(20.0);
            let mut cur_x = outer_pad;

            for (idx, spec) in row.into_iter().enumerate() {
                let key_w = if idx + 1 == 0 {
                    0.0
                } else {
                    avail_w * (spec.weight / total_w.max(1.0))
                };
                keys.push(PositionedKey {
                    rect: Rect::new(cur_x, cur_y, key_w, row_h),
                    spec,
                });
                cur_x += key_w + gap;
            }
            cur_y += row_h + gap;
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

        // Clear to fully transparent outside the rounded chassis corners.
        pixmap.data_mut().fill(0);

        // 1. Unified rounded window background
        let bg = pal.colors.window_bg.with_alpha(alpha);
        UiRenderer::fill_rounded_rect(pixmap, full_rect, self.outer_radius, bg, scale);

        // Subtle header divider line
        let div_y = self.header_rect.y + self.header_rect.h + 2.5;
        let div_color = pal.colors.key_border.with_alpha(0.35 * alpha);
        UiRenderer::draw_hline(
            pixmap,
            self.header_rect.x,
            self.header_rect.x + self.header_rect.w,
            div_y,
            1.0,
            div_color,
            scale,
        );

        // 2. Centered subtle drag grip pill inside empty header space
        let drag_pill_w = 42.0;
        let drag_pill_h = 4.0;
        let left_occupied = self
            .lang_buttons
            .last()
            .map_or(self.esc_rect.x + self.esc_rect.w, |b| b.rect.x + b.rect.w);
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

        // 3. Top toolbar buttons: ESC, Fast Language Switcher, Options, Hide
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

        // Fast language switcher pills (ONLY the active language glows in accent!)
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

        // Options button (monochrome Settings SVG + label)
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

        // Hide button (monochrome Close SVG)
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

        // 4. Keycaps (Media row + Main keyboard grid)
        let shift = state.modifiers.effective_shift();
        let uppercase = state.modifiers.effective_uppercase();
        let altgr = state.modifiers.effective_altgr();
        let is_compact = state.config.layout.style == KeyboardStyle::CompactTv;

        for pos_key in &self.keys {
            let spec = &pos_key.spec;
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
                altgr && spec.has_altgr && state.config.layout.altgr_amber_hints;

            // Optional keycap depth shadow
            if app.keycap_shadow {
                let shadow_rect = Rect::new(rect.x, rect.y + 1.6, rect.w, rect.h);
                UiRenderer::fill_rounded_rect(
                    pixmap,
                    shadow_rect,
                    app.corner_radius,
                    pal.shadow_color,
                    scale,
                );
            }

            // Keycap fill color
            let fill = if is_pressed || mod_locked {
                pal.colors.accent
            } else if mod_active {
                pal.colors.accent.mix(pal.colors.modifier_bg, 0.25)
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
            } else {
                pal.colors.text_icon
            };

            UiRenderer::fill_rounded_rect(
                pixmap,
                rect,
                app.corner_radius,
                fill.with_alpha(alpha),
                scale,
            );

            // Keycap border (amber glow when AltGr is active on an AltGr-capable key!)
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
                    app.corner_radius,
                    border_w,
                    border_col.with_alpha(alpha),
                    scale,
                );
            }

            // Sticky Lock indicator dot in top-right corner of locked modifiers
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

            // Icon-only or primary + corner-hint label rendering
            if let Some(icon_id) = spec.icon {
                let icon_size = (rect.h * 0.48 * app.font_scale).clamp(12.0, 24.0);
                renderer.draw_icon(pixmap, icon_id, rect, icon_size, fg, scale);
                continue;
            }

            let is_rtl = state.active_layout().is_rtl;
            let base_ratio = match spec.role {
                KeyRole::Character => {
                    let base = if is_compact { 0.44 } else { 0.38 };
                    if is_rtl && spec.is_alphabetic {
                        base * 1.24
                    } else {
                        base
                    }
                }
                KeyRole::Space => 0.28,
                KeyRole::Modifier(_) | KeyRole::Action | KeyRole::LayerToggle | KeyRole::Media => {
                    if spec.normal.chars().count() <= 2 {
                        0.36
                    } else {
                        0.29
                    }
                }
            };
            let font_size = (rect.h * base_ratio * app.font_scale).clamp(10.0, 32.0);
            let weight = if spec.role == KeyRole::Space {
                400
            } else {
                app.font_weight
            };

            if spec.role == KeyRole::Character {
                let (primary, secondary) = spec.display_labels(shift, uppercase, altgr);

                // Draw primary glyph cleanly centered (slightly nudged down when corner hint exists)
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

                // Secondary corner hint (amber for AltGr diacritics when enabled, muted otherwise)
                if let Some(hint) = secondary
                    && rect.h >= 34.0
                    && rect.w >= 28.0
                {
                    let is_altgr_hint =
                        spec.has_altgr && !altgr && state.config.layout.altgr_amber_hints;
                    if is_altgr_hint || !spec.is_alphabetic {
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
            } else {
                let text_color = if spec.role == KeyRole::Space && !is_pressed {
                    pal.muted_text
                } else {
                    fg
                };
                renderer.draw_text(
                    pixmap,
                    &spec.normal,
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

    /// Build a synthetic `KeySpec` for the top-bar `ESC` button.
    #[must_use]
    pub fn esc_key_spec() -> KeySpec {
        KeySpec {
            id: "TOOLBAR_ESC".to_owned(),
            action: wlboard_core::KeyAction::Key(KeyCode::ESC),
            role: KeyRole::Action,
            weight: 1.0,
            normal: "ESC".to_owned(),
            shift: "ESC".to_owned(),
            altgr: None,
            altgr_shift: None,
            icon: None,
            is_alphabetic: false,
            has_altgr: false,
        }
    }
}

/// Helper to check whether a key is a modifier kind for status checks.
#[must_use]
pub fn key_modifier_kind(spec: &KeySpec) -> Option<ModifierKind> {
    match spec.role {
        KeyRole::Modifier(kind) => Some(kind),
        _ => None,
    }
}
