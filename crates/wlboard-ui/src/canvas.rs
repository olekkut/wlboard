//! Shared `tiny-skia` + `cosmic-text` + `resvg` rendering engine used by both the
//! floating keyboard overlay and the built-in Settings & Presets window.

use cosmic_text::{
    Align, Attrs, Buffer, Color as TextColor, Family, FontSystem, Metrics, Shaping, SwashCache,
    Weight, Wrap,
};
use tiny_skia::{
    Color as SkiaColor, FillRule, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap, PixmapPaint,
    Stroke, Transform,
};
use wlboard_core::{Color, IconId, Rect};

use crate::icons::IconCache;

/// Text horizontal alignment inside a target rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

/// Shared CPU software renderer with font shaping (BiDi Arabic, Cyrillic, Polish, Latin) and SVG cache.
pub struct UiRenderer {
    pub font_system: FontSystem,
    pub swash_cache: SwashCache,
    pub icons: IconCache,
    ui_font_family: Option<String>,
    available_weights: Vec<u16>,
}

impl Default for UiRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl UiRenderer {
    #[must_use]
    pub fn new() -> Self {
        let mut font_system = FontSystem::new();
        let db = font_system.db_mut();

        // Remove color-emoji and low-res bitmap fonts so ASCII digits, punctuation, and symbols
        // never get hijacked by bitmap emoji glyphs.
        let remove_ids: Vec<_> = db
            .faces()
            .filter(|face| {
                face.families.iter().any(|(name, _)| {
                    let lower = name.to_lowercase();
                    lower.contains("emoji") || lower.starts_with("unifont")
                })
            })
            .map(|face| face.id)
            .collect();
        for id in remove_ids {
            db.remove_face(id);
        }

        // Prefer crisp sans-serif UI families with broad Latin, Polish, Cyrillic, and Arabic coverage.
        const PREFERRED: &[&str] = &[
            "DejaVu Sans",
            "Noto Sans",
            "Inter",
            "Ubuntu",
            "Cantarell",
            "Liberation Sans",
            "FreeSans",
        ];

        let mut ui_font_family = None;
        for candidate in PREFERRED {
            let exists = db.faces().any(|face| {
                face.families
                    .iter()
                    .any(|(name, _)| name.eq_ignore_ascii_case(candidate))
            });
            if exists {
                ui_font_family = Some((*candidate).to_owned());
                break;
            }
        }

        let mut available_weights = Vec::new();
        if let Some(ref fam) = ui_font_family {
            db.set_sans_serif_family(fam);
            for face in db.faces() {
                if face
                    .families
                    .iter()
                    .any(|(name, _)| name.eq_ignore_ascii_case(fam))
                    && !available_weights.contains(&face.weight.0)
                {
                    available_weights.push(face.weight.0);
                }
            }
            available_weights.sort_unstable();
        }

        Self {
            font_system,
            swash_cache: SwashCache::new(),
            icons: IconCache::new(),
            ui_font_family,
            available_weights,
        }
    }

    /// Map a requested CSS weight (400..700) to the closest weight actually present in the UI family
    /// so cosmic-text never falls back to a mismatched CJK font just to match weight 500.
    fn resolve_weight(&self, requested: u16) -> Weight {
        if self.available_weights.is_empty() {
            return Weight(requested);
        }
        let best = self
            .available_weights
            .iter()
            .copied()
            .min_by_key(|&w| {
                let dist = i32::from(w).abs_diff(i32::from(requested));
                // Break ties (e.g., 500 between 400 and 600) cleanly toward lighter for <=500, bolder for >=600.
                let tie = if requested >= 600 {
                    u32::from(w < requested)
                } else {
                    u32::from(w > requested)
                };
                (dist, tie)
            })
            .unwrap_or(requested);
        Weight(best)
    }

