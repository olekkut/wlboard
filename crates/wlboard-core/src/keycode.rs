//! Strongly-typed Linux evdev keycodes and key role classifications.

use serde::{Deserialize, Serialize};

/// Linux evdev keycode matching `<linux/input-event-codes.h>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct KeyCode(pub u16);

impl KeyCode {
    pub const ESC: Self = Self(1);
    pub const KEY_1: Self = Self(2);
    pub const KEY_2: Self = Self(3);
    pub const KEY_3: Self = Self(4);
    pub const KEY_4: Self = Self(5);
    pub const KEY_5: Self = Self(6);
    pub const KEY_6: Self = Self(7);
    pub const KEY_7: Self = Self(8);
    pub const KEY_8: Self = Self(9);
    pub const KEY_9: Self = Self(10);
    pub const KEY_0: Self = Self(11);
    pub const MINUS: Self = Self(12);
    pub const EQUAL: Self = Self(13);
    pub const BACKSPACE: Self = Self(14);
    pub const TAB: Self = Self(15);
    pub const Q: Self = Self(16);
    pub const W: Self = Self(17);
    pub const E: Self = Self(18);
    pub const R: Self = Self(19);
    pub const T: Self = Self(20);
    pub const Y: Self = Self(21);
    pub const U: Self = Self(22);
    pub const I: Self = Self(23);
    pub const O: Self = Self(24);
    pub const P: Self = Self(25);
    pub const LEFTBRACE: Self = Self(26);
    pub const RIGHTBRACE: Self = Self(27);
    pub const ENTER: Self = Self(28);
    pub const LEFTCTRL: Self = Self(29);
    pub const A: Self = Self(30);
    pub const S: Self = Self(31);
    pub const D: Self = Self(32);
    pub const F: Self = Self(33);
    pub const G: Self = Self(34);
    pub const H: Self = Self(35);
    pub const J: Self = Self(36);
    pub const K: Self = Self(37);
    pub const L: Self = Self(38);
    pub const SEMICOLON: Self = Self(39);
    pub const APOSTROPHE: Self = Self(40);
    pub const GRAVE: Self = Self(41);
    pub const LEFTSHIFT: Self = Self(42);
    pub const BACKSLASH: Self = Self(43);
    pub const Z: Self = Self(44);
    pub const X: Self = Self(45);
    pub const C: Self = Self(46);
    pub const V: Self = Self(47);
    pub const B: Self = Self(48);
    pub const N: Self = Self(49);
    pub const M: Self = Self(50);
    pub const COMMA: Self = Self(51);
    pub const DOT: Self = Self(52);
    pub const SLASH: Self = Self(53);
    pub const RIGHTSHIFT: Self = Self(54);
    pub const KPASTERISK: Self = Self(55);
    pub const LEFTALT: Self = Self(56);
    pub const SPACE: Self = Self(57);
    pub const CAPSLOCK: Self = Self(58);
    pub const F1: Self = Self(59);
    pub const F2: Self = Self(60);
    pub const F3: Self = Self(61);
    pub const F4: Self = Self(62);
    pub const F5: Self = Self(63);
    pub const F6: Self = Self(64);
    pub const F7: Self = Self(65);
    pub const F8: Self = Self(66);
    pub const F9: Self = Self(67);
    pub const F10: Self = Self(68);
    pub const NUMLOCK: Self = Self(69);
    pub const KP7: Self = Self(71);
    pub const KP8: Self = Self(72);
    pub const KP9: Self = Self(73);
    pub const KPMINUS: Self = Self(74);
    pub const KP4: Self = Self(75);
    pub const KP5: Self = Self(76);
    pub const KP6: Self = Self(77);
    pub const KPPLUS: Self = Self(78);
    pub const KP1: Self = Self(79);
    pub const KP2: Self = Self(80);
    pub const KP3: Self = Self(81);
    pub const KP0: Self = Self(82);
    pub const KPDOT: Self = Self(83);
    pub const F11: Self = Self(87);
    pub const F12: Self = Self(88);
    pub const KPENTER: Self = Self(96);
    pub const RIGHTCTRL: Self = Self(97);
    pub const KPSLASH: Self = Self(98);
    pub const SYSRQ: Self = Self(99);
    pub const RIGHTALT: Self = Self(100);
    pub const HOME: Self = Self(102);
    pub const UP: Self = Self(103);
    pub const PAGEUP: Self = Self(104);
    pub const LEFT: Self = Self(105);
    pub const RIGHT: Self = Self(106);
    pub const END: Self = Self(107);
    pub const DOWN: Self = Self(108);
    pub const PAGEDOWN: Self = Self(109);
    pub const INSERT: Self = Self(110);
    pub const DELETE: Self = Self(111);
    pub const MUTE: Self = Self(113);
    pub const VOLUMEDOWN: Self = Self(114);
    pub const VOLUMEUP: Self = Self(115);
    pub const LEFTMETA: Self = Self(125);
    pub const NEXTSONG: Self = Self(163);
    pub const PLAYPAUSE: Self = Self(164);
    pub const PREVIOUSSONG: Self = Self(165);
    pub const BRIGHTNESSDOWN: Self = Self(224);
    pub const BRIGHTNESSUP: Self = Self(225);

