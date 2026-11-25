//! Modifier state machine with double-tap Sticky Modifier Lock and XKB bitmask calculation.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::keycode::{KeyCode, ModifierKind};

/// Double-tap interval window to latch a modifier into Sticky Lock mode.
pub const DOUBLE_TAP_WINDOW: Duration = Duration::from_millis(400);

/// State of an individual modifier key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ModifierLevel {
    #[default]
    Off,
    /// Active for the next non-modifier keypress, then released automatically.
    OneShot,
    /// Locked active until tapped again (double-tap Sticky Modifier Lock or CapsLock).
    Locked,
}

impl ModifierLevel {
    #[must_use]
    pub const fn is_active(self) -> bool {
        !matches!(self, Self::Off)
    }

    #[must_use]
    pub const fn is_locked(self) -> bool {
        matches!(self, Self::Locked)
    }
}

/// Effect returned when toggling or consuming modifiers so the input backend stays synchronized.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModifierTransition {
    pub press_codes: Vec<KeyCode>,
    pub release_codes: Vec<KeyCode>,
    pub tap_codes: Vec<KeyCode>,
}

/// Tracks Shift, CapsLock, Ctrl, Alt, AltGr, and Super states.
#[derive(Debug, Clone)]
pub struct ModifierTracker {
    levels: HashMap<ModifierKind, ModifierLevel>,
    last_tap: HashMap<ModifierKind, Instant>,
    /// Modifiers physically held down by an active touch or mouse button press.
    physically_held: HashMap<ModifierKind, usize>,
}

impl Default for ModifierTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl ModifierTracker {
    #[must_use]
    pub fn new() -> Self {
        Self {
            levels: HashMap::new(),
            last_tap: HashMap::new(),
            physically_held: HashMap::new(),
        }
    }

    /// Logical latch state (`Off`, `OneShot`, `Locked`) for a modifier.
    #[must_use]
    pub fn level(&self, kind: ModifierKind) -> ModifierLevel {
        self.levels
            .get(&kind)
            .copied()
            .unwrap_or(ModifierLevel::Off)
    }

    /// Whether the modifier is active either logically or via physical multi-touch hold.
    #[must_use]
    pub fn is_active(&self, kind: ModifierKind) -> bool {
        self.level(kind).is_active() || self.physically_held.get(&kind).copied().unwrap_or(0) > 0
    }

    #[must_use]
    pub fn is_locked(&self, kind: ModifierKind) -> bool {
        self.level(kind).is_locked()
    }

    /// Whether uppercase letters should be displayed (Shift XOR CapsLock).
    #[must_use]
    pub fn effective_uppercase(&self) -> bool {
        self.is_active(ModifierKind::Shift) ^ self.is_active(ModifierKind::CapsLock)
    }

    /// Whether shifted punctuation/digits should be displayed (Shift only).
    #[must_use]
    pub fn effective_shift(&self) -> bool {
        self.is_active(ModifierKind::Shift)
    }

    #[must_use]
    pub fn effective_altgr(&self) -> bool {
        self.is_active(ModifierKind::AltGr)
    }

    /// Mark a modifier as physically held (for multi-touch chording: hold Shift + tap letter).
    pub fn begin_hold(&mut self, kind: ModifierKind) -> ModifierTransition {
        let mut transition = ModifierTransition::default();
        if kind == ModifierKind::CapsLock {
            return transition;
        }
        let was_active = self.is_active(kind);
        *self.physically_held.entry(kind).or_insert(0) += 1;
        if !was_active {
            transition.press_codes.push(kind.keycode());
        }
        transition
    }