    /// Build a smooth rounded-rectangle `tiny-skia` path.
    #[must_use]
    pub fn rounded_rect_path(rect: Rect, radius: f32) -> Option<Path> {
        if rect.w <= 0.0 || rect.h <= 0.0 {
            return None;
        }
        let r = radius.clamp(0.0, rect.w.min(rect.h) * 0.5);
        let x0 = rect.x;
        let y0 = rect.y;
        let x1 = rect.x + rect.w;
        let y1 = rect.y + rect.h;

        let mut pb = PathBuilder::new();
        if r <= 0.01 {
            pb.move_to(x0, y0);
            pb.line_to(x1, y0);
            pb.line_to(x1, y1);
            pb.line_to(x0, y1);
            pb.close();
            return pb.finish();
        }

        // Standard cubic Bezier quarter-circle control factor (4/3 * (sqrt(2) - 1)).
        const K: f32 = 0.552_284_8;
        let c = r * K;

        pb.move_to(x0 + r, y0);
        pb.line_to(x1 - r, y0);
        pb.cubic_to(x1 - r + c, y0, x1, y0 + r - c, x1, y0 + r);
        pb.line_to(x1, y1 - r);
        pb.cubic_to(x1, y1 - r + c, x1 - r + c, y1, x1 - r, y1);
        pb.line_to(x0 + r, y1);
        pb.cubic_to(x0 + r - c, y1, x0, y1 - r + c, x0, y1 - r);
        pb.line_to(x0, y0 + r);
        pb.cubic_to(x0, y0 + r - c, x0 + r - c, y0, x0 + r, y0);
        pb.close();
        pb.finish()
    }

