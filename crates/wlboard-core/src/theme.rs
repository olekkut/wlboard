//! Theme system, 7-slot customizable color palette, HSV/Hex conversions, and WCAG contrast helpers.

use serde::{Deserialize, Serialize};

/// 8-bit RGBA color value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    #[must_use]
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    #[must_use]
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    #[must_use]
    pub fn with_alpha(self, alpha_fraction: f32) -> Self {
        let a = (alpha_fraction.clamp(0.0, 1.0) * 255.0).round() as u8;
        Self { a, ..self }
    }

    /// Parse `#RRGGBB` or `RRGGBB` hex string.
    #[must_use]
    pub fn from_hex(input: &str) -> Option<Self> {
        let s = input.trim().trim_start_matches('#');
        if s.len() == 6 {
            let r = u8::from_str_radix(&s[0..2], 16).ok()?;
            let g = u8::from_str_radix(&s[2..4], 16).ok()?;
            let b = u8::from_str_radix(&s[4..6], 16).ok()?;
            Some(Self::rgb(r, g, b))
        } else if s.len() == 8 {
            let r = u8::from_str_radix(&s[0..2], 16).ok()?;
            let g = u8::from_str_radix(&s[2..4], 16).ok()?;
            let b = u8::from_str_radix(&s[4..6], 16).ok()?;
            let a = u8::from_str_radix(&s[6..8], 16).ok()?;
            Some(Self::rgba(r, g, b, a))
        } else {
            None
        }
    }

    /// Format as uppercase `#RRGGBB`.
    #[must_use]
    pub fn to_hex(self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }

    /// Blend `self` toward `other` by `t` in `0.0..=1.0`.
    #[must_use]
    pub fn mix(self, other: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let inv = 1.0 - t;
        Self {
            r: (f32::from(self.r) * inv + f32::from(other.r) * t).round() as u8,
            g: (f32::from(self.g) * inv + f32::from(other.g) * t).round() as u8,
            b: (f32::from(self.b) * inv + f32::from(other.b) * t).round() as u8,
            a: (f32::from(self.a) * inv + f32::from(other.a) * t).round() as u8,
        }
    }

    /// Lighten (`amount > 0`) or darken (`amount < 0`) relative to white/black.
    #[must_use]
    pub fn adjust_brightness(self, amount: f32) -> Self {
        if amount >= 0.0 {
            self.mix(Self::rgb(255, 255, 255), amount)
        } else {
            self.mix(Self::rgb(0, 0, 0), -amount)
        }
    }

    /// Convert RGB to HSV: `h` in `0..360`, `s` in `0..=1`, `v` in `0..=1`.
    #[must_use]
    pub fn to_hsv(self) -> (f32, f32, f32) {
        let r = f32::from(self.r) / 255.0;
        let g = f32::from(self.g) / 255.0;
        let b = f32::from(self.b) / 255.0;
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let delta = max - min;

        let h = if delta <= f32::EPSILON {
            0.0
        } else if (max - r).abs() <= f32::EPSILON {
            60.0 * (((g - b) / delta).rem_euclid(6.0))
        } else if (max - g).abs() <= f32::EPSILON {
            60.0 * (((b - r) / delta) + 2.0)
        } else {
            60.0 * (((r - g) / delta) + 4.0)
        };
        let s = if max <= f32::EPSILON {
            0.0
        } else {
            delta / max
        };
        (h.rem_euclid(360.0), s.clamp(0.0, 1.0), max.clamp(0.0, 1.0))
    }

    /// Construct color from HSV (`h` in `0..360`, `s` in `0..=1`, `v` in `0..=1`).
    #[must_use]
    pub fn from_hsv(h: f32, s: f32, v: f32) -> Self {
        let h = h.rem_euclid(360.0);
        let s = s.clamp(0.0, 1.0);
        let v = v.clamp(0.0, 1.0);
        let c = v * s;
        let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
        let m = v - c;
        let (r1, g1, b1) = match h {
            h if h < 60.0 => (c, x, 0.0),
            h if h < 120.0 => (x, c, 0.0),
            h if h < 180.0 => (0.0, c, x),
            h if h < 240.0 => (0.0, x, c),
            h if h < 300.0 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        Self::rgb(
            ((r1 + m) * 255.0).round() as u8,
            ((g1 + m) * 255.0).round() as u8,
            ((b1 + m) * 255.0).round() as u8,
        )
    }

    /// WCAG 2.1 relative luminance (`0.0` black .. `1.0` white).
    #[must_use]
    pub fn relative_luminance(self) -> f32 {
        fn channel(v: u8) -> f32 {
            let s = f32::from(v) / 255.0;
            if s <= 0.03928 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        }
        0.2126 * channel(self.r) + 0.7152 * channel(self.g) + 0.0722 * channel(self.b)
    }

    /// WCAG 2.1 contrast ratio (`1.0` .. `21.0`).
    #[must_use]
    pub fn contrast_ratio(self, other: Self) -> f32 {
        let l1 = self.relative_luminance();
        let l2 = other.relative_luminance();
        let (bright, dark) = if l1 >= l2 { (l1, l2) } else { (l2, l1) };
        (bright + 0.05) / (dark + 0.05)
    }

    /// Choose crisp dark or white foreground guaranteed to contrast cleanly on `self`.
    #[must_use]
    pub fn contrasting_text(self) -> Self {
        let white = Self::rgb(255, 255, 255);
        let dark = Self::rgb(16, 18, 24);
        if self.contrast_ratio(white) >= self.contrast_ratio(dark) {
            white
        } else {
            dark
        }
    }
}