    /// Raw Linux evdev scan code.
    #[must_use]
    pub const fn raw(self) -> u16 {
        self.0
    }

    /// Resolve a standard `KEY_*` name from a JSON layout file.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "KEY_ESC" => Self::ESC,
            "KEY_1" => Self::KEY_1,
            "KEY_2" => Self::KEY_2,
            "KEY_3" => Self::KEY_3,
            "KEY_4" => Self::KEY_4,
            "KEY_5" => Self::KEY_5,
            "KEY_6" => Self::KEY_6,
            "KEY_7" => Self::KEY_7,
            "KEY_8" => Self::KEY_8,
            "KEY_9" => Self::KEY_9,
            "KEY_0" => Self::KEY_0,
            "KEY_MINUS" => Self::MINUS,
            "KEY_EQUAL" => Self::EQUAL,
            "KEY_BACKSPACE" => Self::BACKSPACE,
            "KEY_TAB" => Self::TAB,
            "KEY_Q" => Self::Q,
            "KEY_W" => Self::W,
            "KEY_E" => Self::E,
            "KEY_R" => Self::R,
            "KEY_T" => Self::T,
            "KEY_Y" => Self::Y,
            "KEY_U" => Self::U,
            "KEY_I" => Self::I,
            "KEY_O" => Self::O,
            "KEY_P" => Self::P,
            "KEY_LEFTBRACE" => Self::LEFTBRACE,
            "KEY_RIGHTBRACE" => Self::RIGHTBRACE,
            "KEY_ENTER" => Self::ENTER,
            "KEY_LEFTCTRL" => Self::LEFTCTRL,
            "KEY_A" => Self::A,
            "KEY_S" => Self::S,
            "KEY_D" => Self::D,
            "KEY_F" => Self::F,
            "KEY_G" => Self::G,
            "KEY_H" => Self::H,
            "KEY_J" => Self::J,
            "KEY_K" => Self::K,
            "KEY_L" => Self::L,
            "KEY_SEMICOLON" => Self::SEMICOLON,
            "KEY_APOSTROPHE" => Self::APOSTROPHE,
            "KEY_GRAVE" => Self::GRAVE,
            "KEY_LEFTSHIFT" => Self::LEFTSHIFT,
            "KEY_BACKSLASH" => Self::BACKSLASH,
            "KEY_Z" => Self::Z,
            "KEY_X" => Self::X,
            "KEY_C" => Self::C,
            "KEY_V" => Self::V,
            "KEY_B" => Self::B,
            "KEY_N" => Self::N,
            "KEY_M" => Self::M,
            "KEY_COMMA" => Self::COMMA,
            "KEY_DOT" => Self::DOT,
            "KEY_SLASH" => Self::SLASH,
            "KEY_RIGHTSHIFT" => Self::RIGHTSHIFT,
            "KEY_LEFTALT" => Self::LEFTALT,
            "KEY_SPACE" => Self::SPACE,
            "KEY_CAPSLOCK" => Self::CAPSLOCK,
            "KEY_F1" => Self::F1,
            "KEY_F2" => Self::F2,
            "KEY_F3" => Self::F3,
            "KEY_F4" => Self::F4,
            "KEY_F5" => Self::F5,
            "KEY_F6" => Self::F6,
            "KEY_F7" => Self::F7,
            "KEY_F8" => Self::F8,
            "KEY_F9" => Self::F9,
            "KEY_F10" => Self::F10,
            "KEY_F11" => Self::F11,
            "KEY_F12" => Self::F12,
            "KEY_RIGHTCTRL" => Self::RIGHTCTRL,
            "KEY_SYSRQ" => Self::SYSRQ,
            "KEY_RIGHTALT" => Self::RIGHTALT,
            "KEY_HOME" => Self::HOME,
            "KEY_UP" => Self::UP,
            "KEY_PAGEUP" => Self::PAGEUP,
            "KEY_LEFT" => Self::LEFT,
            "KEY_RIGHT" => Self::RIGHT,
            "KEY_END" => Self::END,
            "KEY_DOWN" => Self::DOWN,
            "KEY_PAGEDOWN" => Self::PAGEDOWN,
            "KEY_INSERT" => Self::INSERT,
            "KEY_DELETE" => Self::DELETE,
            "KEY_MUTE" => Self::MUTE,
            "KEY_VOLUMEDOWN" => Self::VOLUMEDOWN,
            "KEY_VOLUMEUP" => Self::VOLUMEUP,
            "KEY_LEFTMETA" => Self::LEFTMETA,
            "KEY_NEXTSONG" => Self::NEXTSONG,
            "KEY_PLAYPAUSE" => Self::PLAYPAUSE,
            "KEY_PREVIOUSSONG" => Self::PREVIOUSSONG,
            "KEY_BRIGHTNESSDOWN" => Self::BRIGHTNESSDOWN,
            "KEY_BRIGHTNESSUP" => Self::BRIGHTNESSUP,
            _ => return None,
        })
    }

    /// Complete set of evdev keycodes registered on a virtual uinput device.
    #[must_use]
    pub const fn all_supported() -> &'static [Self] {
        &[
            Self::ESC,
            Self::KEY_1,
            Self::KEY_2,
            Self::KEY_3,
            Self::KEY_4,
            Self::KEY_5,
            Self::KEY_6,
            Self::KEY_7,
            Self::KEY_8,
            Self::KEY_9,
            Self::KEY_0,
            Self::MINUS,
            Self::EQUAL,
            Self::BACKSPACE,
            Self::TAB,
            Self::Q,
            Self::W,
            Self::E,
            Self::R,
            Self::T,
            Self::Y,
            Self::U,
            Self::I,
            Self::O,
            Self::P,
            Self::LEFTBRACE,
            Self::RIGHTBRACE,
            Self::ENTER,
            Self::LEFTCTRL,
            Self::A,
            Self::S,
            Self::D,
            Self::F,
            Self::G,
            Self::H,
            Self::J,
            Self::K,
            Self::L,
            Self::SEMICOLON,
            Self::APOSTROPHE,
            Self::GRAVE,
            Self::LEFTSHIFT,
            Self::BACKSLASH,
            Self::Z,
            Self::X,
            Self::C,
            Self::V,
            Self::B,
            Self::N,
            Self::M,
            Self::COMMA,
            Self::DOT,
            Self::SLASH,
            Self::RIGHTSHIFT,
            Self::KPASTERISK,
            Self::LEFTALT,
            Self::SPACE,
            Self::CAPSLOCK,
            Self::F1,
            Self::F2,
            Self::F3,
            Self::F4,
            Self::F5,
            Self::F6,
            Self::F7,
            Self::F8,
            Self::F9,
            Self::F10,
            Self::NUMLOCK,
            Self::KP7,
            Self::KP8,
            Self::KP9,
            Self::KPMINUS,
            Self::KP4,
            Self::KP5,
            Self::KP6,
            Self::KPPLUS,
            Self::KP1,
            Self::KP2,
            Self::KP3,
            Self::KP0,
            Self::KPDOT,
            Self::F11,
            Self::F12,
            Self::KPENTER,
            Self::RIGHTCTRL,
            Self::KPSLASH,
            Self::SYSRQ,
            Self::RIGHTALT,
            Self::HOME,
            Self::UP,
            Self::PAGEUP,
            Self::LEFT,
            Self::RIGHT,
            Self::END,
            Self::DOWN,
            Self::PAGEDOWN,
            Self::INSERT,
            Self::DELETE,
            Self::MUTE,
            Self::VOLUMEDOWN,
            Self::VOLUMEUP,
            Self::LEFTMETA,
            Self::NEXTSONG,
            Self::PLAYPAUSE,
            Self::PREVIOUSSONG,
            Self::BRIGHTNESSDOWN,
            Self::BRIGHTNESSUP,
        ]
    }
}