    /// Fill an anti-aliased rounded rectangle at HiDPI `scale`.
    pub fn fill_rounded_rect(
        pixmap: &mut Pixmap,
        rect: Rect,
        radius: f32,
        color: Color,
        scale: f32,
    ) {
        if color.a == 0 {
            return;
        }
        let Some(path) = Self::rounded_rect_path(rect, radius) else {
            return;
        };
        let mut paint = Paint {
            anti_alias: true,
            ..Paint::default()
        };
        paint.set_color(to_skia_color(color));
        pixmap.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::from_scale(scale, scale),
            None,
        );
    }

    /// Stroke an anti-aliased rounded rectangle border cleanly inside `rect`.
    pub fn stroke_rounded_rect(
        pixmap: &mut Pixmap,
        rect: Rect,
        radius: f32,
        width: f32,
        color: Color,
        scale: f32,
    ) {
        if width <= 0.05 || color.a == 0 {
            return;
        }
        let inset = rect.inset(width * 0.5);
        let inner_r = (radius - width * 0.5).max(0.0);
        let Some(path) = Self::rounded_rect_path(inset, inner_r) else {
            return;
        };
        let mut paint = Paint {
            anti_alias: true,
            ..Paint::default()
        };
        paint.set_color(to_skia_color(color));
        let stroke = Stroke {
            width,
            line_cap: LineCap::Round,
            line_join: LineJoin::Round,
            ..Stroke::default()
        };
        pixmap.stroke_path(
            &path,
            &paint,
            &stroke,
            Transform::from_scale(scale, scale),
            None,
        );
    }

    /// Draw a solid circle (used for modifier lock dots, toggle knobs, and color picker handles).
    pub fn fill_circle(
        pixmap: &mut Pixmap,
        cx: f32,
        cy: f32,
        radius: f32,
        color: Color,
        scale: f32,
    ) {
        let Some(path) = PathBuilder::from_circle(cx, cy, radius) else {
            return;
        };
        let mut paint = Paint {
            anti_alias: true,
            ..Paint::default()
        };
        paint.set_color(to_skia_color(color));
        pixmap.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::from_scale(scale, scale),
            None,
        );
    }

    /// Draw a subtle horizontal line divider.
    pub fn draw_hline(
        pixmap: &mut Pixmap,
        x0: f32,
        x1: f32,
        y: f32,
        width: f32,
        color: Color,
        scale: f32,
    ) {
        let mut pb = PathBuilder::new();
        pb.move_to(x0, y);
        pb.line_to(x1, y);
        let Some(path) = pb.finish() else {
            return;
        };
        let mut paint = Paint {
            anti_alias: true,
            ..Paint::default()
        };
        paint.set_color(to_skia_color(color));
        let stroke = Stroke {
            width,
            ..Stroke::default()
        };
        pixmap.stroke_path(
            &path,
            &paint,
            &stroke,
            Transform::from_scale(scale, scale),
            None,
        );
    }

    /// Composite a cached, theme-tinted SVG icon centered inside `rect`.
    pub fn draw_icon(
        &mut self,
        pixmap: &mut Pixmap,
        id: IconId,
        rect: Rect,
        icon_logical_size: f32,
        color: Color,
        scale: f32,
    ) {
        let size_px = (icon_logical_size * scale).round().max(8.0) as u32;
        let Some(icon_pm) = self.icons.get(id, size_px, color) else {
            return;
        };
        let center = rect.center();
        let x = (center.x * scale - size_px as f32 * 0.5).round() as i32;
        let y = (center.y * scale - size_px as f32 * 0.5).round() as i32;
        let paint = PixmapPaint {
            opacity: f32::from(color.a) / 255.0,
            ..PixmapPaint::default()
        };
        pixmap.draw_pixmap(x, y, icon_pm.as_ref(), &paint, Transform::identity(), None);
    }

    /// Shape and draw crisp anti-aliased text (supports Latin, Polish, Cyrillic, Arabic BiDi/shaping)
    /// aligned inside `rect` in logical coordinates.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_text(
        &mut self,
        pixmap: &mut Pixmap,
        text: &str,
        rect: Rect,
        font_size_logical: f32,
        weight: u16,
        color: Color,
        align: TextAlign,
        scale: f32,
    ) {
        if text.is_empty() || rect.w <= 1.0 || rect.h <= 1.0 {
            return;
        }

        let font_px = (font_size_logical * scale).clamp(7.0, 128.0);
        let line_height_px = (font_px * 1.24).ceil();
        let box_w_px = (rect.w * scale).max(1.0);
        let box_h_px = (rect.h * scale).max(line_height_px);

        let mut buffer = Buffer::new(&mut self.font_system, Metrics::new(font_px, line_height_px));
        buffer.set_wrap(Wrap::None);
        buffer.set_size(Some(box_w_px), Some(box_h_px));

        let cosmic_align = match align {
            TextAlign::Left => Align::Left,
            TextAlign::Center => Align::Center,
            TextAlign::Right => Align::Right,
        };
        let resolved_weight = self.resolve_weight(weight);
        let family = self
            .ui_font_family
            .as_deref()
            .map_or(Family::SansSerif, Family::Name);
        let attrs = Attrs::new().family(family).weight(resolved_weight);
        buffer.set_text(text, &attrs, Shaping::Advanced, Some(cosmic_align));
        buffer.shape_until_scroll(&mut self.font_system, false);

        // Compute vertical centering offset based on total laid-out lines.
        let line_count = buffer.layout_runs().count().max(1) as f32;
        let text_total_h = line_count * line_height_px;
        let origin_x = (rect.x * scale).round() as i32;
        let origin_y = ((rect.y * scale) + ((rect.h * scale - text_total_h) * 0.5)).round() as i32;

        let clip_x0 = (rect.x * scale).floor() as i32;
        let clip_y0 = (rect.y * scale).floor() as i32;
        let clip_x1 = ((rect.x + rect.w) * scale).ceil() as i32;
        let clip_y1 = ((rect.y + rect.h) * scale).ceil() as i32;

        let pm_w = pixmap.width() as i32;
        let pm_h = pixmap.height() as i32;
        let data = pixmap.data_mut();
        let text_color = TextColor::rgba(color.r, color.g, color.b, color.a);

        buffer.draw(
            &mut self.font_system,
            &mut self.swash_cache,
            text_color,
            |gx, gy, w, h, glyph_color| {
                let a = glyph_color.a();
                if a == 0 {
                    return;
                }
                let px0 = origin_x + gx;
                let py0 = origin_y + gy;
                let alpha = f32::from(a) / 255.0;
                let sr = f32::from(glyph_color.r()) * alpha;
                let sg = f32::from(glyph_color.g()) * alpha;
                let sb = f32::from(glyph_color.b()) * alpha;
                let inv_a = 1.0 - alpha;

                for dy in 0..h as i32 {
                    let y = py0 + dy;
                    if y < 0 || y >= pm_h || y < clip_y0 || y >= clip_y1 {
                        continue;
                    }
                    for dx in 0..w as i32 {
                        let x = px0 + dx;
                        if x < 0 || x >= pm_w || x < clip_x0 || x >= clip_x1 {
                            continue;
                        }
                        let idx = ((y * pm_w + x) * 4) as usize;
                        // tiny-skia stores premultiplied RGBA bytes.
                        let dr = f32::from(data[idx]);
                        let dg = f32::from(data[idx + 1]);
                        let db = f32::from(data[idx + 2]);
                        let da = f32::from(data[idx + 3]);

                        data[idx] = (sr + dr * inv_a).clamp(0.0, 255.0) as u8;
                        data[idx + 1] = (sg + dg * inv_a).clamp(0.0, 255.0) as u8;
                        data[idx + 2] = (sb + db * inv_a).clamp(0.0, 255.0) as u8;
                        data[idx + 3] = (alpha * 255.0 + da * inv_a).clamp(0.0, 255.0) as u8;
                    }
                }
            },
        );
    }
}