/// Built-in theme bases with completely neutral naming.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeBase {
    #[default]
    GraphiteDark,
    FrostLight,
    HighContrastTv,
    Slate,
}

impl ThemeBase {
    pub const ALL: &'static [Self] = &[
        Self::GraphiteDark,
        Self::FrostLight,
        Self::HighContrastTv,
        Self::Slate,
    ];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::GraphiteDark => "Graphite Dark",
            Self::FrostLight => "Frost Light",
            Self::HighContrastTv => "High Contrast TV",
            Self::Slate => "Slate",
        }
    }

    #[must_use]
    pub const fn default_colors(self) -> ThemeColors {
        match self {
            Self::GraphiteDark => ThemeColors {
                window_bg: Color::rgb(0x1B, 0x1D, 0x22),
                key_bg: Color::rgb(0x34, 0x38, 0x42),
                modifier_bg: Color::rgb(0x27, 0x2A, 0x33),
                text_icon: Color::rgb(0xF4, 0xF6, 0xFA),
                key_border: Color::rgb(0x4D, 0x53, 0x61),
                window_outline: Color::rgb(0xFF, 0xFF, 0xFF),
                accent: Color::rgb(0x25, 0x63, 0xEB),
            },
            Self::FrostLight => ThemeColors {
                window_bg: Color::rgb(0xE8, 0xEC, 0xF2),
                key_bg: Color::rgb(0xFF, 0xFF, 0xFF),
                modifier_bg: Color::rgb(0xD6, 0xDD, 0xE8),
                text_icon: Color::rgb(0x16, 0x1A, 0x24),
                key_border: Color::rgb(0xC0, 0xC8, 0xD6),
                window_outline: Color::rgb(0x7E, 0x89, 0x9B),
                accent: Color::rgb(0x25, 0x63, 0xEB),
            },
            Self::HighContrastTv => ThemeColors {
                window_bg: Color::rgb(0x0A, 0x0B, 0x0E),
                key_bg: Color::rgb(0x1C, 0x20, 0x28),
                modifier_bg: Color::rgb(0x12, 0x15, 0x1C),
                text_icon: Color::rgb(0xFF, 0xFF, 0xFF),
                key_border: Color::rgb(0x88, 0x92, 0xA6),
                window_outline: Color::rgb(0xFF, 0xFF, 0xFF),
                accent: Color::rgb(0xFF, 0xD4, 0x00),
            },
            Self::Slate => ThemeColors {
                window_bg: Color::rgb(0x1D, 0x24, 0x31),
                key_bg: Color::rgb(0x2E, 0x38, 0x4B),
                modifier_bg: Color::rgb(0x23, 0x2B, 0x3B),
                text_icon: Color::rgb(0xEB, 0xF1, 0xFA),
                key_border: Color::rgb(0x48, 0x56, 0x70),
                window_outline: Color::rgb(0xB8, 0xCA, 0xE6),
                accent: Color::rgb(0x02, 0x84, 0xC7),
            },
        }
    }
}