/// Logical modifier keys supported by `wlboard`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ModifierKind {
    Shift,
    CapsLock,
    Ctrl,
    Alt,
    AltGr,
    Super,
}

impl ModifierKind {
    /// Primary Linux evdev scan code for this modifier.
    #[must_use]
    pub const fn keycode(self) -> KeyCode {
        match self {
            Self::Shift => KeyCode::LEFTSHIFT,
            Self::CapsLock => KeyCode::CAPSLOCK,
            Self::Ctrl => KeyCode::LEFTCTRL,
            Self::Alt => KeyCode::LEFTALT,
            Self::AltGr => KeyCode::RIGHTALT,
            Self::Super => KeyCode::LEFTMETA,
        }
    }

    /// Standard XKB modifier bitmask bit for Wayland virtual keyboard protocol.
    #[must_use]
    pub const fn xkb_mask(self) -> u32 {
        match self {
            Self::Shift => 1 << 0,
            Self::CapsLock => 1 << 1,
            Self::Ctrl => 1 << 2,
            Self::Alt => 1 << 3,
            Self::Super => 1 << 6,
            Self::AltGr => 1 << 7,
        }
    }
}

/// Semantic icon keys rendered via embedded monochrome SVG icons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IconId {
    Menu,
    Settings,
    Backspace,
    Close,
    ResizeGrip,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    BrightnessDown,
    BrightnessUp,
    MediaPrev,
    MediaPlayPause,
    MediaNext,
    AudioMute,
    AudioLow,
    AudioHigh,
    Screenshot,
}

/// Functional category of a keycap (controls visual styling & behavior).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyRole {
    Character,
    Space,
    Modifier(ModifierKind),
    Action,
    Media,
    LayerToggle,
}
