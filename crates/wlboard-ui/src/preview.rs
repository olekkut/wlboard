//! Headless offline PNG preview renderer used for automated visual QA and README screenshots.

use std::fs;
use std::path::Path;

use tiny_skia::Pixmap;
use wlboard_core::{
    AppConfig, AppearanceSettings, Color, ColorSlot, KeyboardState, KeyboardStyle, ModifierKind,
    PresetStore, ThemeBase,
};

use crate::canvas::UiRenderer;
use crate::keyboard_view::KeyboardViewLayout;
use crate::settings_view::{
    SETTINGS_DEFAULT_HEIGHT, SETTINGS_DEFAULT_WIDTH, SettingsTab, SettingsView,
};

/// Render the floating keyboard overlay into a premultiplied RGBA `Pixmap` at HiDPI `scale`.
#[must_use]
pub fn render_keyboard_pixmap(
    renderer: &mut UiRenderer,
    state: &KeyboardState,
    width: u32,
    height: u32,
    scale: f32,
) -> Pixmap {
    let pw = ((width as f32) * scale).round().max(1.0) as u32;
    let ph = ((height as f32) * scale).round().max(1.0) as u32;
    let mut pixmap = Pixmap::new(pw, ph).expect("valid pixmap dimensions");
    let layout = KeyboardViewLayout::compute(state, width as f32, height as f32);
    layout.render(renderer, &mut pixmap, state, None, scale);
    pixmap
}

/// Render the built-in `Customize wlboard` Settings window into a `Pixmap` at HiDPI `scale`.
#[must_use]
pub fn render_settings_pixmap(
    renderer: &mut UiRenderer,
    settings: &mut SettingsView,
    state: &KeyboardState,
    scale: f32,
) -> Pixmap {
    let pw = (settings.width * scale).round().max(1.0) as u32;
    let ph = (settings.height * scale).round().max(1.0) as u32;
    let mut pixmap = Pixmap::new(pw, ph).expect("valid settings pixmap dimensions");
    settings.render(renderer, &mut pixmap, state, scale);
    pixmap
}

/// Composite a keyboard preview onto a subtle desktop backdrop so white borders and transparency shine clearly.
#[must_use]
fn composite_on_backdrop(fg: &Pixmap, backdrop: Color, pad: u32) -> Pixmap {
    let w = fg.width() + pad * 2;
    let h = fg.height() + pad * 2;
    let mut out = Pixmap::new(w, h).expect("backdrop pixmap");
    out.fill(crate::canvas::to_skia_color(backdrop));
    out.draw_pixmap(
        pad as i32,
        pad as i32,
        fg.as_ref(),
        &tiny_skia::PixmapPaint::default(),
        tiny_skia::Transform::identity(),
        None,
    );
    out
}

