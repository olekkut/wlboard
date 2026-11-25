//! Input injection abstraction with auto-selection
//! (`zwp_input_method_v1` in IM mode, or `zwp_virtual_keyboard_v1` -> `/dev/uinput` -> null)
//! and 250 ms stuck-key watchdog.

pub mod input_method;
pub mod null;
pub mod uinput;
pub mod wayland_vk;
pub mod xkb_keymap;

use std::time::{Duration, Instant};

use thiserror::Error;
use tracing::{info, warn};
use wayland_protocols::wp::input_method::zv1::client::zwp_input_method_context_v1::ZwpInputMethodContextV1;
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1;
use wlboard_core::{InputCommand, KeyCode, LayoutDefinition, ModifierKind};

pub use input_method::WaylandInputMethodBackend;
pub use null::NullInputBackend;
pub use uinput::UInputBackend;
pub use wayland_vk::WaylandVirtualKeyboardBackend;

/// Orphaned held keys without an active contact or modifier lock are forcefully released after 250 ms.
pub const STUCK_KEY_TIMEOUT: Duration = Duration::from_millis(250);

#[derive(Debug, Error)]
pub enum InputError {
    #[error("I/O error on input backend: {0}")]
    Io(#[from] std::io::Error),
}

/// Unified key injection interface implemented by Wayland virtual keyboard, uinput, input-method, and null backends.
pub trait InputBackend: Send {
    fn name(&self) -> &'static str;

    fn status_detail(&self) -> String {
        self.name().to_owned()
    }

    fn press_key(&mut self, code: KeyCode) -> Result<(), InputError>;
    fn release_key(&mut self, code: KeyCode) -> Result<(), InputError>;

    fn tap_key(&mut self, code: KeyCode) -> Result<(), InputError> {
        self.press_key(code)?;
        self.release_key(code)
    }

    /// Type a UTF-8 string (Text Snippet or direct Unicode commit). Default translates ASCII via evdev.
    fn type_text(&mut self, text: &str) -> Result<(), InputError> {
        for ch in text.chars() {
            if let Some((kc, need_shift)) = KeyCode::from_ascii(ch) {
                if need_shift {
                    self.press_key(KeyCode::LEFTSHIFT)?;
                }
                self.tap_key(kc)?;
                if need_shift {
                    self.release_key(KeyCode::LEFTSHIFT)?;
                }
            }
        }
        Ok(())
    }

    /// Emit a multi-key shortcut chord cleanly (e.g. `Ctrl+C`, `Alt+Tab`).
    fn send_shortcut(
        &mut self,
        modifiers: &[ModifierKind],
        keycode: KeyCode,
    ) -> Result<(), InputError> {
        for m in modifiers {
            self.press_key(m.keycode())?;
        }
        self.tap_key(keycode)?;
        for m in modifiers.iter().rev() {
            self.release_key(m.keycode())?;
        }
        Ok(())
    }

    fn sync_modifiers(&mut self, _depressed: u32, _locked: u32) -> Result<(), InputError> {
        Ok(())
    }

    fn apply_layout(&mut self, _layout: &LayoutDefinition) -> Result<(), InputError> {
        Ok(())
    }

    fn release_all(&mut self) -> Result<(), InputError>;

    fn watchdog_tick(&mut self, active_held: &[KeyCode], now: Instant)
    -> Result<usize, InputError>;
}

/// Auto-selects the best available Wayland input injection backend:
/// 1. `WaylandInputMethodBackend` when launched in KDE Plasma `--input-method` (Lockscreen/Virtual Keyboard) mode.
/// 2. `zwp_virtual_keyboard_v1` if advertised by the compositor (Hyprland, Sway, wlroots).
/// 3. Pure-Rust `/dev/uinput` fallback with 250 ms stuck-key watchdog (KDE Plasma Wayland desktop).
/// 4. Safe read-only `NullInputBackend` reporting setup guidance in Settings.
pub struct InputManager {
    backend: Box<dyn InputBackend>,
    im_backend: Option<WaylandInputMethodBackend>,
    uses_wayland_vk: bool,
}

impl InputManager {
    #[must_use]
    pub fn auto_select(
        wayland_vk: Option<ZwpVirtualKeyboardV1>,
        initial_layout: &LayoutDefinition,
    ) -> Self {
        if let Some(vk) = wayland_vk {
            match WaylandVirtualKeyboardBackend::new(vk, initial_layout) {
                Ok(b) => {
                    info!("Selected input backend: {}", b.name());
                    return Self {
                        backend: Box::new(b),
                        im_backend: None,
                        uses_wayland_vk: true,
                    };
                }
                Err(err) => {
                    warn!("Wayland virtual keyboard setup failed ({err}); falling back to uinput");
                }
            }
        }

        match UInputBackend::open() {
            Ok(b) => {
                info!("Selected input backend: {}", b.name());
                Self {
                    backend: Box::new(b),
                    im_backend: None,
                    uses_wayland_vk: false,
                }
            }
            Err(err) => {
                warn!("Could not open /dev/uinput ({err}); running in read-only fallback mode");
                Self {
                    backend: Box::new(NullInputBackend::new(format!(
                        "Run setup-uinput.sh or grant /dev/uinput access: {err}"
                    ))),
                    im_backend: None,
                    uses_wayland_vk: false,
                }
            }
        }
    }