    /// Release a physical hold; if it was a quick tap (no character typed during hold),
    /// toggle OneShot / Sticky Lock state.
    pub fn end_hold(
        &mut self,
        kind: ModifierKind,
        chording_used: bool,
        sticky_lock_enabled: bool,
        now: Instant,
    ) -> ModifierTransition {
        let mut transition = ModifierTransition::default();
        if kind == ModifierKind::CapsLock {
            let next = if self.level(ModifierKind::CapsLock).is_active() {
                ModifierLevel::Off
            } else {
                ModifierLevel::Locked
            };
            self.levels.insert(ModifierKind::CapsLock, next);
            transition.tap_codes.push(KeyCode::CAPSLOCK);
            return transition;
        }

        if let Some(count) = self.physically_held.get_mut(&kind) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.physically_held.remove(&kind);
            }
        }

        if chording_used {
            // Modifier was used as a chord (held while tapping another key); release it now.
            if !self.is_active(kind) {
                transition.release_codes.push(kind.keycode());
            }
            self.last_tap.remove(&kind);
            return transition;
        }

        let current = self.level(kind);
        let is_double_tap = sticky_lock_enabled
            && self
                .last_tap
                .get(&kind)
                .is_some_and(|prev| now.duration_since(*prev) <= DOUBLE_TAP_WINDOW);

        match current {
            ModifierLevel::Off => {
                self.levels.insert(kind, ModifierLevel::OneShot);
                self.last_tap.insert(kind, now);
                // Key is already pressed from begin_hold; keep it pressed for the next character.
            }
            ModifierLevel::OneShot => {
                if is_double_tap {
                    self.levels.insert(kind, ModifierLevel::Locked);
                    self.last_tap.remove(&kind);
                    // Remains pressed in locked state.
                } else {
                    self.levels.insert(kind, ModifierLevel::Off);
                    self.last_tap.remove(&kind);
                    if !self.is_active(kind) {
                        transition.release_codes.push(kind.keycode());
                    }
                }
            }
            ModifierLevel::Locked => {
                self.levels.insert(kind, ModifierLevel::Off);
                self.last_tap.remove(&kind);
                if !self.is_active(kind) {
                    transition.release_codes.push(kind.keycode());
                }
            }
        }
        transition
    }

    /// Consume all `OneShot` modifiers after a non-modifier key is typed.
    pub fn consume_one_shot(&mut self) -> ModifierTransition {
        let mut transition = ModifierTransition::default();
        let kinds = [
            ModifierKind::Shift,
            ModifierKind::Ctrl,
            ModifierKind::Alt,
            ModifierKind::AltGr,
            ModifierKind::Super,
        ];
        for kind in kinds {
            if self.level(kind) == ModifierLevel::OneShot {
                self.levels.insert(kind, ModifierLevel::Off);
                self.last_tap.remove(&kind);
                if !self.is_active(kind) {
                    transition.release_codes.push(kind.keycode());
                }
            }
        }
        transition
    }

    /// Clear every active modifier (used on hide, layout reset, or panic recovery).
    pub fn clear_all(&mut self) -> ModifierTransition {
        let mut transition = ModifierTransition::default();
        let kinds = [
            ModifierKind::Shift,
            ModifierKind::Ctrl,
            ModifierKind::Alt,
            ModifierKind::AltGr,
            ModifierKind::Super,
        ];
        for kind in kinds {
            if self.is_active(kind) {
                transition.release_codes.push(kind.keycode());
            }
        }
        self.levels.clear();
        self.last_tap.clear();
        self.physically_held.clear();
        transition
    }

    /// Compute `(depressed, locked)` XKB modifier bitmasks for `zwp_virtual_keyboard_v1`.
    #[must_use]
    pub fn xkb_masks(&self) -> (u32, u32) {
        let mut depressed = 0u32;
        let mut locked = 0u32;

        for kind in [
            ModifierKind::Shift,
            ModifierKind::Ctrl,
            ModifierKind::Alt,
            ModifierKind::AltGr,
            ModifierKind::Super,
        ] {
            if self.is_active(kind) {
                depressed |= kind.xkb_mask();
            }
        }
        if self.is_active(ModifierKind::CapsLock) {
            locked |= ModifierKind::CapsLock.xkb_mask();
        }
        (depressed, locked)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_tap_activates_one_shot_and_consumes_after_key() {
        let mut mods = ModifierTracker::new();
        let t0 = Instant::now();

        let press = mods.begin_hold(ModifierKind::Shift);
        assert_eq!(press.press_codes, vec![KeyCode::LEFTSHIFT]);

        let release = mods.end_hold(ModifierKind::Shift, false, true, t0);
        assert!(release.release_codes.is_empty());
        assert_eq!(mods.level(ModifierKind::Shift), ModifierLevel::OneShot);
        assert!(mods.effective_shift());

        let consumed = mods.consume_one_shot();
        assert_eq!(consumed.release_codes, vec![KeyCode::LEFTSHIFT]);
        assert_eq!(mods.level(ModifierKind::Shift), ModifierLevel::Off);
        assert!(!mods.effective_shift());
    }

    #[test]
    fn double_tap_locks_modifier_when_sticky_enabled() {
        let mut mods = ModifierTracker::new();
        let t0 = Instant::now();
        let t1 = t0 + Duration::from_millis(180);

        mods.begin_hold(ModifierKind::AltGr);
        mods.end_hold(ModifierKind::AltGr, false, true, t0);
        assert_eq!(mods.level(ModifierKind::AltGr), ModifierLevel::OneShot);

        mods.begin_hold(ModifierKind::AltGr);
        let rel = mods.end_hold(ModifierKind::AltGr, false, true, t1);
        assert!(rel.release_codes.is_empty());
        assert_eq!(mods.level(ModifierKind::AltGr), ModifierLevel::Locked);

        // Consuming one-shot must keep locked modifier active
        let consumed = mods.consume_one_shot();
        assert!(consumed.release_codes.is_empty());
        assert!(mods.effective_altgr());

        // Third tap turns lock off
        let t2 = t1 + Duration::from_millis(500);
        mods.begin_hold(ModifierKind::AltGr);
        let off = mods.end_hold(ModifierKind::AltGr, false, true, t2);
        assert_eq!(off.release_codes, vec![KeyCode::RIGHTALT]);
        assert_eq!(mods.level(ModifierKind::AltGr), ModifierLevel::Off);
    }

    #[test]
    fn chording_releases_modifier_immediately_without_latching() {
        let mut mods = ModifierTracker::new();
        let t0 = Instant::now();

        mods.begin_hold(ModifierKind::Shift);
        let rel = mods.end_hold(ModifierKind::Shift, true, true, t0);
        assert_eq!(rel.release_codes, vec![KeyCode::LEFTSHIFT]);
        assert_eq!(mods.level(ModifierKind::Shift), ModifierLevel::Off);
    }

    #[test]
    fn capslock_xor_shift_controls_letter_case() {
        let mut mods = ModifierTracker::new();
        let t0 = Instant::now();
        let tap = mods.end_hold(ModifierKind::CapsLock, false, true, t0);
        assert_eq!(tap.tap_codes, vec![KeyCode::CAPSLOCK]);
        assert!(mods.effective_uppercase());

        mods.begin_hold(ModifierKind::Shift);
        assert!(!mods.effective_uppercase());
        assert!(mods.effective_shift());
    }
}