/// Convert `wlboard_core::Color` into `tiny_skia::Color`.
#[must_use]
pub fn to_skia_color(c: Color) -> SkiaColor {
    SkiaColor::from_rgba8(c.r, c.g, c.b, c.a)
}

/// Convert a premultiplied RGBA `tiny_skia::Pixmap` into Wayland `wl_shm` `Argb8888` (little-endian BGRA).
pub fn copy_pixmap_to_wayland_argb8888(pixmap: &Pixmap, dst: &mut [u8]) {
    let (src_chunks, _) = pixmap.data().as_chunks::<4>();
    let (dst_chunks, _) = dst.as_chunks_mut::<4>();
    for (s, d) in src_chunks.iter().zip(dst_chunks.iter_mut()) {
        d[0] = s[2]; // B
        d[1] = s[1]; // G
        d[2] = s[0]; // R
        d[3] = s[3]; // A
    }
}

/// Zero-fill a `dst_w x dst_h` BGRA buffer (fully transparent) and blit `pixmap` at `(offset_x, offset_y)`.
pub fn copy_pixmap_offset_to_argb8888(
    pixmap: &Pixmap,
    dst: &mut [u8],
    dst_w: usize,
    dst_h: usize,
    offset_x: usize,
    offset_y: usize,
) {
    dst.fill(0);
    let src_w = pixmap.width() as usize;
    let src_h = pixmap.height() as usize;
    let src = pixmap.data();
    let copy_w = src_w.min(dst_w.saturating_sub(offset_x));
    let copy_h = src_h.min(dst_h.saturating_sub(offset_y));
    for y in 0..copy_h {
        let src_row_start = y * src_w * 4;
        let dst_row_start = ((offset_y + y) * dst_w + offset_x) * 4;
        let src_row = &src[src_row_start..src_row_start + copy_w * 4];
        let dst_row = &mut dst[dst_row_start..dst_row_start + copy_w * 4];
        let (s_chunks, _) = src_row.as_chunks::<4>();
        let (d_chunks, _) = dst_row.as_chunks_mut::<4>();
        for (s, d) in s_chunks.iter().zip(d_chunks.iter_mut()) {
            d[0] = s[2]; // B
            d[1] = s[1]; // G
            d[2] = s[0]; // R
            d[3] = s[3]; // A
        }
    }
}