/// Curated quick-select accent swatches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccentSwatch {
    pub name: &'static str,
    pub color: Color,
}

pub const ACCENT_SWATCHES: &[AccentSwatch] = &[
    AccentSwatch {
        name: "Cobalt",
        color: Color::rgb(0x25, 0x63, 0xEB),
    },
    AccentSwatch {
        name: "Cyan",
        color: Color::rgb(0x02, 0x84, 0xC7),
    },
    AccentSwatch {
        name: "Emerald",
        color: Color::rgb(0x05, 0x96, 0x69),
    },
    AccentSwatch {
        name: "Amber",
        color: Color::rgb(0xD9, 0x77, 0x06),
    },
    AccentSwatch {
        name: "Crimson",
        color: Color::rgb(0xDC, 0x26, 0x26),
    },
    AccentSwatch {
        name: "Violet",
        color: Color::rgb(0x7C, 0x3A, 0xED),
    },
    AccentSwatch {
        name: "Rose",
        color: Color::rgb(0xDB, 0x27, 0x77),
    },
    AccentSwatch {
        name: "Steel",
        color: Color::rgb(0x47, 0x55, 0x69),
    },
];

/// The 7 individually customizable color slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorSlot {
    WindowBg,
    KeyBg,
    ModifierBg,
    TextIcon,
    KeyBorder,
    WindowOutline,
    Accent,
}

impl ColorSlot {
    pub const ALL: &'static [Self] = &[
        Self::WindowBg,
        Self::KeyBg,
        Self::ModifierBg,
        Self::TextIcon,
        Self::KeyBorder,
        Self::WindowOutline,
        Self::Accent,
    ];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::WindowBg => "Window BG",
            Self::KeyBg => "Key BG",
            Self::ModifierBg => "Modifier BG",
            Self::TextIcon => "Text & Icons",
            Self::KeyBorder => "Key Border",
            Self::WindowOutline => "Outline",
            Self::Accent => "Accent",
        }
    }
}

/// The 7 editable theme colors stored in config and user presets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeColors {
    pub window_bg: Color,
    pub key_bg: Color,
    pub modifier_bg: Color,
    pub text_icon: Color,
    pub key_border: Color,
    pub window_outline: Color,
    pub accent: Color,
}

impl Default for ThemeColors {
    fn default() -> Self {
        ThemeBase::GraphiteDark.default_colors()
    }
}

impl ThemeColors {
    #[must_use]
    pub const fn get(&self, slot: ColorSlot) -> Color {
        match slot {
            ColorSlot::WindowBg => self.window_bg,
            ColorSlot::KeyBg => self.key_bg,
            ColorSlot::ModifierBg => self.modifier_bg,
            ColorSlot::TextIcon => self.text_icon,
            ColorSlot::KeyBorder => self.key_border,
            ColorSlot::WindowOutline => self.window_outline,
            ColorSlot::Accent => self.accent,
        }
    }

