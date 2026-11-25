//! Wayland `zwp_input_method_v1` input backend for KDE Plasma Virtual Keyboard,
//! KDE Plasma Lockscreen (`kscreenlocker`), and SDDM Login Screen.

use std::time::Instant;

use tracing::debug;
use wayland_protocols::wp::input_method::zv1::client::zwp_input_method_context_v1::ZwpInputMethodContextV1;
use wlboard_core::{KeyCode, ModifierKind};

use crate::input::{InputBackend, InputError};

/// Standard modifier map advertised via `zwp_input_method_context_v1::modifiers_map`.
const IM_MODIFIERS_MAP: &[u8] = b"Shift\0Control\0Mod1\0Mod4\0Mod5\0";

/// Wayland `zwp_input_method_v1` backend supporting direct Unicode `commit_string`,
/// X11 `keysym` dispatch, and optional fallback backend for media/hardware keys.
pub struct WaylandInputMethodBackend {
    context: Option<ZwpInputMethodContextV1>,
    serial: u32,
    started_at: Instant,
    fallback: Option<Box<dyn InputBackend>>,
}

impl Default for WaylandInputMethodBackend {
    fn default() -> Self {
        Self::new(None)
    }
}

impl WaylandInputMethodBackend {
    #[must_use]
    pub fn new(fallback: Option<Box<dyn InputBackend>>) -> Self {
        Self {
            context: None,
            serial: 0,
            started_at: Instant::now(),
            fallback,
        }
    }

    pub fn activate(&mut self, ctx: ZwpInputMethodContextV1) {
        if let Some(old) = self.context.take() {
            old.destroy();
        }
        ctx.modifiers_map(IM_MODIFIERS_MAP.to_vec());
        self.context = Some(ctx);
        self.serial = 0;
        debug!("Wayland input-method context activated");
    }

    pub fn deactivate(&mut self) {
        if let Some(old) = self.context.take() {
            old.destroy();
        }
        debug!("Wayland input-method context deactivated");
    }

    pub fn update_serial(&mut self, serial: u32) {
        self.serial = serial;
    }

    #[must_use]
    pub fn is_active(&self) -> bool {
        self.context.is_some()
    }

    fn timestamp_ms(&self) -> u32 {
        self.started_at.elapsed().as_millis() as u32
    }

    fn im_modifier_mask(modifiers: &[ModifierKind]) -> u32 {
        let mut mask = 0u32;
        for m in modifiers {
            mask |= match m {
                ModifierKind::Shift => 1 << 0,
                ModifierKind::Ctrl => 1 << 1,
                ModifierKind::Alt => 1 << 2,
                ModifierKind::Super => 1 << 3,
                ModifierKind::AltGr => 1 << 4,
                ModifierKind::CapsLock => 0,
            };
        }
        mask
    }

    /// Commit UTF-8 text directly into the active Wayland text-input / lockscreen password field.
    pub fn commit_text(&mut self, text: &str) -> bool {
        if let Some(ref ctx) = self.context {
            ctx.commit_string(self.serial, text.to_owned());
            return true;
        }
        false
    }

    /// Send an X11 keysym tap with optional modifier bitmask via `zwp_input_method_context_v1::keysym`.
    pub fn send_keysym_tap(&mut self, keycode: KeyCode, modifiers: &[ModifierKind]) -> bool {
        let Some(ref ctx) = self.context else {
            return false;
        };
        let Some(sym) = keycode.to_x11_keysym() else {
            return false;
        };
        let ts = self.timestamp_ms();
        let mask = Self::im_modifier_mask(modifiers);
        ctx.keysym(self.serial, ts, sym, 1, mask);
        ctx.keysym(self.serial, ts.saturating_add(1), sym, 0, mask);
        true
    }
}

impl InputBackend for WaylandInputMethodBackend {
    fn name(&self) -> &'static str {
        "Wayland Input Method (zwp_input_method_v1 — Lockscreen Ready)"
    }

    fn press_key(&mut self, code: KeyCode) -> Result<(), InputError> {
        if let Some(ref ctx) = self.context
            && let Some(sym) = code.to_x11_keysym()
        {
            ctx.keysym(self.serial, self.timestamp_ms(), sym, 1, 0);
            return Ok(());
        }
        if let Some(ref mut fb) = self.fallback {
            return fb.press_key(code);
        }
        Ok(())
    }

    fn release_key(&mut self, code: KeyCode) -> Result<(), InputError> {
        if let Some(ref ctx) = self.context
            && let Some(sym) = code.to_x11_keysym()
        {
            ctx.keysym(self.serial, self.timestamp_ms(), sym, 0, 0);
            return Ok(());
        }
        if let Some(ref mut fb) = self.fallback {
            return fb.release_key(code);
        }
        Ok(())
    }

    fn type_text(&mut self, text: &str) -> Result<(), InputError> {
        if self.commit_text(text) {
            return Ok(());
        }
        if let Some(ref mut fb) = self.fallback {
            return fb.type_text(text);
        }
        Ok(())
    }

    fn send_shortcut(
        &mut self,
        modifiers: &[ModifierKind],
        keycode: KeyCode,
    ) -> Result<(), InputError> {
        if self.send_keysym_tap(keycode, modifiers) {
            return Ok(());
        }
        if let Some(ref mut fb) = self.fallback {
            return fb.send_shortcut(modifiers, keycode);
        }
        Ok(())
    }

    fn release_all(&mut self) -> Result<(), InputError> {
        if let Some(ref mut fb) = self.fallback {
            fb.release_all()?;
        }
        Ok(())
    }

    fn watchdog_tick(
        &mut self,
        active_held: &[KeyCode],
        now: Instant,
    ) -> Result<usize, InputError> {
        if let Some(ref mut fb) = self.fallback {
            return fb.watchdog_tick(active_held, now);
        }
        Ok(0)
    }
}

impl Drop for WaylandInputMethodBackend {
    fn drop(&mut self) {
        self.deactivate();
    }
}