/// Generate the full showcase suite of PNG screenshots for visual inspection and documentation.
pub fn generate_preview_suite(out_dir: &Path) -> Result<Vec<String>, String> {
    fs::create_dir_all(out_dir).map_err(|e| format!("create_dir_all failed: {e}"))?;
    let mut renderer = UiRenderer::new();
    let temp_presets = out_dir.join(".preview_presets_tmp");
    let _ = fs::remove_dir_all(&temp_presets);
    let store = PresetStore::with_dir(&temp_presets);

    // Pre-populate two showcase user presets so the Settings screenshot displays real saved presets.
    let mut midnight = AppearanceSettings::default();
    midnight.colors.window_bg = Color::rgb(0x0E, 0x15, 0x26);
    midnight.colors.key_bg = Color::rgb(0x1B, 0x27, 0x42);
    midnight.colors.modifier_bg = Color::rgb(0x14, 0x1D, 0x33);
    midnight.colors.text_icon = Color::rgb(0xE8, 0xF4, 0xFF);
    midnight.colors.key_border = Color::rgb(0x33, 0x4E, 0x7A);
    midnight.colors.window_outline = Color::rgb(0x38, 0xBD, 0xF8);
    midnight.colors.accent = Color::rgb(0x02, 0x84, 0xC7);
    midnight.outline_width = 2.5;
    let _ = store.save("Midnight Cyan", &midnight);

    let mut emerald = AppearanceSettings::from_base(ThemeBase::Slate);
    emerald.colors.accent = Color::rgb(0x10, 0xB9, 0x81);
    let _ = store.save("Emerald Night", &emerald);

    let mut written = Vec::new();
    let dark_backdrop = Color::rgb(0x0D, 0x10, 0x16);
    let light_backdrop = Color::rgb(0xC9, 0xD2, 0xDF);

    // 1. Graphite Dark — Standard EN + Media Row
    {
        let mut cfg = AppConfig::default();
        cfg.layout.active = "en".to_owned();
        cfg.layout.style = KeyboardStyle::Standard;
        cfg.layout.show_media_row = true;
        let state = KeyboardState::new(cfg, &store);
        let pm = render_keyboard_pixmap(&mut renderer, &state, 920, 360, 2.0);
        let framed = composite_on_backdrop(&pm, dark_backdrop, 32);
        let path = out_dir.join("graphite_dark_standard_media.png");
        framed
            .save_png(&path)
            .map_err(|e| format!("save_png failed: {e}"))?;
        written.push(path.display().to_string());
    }

    // 2. Graphite Dark — Compact for TV (Ukrainian UA)
    {
        let mut cfg = AppConfig::default();
        cfg.layout.active = "uk".to_owned();
        cfg.layout.style = KeyboardStyle::CompactTv;
        cfg.layout.show_media_row = false;
        let state = KeyboardState::new(cfg, &store);
        let pm = render_keyboard_pixmap(&mut renderer, &state, 900, 310, 2.0);
        let framed = composite_on_backdrop(&pm, dark_backdrop, 32);
        let path = out_dir.join("graphite_dark_compact_tv_ua.png");
        framed
            .save_png(&path)
            .map_err(|e| format!("save_png failed: {e}"))?;
        written.push(path.display().to_string());
    }

    // 3. Frost Light — Standard Polish (PL) with AltGr active + amber diacritic borders
    {
        let mut cfg = AppConfig::default();
        cfg.layout.active = "pl".to_owned();
        cfg.layout.style = KeyboardStyle::Standard;
        cfg.layout.altgr_amber_hints = true;
        cfg.appearance.apply_base(ThemeBase::FrostLight);
        let mut state = KeyboardState::new(cfg, &store);
        let now = std::time::Instant::now();
        state.modifiers.begin_hold(ModifierKind::AltGr);
        state
            .modifiers
            .end_hold(ModifierKind::AltGr, false, true, now);
        let pm = render_keyboard_pixmap(&mut renderer, &state, 920, 320, 2.0);
        let framed = composite_on_backdrop(&pm, light_backdrop, 32);
        let path = out_dir.join("frost_light_pl_altgr.png");
        framed
            .save_png(&path)
            .map_err(|e| format!("save_png failed: {e}"))?;
        written.push(path.display().to_string());
    }

    // 4. Custom User Preset ("Midnight Cyan") — Arabic (AR) Standard
    {
        let mut cfg = AppConfig::default();
        cfg.layout.active = "ar".to_owned();
        cfg.layout.style = KeyboardStyle::Standard;
        cfg.appearance = midnight.clone();
        cfg.active_preset = Some("Midnight Cyan".to_owned());
        let state = KeyboardState::new(cfg, &store);
        let pm = render_keyboard_pixmap(&mut renderer, &state, 920, 320, 2.0);
        let framed = composite_on_backdrop(&pm, Color::rgb(0x06, 0x0A, 0x14), 32);
        let path = out_dir.join("custom_preset_midnight_cyan_ar.png");
        framed
            .save_png(&path)
            .map_err(|e| format!("save_png failed: {e}"))?;
        written.push(path.display().to_string());
    }

    // 5. Built-in Customize wlboard Settings & Presets window (Graphite Dark, Tab 1)
    {
        let cfg = AppConfig {
            active_preset: Some("Midnight Cyan".to_owned()),
            ..Default::default()
        };
        let mut state = KeyboardState::new(cfg, &store);
        state.input_backend_status =
            "Wayland Virtual Keyboard (zwp_virtual_keyboard_v1)".to_owned();
        state.layout_sync_status = "KDE Plasma D-Bus Connected (4 layouts)".to_owned();
        let mut settings = SettingsView::new(&state.config.appearance);
        settings.width = SETTINGS_DEFAULT_WIDTH as f32;
        settings.height = SETTINGS_DEFAULT_HEIGHT as f32;
        settings.tab = SettingsTab::Appearance;
        settings.selected_slot = ColorSlot::WindowOutline;
        settings.sync_from_appearance(&state.config.appearance);
        settings.preset_name_input = "Studio Graphite".to_owned();
        let pm = render_settings_pixmap(&mut renderer, &mut settings, &state, 2.0);
        let path = out_dir.join("customize_settings_window.png");
        pm.save_png(&path)
            .map_err(|e| format!("save_png failed: {e}"))?;
        written.push(path.display().to_string());
    }

    let _ = fs::remove_dir_all(&temp_presets);
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_non_empty_keyboard_and_settings_pixmaps() {
        let mut renderer = UiRenderer::new();
        let store = PresetStore::with_dir(std::env::temp_dir().join("wlboard_preview_test_empty"));
        let state = KeyboardState::new(AppConfig::default(), &store);

        let kb = render_keyboard_pixmap(&mut renderer, &state, 860, 300, 1.0);
        assert_eq!(kb.width(), 860);
        assert_eq!(kb.height(), 300);
        assert!(kb.data().contains(&255));

        let mut settings = SettingsView::new(&state.config.appearance);
        let sp = render_settings_pixmap(&mut renderer, &mut settings, &state, 1.0);
        assert_eq!(sp.width(), SETTINGS_DEFAULT_WIDTH);
        assert_eq!(sp.height(), SETTINGS_DEFAULT_HEIGHT);
        assert!(sp.data().iter().any(|&b| b > 0));
    }
}