    pub fn set(&mut self, slot: ColorSlot, color: Color) {
        match slot {
            ColorSlot::WindowBg => self.window_bg = color,
            ColorSlot::KeyBg => self.key_bg = color,
            ColorSlot::ModifierBg => self.modifier_bg = color,
            ColorSlot::TextIcon => self.text_icon = color,
            ColorSlot::KeyBorder => self.key_border = color,
            ColorSlot::WindowOutline => self.window_outline = color,
            ColorSlot::Accent => self.accent = color,
        }
    }
}

fn default_scale_one() -> f32 {
    1.0
}

/// Complete visual appearance configuration (also saved inside User Presets).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppearanceSettings {
    pub base: ThemeBase,
    pub colors: ThemeColors,
    pub outline_width: f32,
    pub key_border_width: f32,
    pub opacity: f32,
    pub font_scale: f32,
    #[serde(default = "default_scale_one")]
    pub letter_font_scale: f32,
    #[serde(default = "default_scale_one")]
    pub sub_label_scale: f32,
    #[serde(default = "default_scale_one")]
    pub modifier_font_scale: f32,
    #[serde(default = "default_scale_one")]
    pub media_icon_scale: f32,
    #[serde(default = "default_scale_one")]
    pub key_icon_scale: f32,
    pub font_weight: u16,
    pub corner_radius: f32,
    pub key_spacing: f32,
    pub keycap_shadow: bool,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self::from_base(ThemeBase::GraphiteDark)
    }
}

impl AppearanceSettings {
    #[must_use]
    pub fn from_base(base: ThemeBase) -> Self {
        let (outline_width, key_border_width) = match base {
            ThemeBase::HighContrastTv => (2.5, 1.5),
            _ => (2.0, 1.0),
        };
        Self {
            base,
            colors: base.default_colors(),
            outline_width,
            key_border_width,
            opacity: 0.96,
            font_scale: 1.0,
            letter_font_scale: 1.0,
            sub_label_scale: 1.0,
            modifier_font_scale: 1.0,
            media_icon_scale: 1.0,
            key_icon_scale: 1.0,
            font_weight: 500,
            corner_radius: 8.0,
            key_spacing: 5.0,
            keycap_shadow: true,
        }
    }

    /// Apply a new theme base while preserving geometry & typography sliders.
    pub fn apply_base(&mut self, base: ThemeBase) {
        self.base = base;
        self.colors = base.default_colors();
        if base == ThemeBase::HighContrastTv {
            self.outline_width = self.outline_width.max(2.0);
            self.key_border_width = self.key_border_width.max(1.5);
        }
    }

    /// Reset all independent font and icon scale factors back to 100% (1.0).
    pub fn reset_sizes(&mut self) {
        self.font_scale = 1.0;
        self.letter_font_scale = 1.0;
        self.sub_label_scale = 1.0;
        self.modifier_font_scale = 1.0;
        self.media_icon_scale = 1.0;
        self.key_icon_scale = 1.0;
        self.font_weight = 500;
    }

    /// Clamp all numeric settings into valid UI bounds.
    pub fn clamp(&mut self) {
        self.outline_width = self.outline_width.clamp(0.0, 4.0);
        self.key_border_width = self.key_border_width.clamp(0.0, 3.0);
        self.opacity = self.opacity.clamp(0.30, 1.00);
        self.font_scale = self.font_scale.clamp(0.75, 1.60);
        self.letter_font_scale = self.letter_font_scale.clamp(0.60, 1.80);
        self.sub_label_scale = self.sub_label_scale.clamp(0.60, 1.80);
        self.modifier_font_scale = self.modifier_font_scale.clamp(0.60, 1.80);
        self.media_icon_scale = self.media_icon_scale.clamp(0.60, 1.80);
        self.key_icon_scale = self.key_icon_scale.clamp(0.60, 1.80);
        self.font_weight = match self.font_weight {
            ..=449 => 400,
            450..=549 => 500,
            550..=649 => 600,
            650.. => 700,
        };
        self.corner_radius = self.corner_radius.clamp(0.0, 16.0);
        self.key_spacing = self.key_spacing.clamp(1.0, 8.0);
    }

