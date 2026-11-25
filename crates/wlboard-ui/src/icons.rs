//! Embedded original monochrome stroke SVG icons rasterized at HiDPI via `resvg` + `tiny-skia`.

use std::collections::HashMap;

use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg::{Options, Tree};
use wlboard_core::{Color, IconId};

/// Clean-room 24x24 monochrome stroke SVG path markup for each semantic icon.
#[must_use]
pub const fn svg_template(id: IconId) -> &'static str {
    match id {
        IconId::Menu => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="COLOR" stroke-width="2.1" stroke-linecap="round"><line x1="4" y1="6.5" x2="20" y2="6.5"/><line x1="4" y1="12" x2="20" y2="12"/><line x1="4" y1="17.5" x2="20" y2="17.5"/></svg>"##
        }
        IconId::Settings => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="COLOR" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3.2"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09a1.65 1.65 0 0 0-1-1.51 1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09a1.65 1.65 0 0 0 1.51-1 1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33h.08a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82v.08a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1Z"/></svg>"##
        }
        IconId::Backspace => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="COLOR" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 5H9l-6 7 6 7h12a1 1 0 0 0 1-1V6a1 1 0 0 0-1-1Z"/><line x1="17.5" y1="9.5" x2="12.5" y2="14.5"/><line x1="12.5" y1="9.5" x2="17.5" y2="14.5"/></svg>"##
        }
        IconId::ForwardDelete => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="COLOR" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 5h12l6 7-6 7H3a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1Z"/><line x1="6.5" y1="9.5" x2="11.5" y2="14.5"/><line x1="11.5" y1="9.5" x2="6.5" y2="14.5"/></svg>"##
        }
        IconId::Tab => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="COLOR" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="4" y1="12" x2="16.5" y2="12"/><polyline points="12,7.5 16.5,12 12,16.5"/><line x1="20" y1="6.5" x2="20" y2="17.5"/></svg>"##
        }
        IconId::CapsLock => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="COLOR" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="12,4 5,12 9,12 9,16 15,16 15,12 19,12"/><line x1="8" y1="20" x2="16" y2="20"/></svg>"##
        }
        IconId::Shift => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="COLOR" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="12,4.5 4.5,13 9,13 9,19.5 15,19.5 15,13 19.5,13"/></svg>"##
        }
        IconId::Return => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="COLOR" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="9,10 5,14 9,18"/><path d="M19 6v6a2 2 0 0 1-2 2H5"/></svg>"##
        }
        IconId::SuperKey => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="COLOR" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><rect x="4" y="4" width="6.5" height="6.5" rx="1.2"/><rect x="13.5" y="4" width="6.5" height="6.5" rx="1.2"/><rect x="4" y="13.5" width="6.5" height="6.5" rx="1.2"/><rect x="13.5" y="13.5" width="6.5" height="6.5" rx="1.2"/></svg>"##
        }
        IconId::Close => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="COLOR" stroke-width="2.2" stroke-linecap="round"><line x1="6" y1="6" x2="18" y2="18"/><line x1="18" y1="6" x2="6" y2="18"/></svg>"##
        }
        IconId::ResizeGrip => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="COLOR" stroke-width="2" stroke-linecap="round"><line x1="20" y1="9" x2="9" y2="20"/><line x1="20" y1="14.5" x2="14.5" y2="20"/><line x1="20" y1="20" x2="19.5" y2="20"/></svg>"##
        }
        IconId::ArrowUp => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="COLOR" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><polyline points="6,15 12,9 18,15"/></svg>"##
        }
        IconId::ArrowDown => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="COLOR" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><polyline points="6,9 12,15 18,9"/></svg>"##
        }
        IconId::ArrowLeft => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="COLOR" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><polyline points="15,6 9,12 15,18"/></svg>"##
        }
        IconId::ArrowRight => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="COLOR" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><polyline points="9,6 15,12 9,18"/></svg>"##
        }
        IconId::BrightnessDown => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="COLOR" stroke-width="2" stroke-linecap="round"><circle cx="12" cy="12" r="3.4"/><line x1="12" y1="4.5" x2="12" y2="6"/><line x1="12" y1="18" x2="12" y2="19.5"/><line x1="4.5" y1="12" x2="6" y2="12"/><line x1="18" y1="12" x2="19.5" y2="12"/><line x1="6.7" y1="6.7" x2="7.8" y2="7.8"/><line x1="16.2" y1="16.2" x2="17.3" y2="17.3"/><line x1="17.3" y1="6.7" x2="16.2" y2="7.8"/><line x1="7.8" y1="16.2" x2="6.7" y2="17.3"/></svg>"##
        }
        IconId::BrightnessUp => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="COLOR" stroke-width="2" stroke-linecap="round"><circle cx="12" cy="12" r="4.2"/><line x1="12" y1="2.5" x2="12" y2="5"/><line x1="12" y1="19" x2="12" y2="21.5"/><line x1="2.5" y1="12" x2="5" y2="12"/><line x1="19" y1="12" x2="21.5" y2="12"/><line x1="5.3" y1="5.3" x2="7.1" y2="7.1"/><line x1="16.9" y1="16.9" x2="18.7" y2="18.7"/><line x1="18.7" y1="5.3" x2="16.9" y2="7.1"/><line x1="7.1" y1="16.9" x2="5.3" y2="18.7"/></svg>"##
        }
        IconId::MediaPrev => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="COLOR" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="5.5" y1="6" x2="5.5" y2="18"/><polygon points="18.5,6.5 9.5,12 18.5,17.5" fill="COLOR"/></svg>"##
        }
        IconId::MediaPlayPause => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="COLOR" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="4.5,6.5 12.5,12 4.5,17.5" fill="COLOR"/><line x1="15.8" y1="6.5" x2="15.8" y2="17.5"/><line x1="19.8" y1="6.5" x2="19.8" y2="17.5"/></svg>"##
        }
        IconId::MediaNext => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="COLOR" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="5.5,6.5 14.5,12 5.5,17.5" fill="COLOR"/><line x1="18.5" y1="6" x2="18.5" y2="18"/></svg>"##
        }
        IconId::AudioMute => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="COLOR" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="3.5,9.5 7.8,9.5 12.8,5.5 12.8,18.5 7.8,14.5 3.5,14.5"/><line x1="16.2" y1="9.2" x2="21" y2="14"/><line x1="21" y1="9.2" x2="16.2" y2="14"/></svg>"##
        }
        IconId::AudioLow => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="COLOR" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="4.5,9.5 8.8,9.5 13.8,5.5 13.8,18.5 8.8,14.5 4.5,14.5"/><path d="M17.2 9.2a4.3 4.3 0 0 1 0 5.6"/></svg>"##
        }
        IconId::AudioHigh => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="COLOR" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="3.5,9.5 7.5,9.5 12.2,5.5 12.2,18.5 7.5,14.5 3.5,14.5"/><path d="M15.4 9.2a4.2 4.2 0 0 1 0 5.6"/><path d="M18.4 6.5a8 8 0 0 1 0 11"/></svg>"##
        }
        IconId::Screenshot => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="COLOR" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4.5 8.5V6.5a2 2 0 0 1 2-2h2"/><path d="M15.5 4.5h2a2 2 0 0 1 2 2v2"/><path d="M19.5 15.5v2a2 2 0 0 1-2 2h-2"/><path d="M8.5 19.5h-2a2 2 0 0 1-2-2v-2"/><circle cx="12" cy="12" r="3"/></svg>"##
        }
        IconId::Panels => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="COLOR" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="18" height="16" rx="2"/><line x1="9" y1="4" x2="9" y2="20"/><line x1="9" y1="12" x2="21" y2="12"/></svg>"##
        }
        IconId::Plus => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="COLOR" stroke-width="2.2" stroke-linecap="round"><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></svg>"##
        }
        IconId::Trash => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="COLOR" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3,6 5,6 21,6"/><path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6"/><path d="M10 11v6"/><path d="M14 11v6"/><path d="M9 6V4a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2"/></svg>"##
        }
        IconId::Copy => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="COLOR" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="11" height="11" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>"##
        }
        IconId::Folder => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="COLOR" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 6.5A1.5 1.5 0 0 1 4.5 5h4.8l2 2.2h8.2A1.5 1.5 0 0 1 21 8.7v9.8a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 18.5Z"/></svg>"##
        }
    }
}