    /// Construct an InputManager prioritizing Wayland `zwp_input_method_v1` with optional uinput fallback.
    #[must_use]
    pub fn for_input_method() -> Self {
        let fallback: Option<Box<dyn InputBackend>> = UInputBackend::open()
            .ok()
            .map(|b| Box::new(b) as Box<dyn InputBackend>);
        let im = WaylandInputMethodBackend::new(fallback);
        info!("Initialized Wayland Input Method mode (Lockscreen / Plasma Virtual Keyboard)");
        Self {
            backend: Box::new(NullInputBackend::new("Input Method idle")),
            im_backend: Some(im),
            uses_wayland_vk: false,
        }
    }

    #[must_use]
    pub fn with_backend(backend: Box<dyn InputBackend>, uses_wayland_vk: bool) -> Self {
        Self {
            backend,
            im_backend: None,
            uses_wayland_vk,
        }
    }

    pub fn activate_input_method(&mut self, ctx: ZwpInputMethodContextV1) {
        if let Some(ref mut im) = self.im_backend {
            im.activate(ctx);
        }
    }

    pub fn deactivate_input_method(&mut self) {
        if let Some(ref mut im) = self.im_backend {
            im.deactivate();
        }
    }

    pub fn update_input_method_serial(&mut self, serial: u32) {
        if let Some(ref mut im) = self.im_backend {
            im.update_serial(serial);
        }
    }

    #[must_use]
    pub fn status_detail(&self) -> String {
        if let Some(ref im) = self.im_backend {
            im.status_detail()
        } else {
            self.backend.status_detail()
        }
    }

    #[must_use]
    pub fn uses_wayland_vk(&self) -> bool {
        self.uses_wayland_vk
    }

    pub fn execute(&mut self, commands: &[InputCommand]) {
        let target: &mut dyn InputBackend = if let Some(ref mut im) = self.im_backend {
            im
        } else {
            self.backend.as_mut()
        };

        for cmd in commands {
            let res = match cmd {
                InputCommand::Press(kc) => target.press_key(*kc),
                InputCommand::Release(kc) => target.release_key(*kc),
                InputCommand::Tap(kc) => target.tap_key(*kc),
                InputCommand::TypeText(text) => target.type_text(text),
                InputCommand::Shortcut { modifiers, keycode } => {
                    target.send_shortcut(modifiers, *keycode)
                }
                InputCommand::SyncModifiers { depressed, locked } => {
                    target.sync_modifiers(*depressed, *locked)
                }
                InputCommand::ReleaseAll => target.release_all(),
            };
            if let Err(err) = res {
                warn!("input backend command {cmd:?} failed: {err}");
            }
        }
    }

    pub fn apply_layout(&mut self, layout: &LayoutDefinition) {
        if let Err(err) = self.backend.apply_layout(layout) {
            warn!(
                "failed applying layout '{}' to input backend: {err}",
                layout.code
            );
        }
    }

    pub fn release_all(&mut self) {
        if let Some(ref mut im) = self.im_backend {
            let _ = im.release_all();
        }
        let _ = self.backend.release_all();
    }

    pub fn watchdog_tick(&mut self, active_held: &[KeyCode], now: Instant) -> usize {
        if let Some(ref mut im) = self.im_backend {
            im.watchdog_tick(active_held, now).unwrap_or(0)
        } else {
            self.backend.watchdog_tick(active_held, now).unwrap_or(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    struct MockBackend {
        pressed: HashMap<KeyCode, Instant>,
        events: Vec<(KeyCode, bool)>,
    }

    impl InputBackend for MockBackend {
        fn name(&self) -> &'static str {
            "mock"
        }

        fn press_key(&mut self, code: KeyCode) -> Result<(), InputError> {
            self.pressed
                .insert(code, Instant::now() - Duration::from_millis(300));
            self.events.push((code, true));
            Ok(())
        }

        fn release_key(&mut self, code: KeyCode) -> Result<(), InputError> {
            self.pressed.remove(&code);
            self.events.push((code, false));
            Ok(())
        }

        fn release_all(&mut self) -> Result<(), InputError> {
            self.pressed.clear();
            Ok(())
        }

        fn watchdog_tick(
            &mut self,
            active_held: &[KeyCode],
            now: Instant,
        ) -> Result<usize, InputError> {
            let stale: Vec<KeyCode> = self
                .pressed
                .iter()
                .filter(|(kc, since)| {
                    !active_held.contains(kc) && now.duration_since(**since) >= STUCK_KEY_TIMEOUT
                })
                .map(|(kc, _)| *kc)
                .collect();
            let count = stale.len();
            for kc in stale {
                self.release_key(kc)?;
            }
            Ok(count)
        }
    }

    #[test]
    fn watchdog_releases_orphaned_keys_but_preserves_actively_held_keys() {
        let mock = MockBackend {
            pressed: HashMap::new(),
            events: Vec::new(),
        };
        let mut mgr = InputManager::with_backend(Box::new(mock), false);
        mgr.execute(&[
            InputCommand::Press(KeyCode::A),
            InputCommand::Press(KeyCode::LEFTSHIFT),
        ]);

        let released = mgr.watchdog_tick(&[KeyCode::LEFTSHIFT], Instant::now());
        assert_eq!(released, 1);
    }
}