    /// Derive full resolved palette including hover, press, AltGr amber highlights, and contrast-safe text.
    #[must_use]
    pub fn resolved_palette(&self) -> ResolvedPalette {
        let is_light = self.colors.window_bg.relative_luminance() > 0.35;
        let hover_delta = if is_light { -0.07 } else { 0.10 };
        let press_delta = if is_light { -0.14 } else { 0.20 };

        let key_hover = self.colors.key_bg.adjust_brightness(hover_delta);
        let key_pressed = self.colors.key_bg.adjust_brightness(press_delta);
        let modifier_hover = self.colors.modifier_bg.adjust_brightness(hover_delta);
        let toolbar_bg = if is_light {
            self.colors.window_bg.adjust_brightness(-0.04)
        } else {
            self.colors.window_bg.adjust_brightness(0.04)
        };
        let muted_text = self.colors.text_icon.mix(self.colors.key_bg, 0.38);
        let accent_text = self.colors.accent.contrasting_text();
        let altgr_amber = if is_light {
            Color::rgb(0xD9, 0x77, 0x06)
        } else {
            Color::rgb(0xF5, 0x9E, 0x0B)
        };
        let shadow_color = if is_light {
            Color::rgba(0, 0, 0, 38)
        } else {
            Color::rgba(0, 0, 0, 115)
        };

        ResolvedPalette {
            colors: self.colors,
            is_light,
            key_hover,
            key_pressed,
            modifier_hover,
            toolbar_bg,
            muted_text,
            accent_text,
            altgr_amber,
            shadow_color,
        }
    }
}

/// Ready-to-render palette derived from `AppearanceSettings`.
#[derive(Debug, Clone, Copy)]
pub struct ResolvedPalette {
    pub colors: ThemeColors,
    pub is_light: bool,
    pub key_hover: Color,
    pub key_pressed: Color,
    pub modifier_hover: Color,
    pub toolbar_bg: Color,
    pub muted_text: Color,
    pub accent_text: Color,
    pub altgr_amber: Color,
    pub shadow_color: Color,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_builtin_themes_exceed_wcag_aa_contrast() {
        for base in ThemeBase::ALL {
            let colors = base.default_colors();
            let key_ratio = colors.text_icon.contrast_ratio(colors.key_bg);
            let mod_ratio = colors.text_icon.contrast_ratio(colors.modifier_bg);
            assert!(
                key_ratio >= 7.0,
                "{:?} key contrast too low: {key_ratio:.2}",
                base
            );
            assert!(
                mod_ratio >= 7.0,
                "{:?} modifier contrast too low: {mod_ratio:.2}",
                base
            );
        }
    }

    #[test]
    fn graphite_dark_has_crisp_white_outline_by_default() {
        let settings = AppearanceSettings::default();
        assert_eq!(settings.base, ThemeBase::GraphiteDark);
        assert_eq!(settings.colors.window_outline, Color::rgb(255, 255, 255));
        assert!((settings.outline_width - 2.0).abs() < f32::EPSILON);
    }

    #[test]
    fn hsv_and_hex_round_trip() {
        let cobalt = Color::from_hex("#2F81F7").expect("valid hex");
        assert_eq!(cobalt.to_hex(), "#2F81F7");

        let (h, s, v) = cobalt.to_hsv();
        let rebuilt = Color::from_hsv(h, s, v);
        assert!((i16::from(cobalt.r) - i16::from(rebuilt.r)).abs() <= 1);
        assert!((i16::from(cobalt.g) - i16::from(rebuilt.g)).abs() <= 1);
        assert!((i16::from(cobalt.b) - i16::from(rebuilt.b)).abs() <= 1);
    }
}