/// Cache of rasterized, theme-tinted SVG icons.
#[derive(Default)]
pub struct IconCache {
    entries: HashMap<(IconId, u32, u32), Pixmap>,
}

impl IconCache {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Rasterize (or retrieve cached) monochrome SVG tinted to `color` at `size_px x size_px`.
    pub fn get(&mut self, id: IconId, size_px: u32, color: Color) -> Option<&Pixmap> {
        let size_px = size_px.clamp(8, 128);
        let packed_color = (u32::from(color.r) << 24)
            | (u32::from(color.g) << 16)
            | (u32::from(color.b) << 8)
            | u32::from(color.a);
        let key = (id, size_px, packed_color);

        if !self.entries.contains_key(&key) {
            if self.entries.len() > 384 {
                self.entries.clear();
            }
            let hex = color.to_hex();
            let svg = svg_template(id).replace("COLOR", &hex);
            let opt = Options::default();
            let tree = Tree::from_str(&svg, &opt).ok()?;
            let mut pixmap = Pixmap::new(size_px, size_px)?;
            let scale = size_px as f32 / tree.size().width().max(1.0);
            resvg::render(
                &tree,
                Transform::from_scale(scale, scale),
                &mut pixmap.as_mut(),
            );
            self.entries.insert(key, pixmap);
        }

        self.entries.get(&key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_embedded_svgs_rasterize_cleanly_at_hidpi() {
        let mut cache = IconCache::new();
        let all = [
            IconId::Menu,
            IconId::Settings,
            IconId::Backspace,
            IconId::ForwardDelete,
            IconId::Tab,
            IconId::CapsLock,
            IconId::Shift,
            IconId::Return,
            IconId::SuperKey,
            IconId::Close,
            IconId::ResizeGrip,
            IconId::ArrowUp,
            IconId::ArrowDown,
            IconId::ArrowLeft,
            IconId::ArrowRight,
            IconId::BrightnessDown,
            IconId::BrightnessUp,
            IconId::MediaPrev,
            IconId::MediaPlayPause,
            IconId::MediaNext,
            IconId::AudioMute,
            IconId::AudioLow,
            IconId::AudioHigh,
            IconId::Screenshot,
            IconId::Panels,
            IconId::Plus,
            IconId::Trash,
            IconId::Copy,
            IconId::Folder,
        ];
        for id in all {
            let pm = cache
                .get(id, 32, Color::rgb(255, 255, 255))
                .expect("SVG icon must parse and rasterize");
            assert_eq!(pm.width(), 32);
            assert!(
                pm.data().iter().any(|&b| b > 0),
                "icon {id:?} produced empty pixmap"
            );
        }
    }
}
