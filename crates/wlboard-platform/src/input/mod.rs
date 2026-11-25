//! Input injection abstraction with auto-selection (`zwp_virtual_keyboard_v1` -> `/dev/uinput` -> null)
//! and 250 ms stuck-key watchdog.

pub mod null;
pub mod uinput;
pub mod wayland_vk;
pub mod xkb_keymap;

use std::time::{Duration, Instant};

use thiserror::Error;
use tracing::{info, warn};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1;
use wlboard_core::{InputCommand, KeyCode, LayoutDefinition};

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

/// Unified key injection interface implemented by Wayland virtual keyboard, uinput, and null backends.
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
/// 1. `zwp_virtual_keyboard_v1` if advertised by the compositor (Hyprland, Sway, wlroots).
/// 2. Pure-Rust `/dev/uinput` fallback with 250 ms stuck-key watchdog (KDE Plasma Wayland).
/// 3. Safe read-only `NullInputBackend` reporting setup guidance in Settings.
pub struct InputManager {
    backend: Box<dyn InputBackend>,
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
                    uses_wayland_vk: false,
                }
            }
            Err(err) => {
                warn!("Could not open /dev/uinput ({err}); running in read-only fallback mode");
                Self {
                    backend: Box::new(NullInputBackend::new(format!(
                        "Run setup-uinput.sh or grant /dev/uinput access: {err}"
                    ))),
                    uses_wayland_vk: false,
                }
            }
        }
    }

    #[must_use]
    pub fn with_backend(backend: Box<dyn InputBackend>, uses_wayland_vk: bool) -> Self {
        Self {
            backend,
            uses_wayland_vk,
        }
    }

    #[must_use]
    pub fn status_detail(&self) -> String {
        self.backend.status_detail()
    }

    #[must_use]
    pub fn uses_wayland_vk(&self) -> bool {
        self.uses_wayland_vk
    }

    pub fn execute(&mut self, commands: &[InputCommand]) {
        for cmd in commands {
            let res = match *cmd {
                InputCommand::Press(kc) => self.backend.press_key(kc),
                InputCommand::Release(kc) => self.backend.release_key(kc),
                InputCommand::Tap(kc) => self.backend.tap_key(kc),
                InputCommand::SyncModifiers { depressed, locked } => {
                    self.backend.sync_modifiers(depressed, locked)
                }
                InputCommand::ReleaseAll => self.backend.release_all(),
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
        let _ = self.backend.release_all();
    }

    pub fn watchdog_tick(&mut self, active_held: &[KeyCode], now: Instant) -> usize {
        self.backend.watchdog_tick(active_held, now).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    struct MockBackend {
        pressed: HashMap<KeyCode, Instant>,
        released: Vec<KeyCode>,
    }

    impl InputBackend for MockBackend {
        fn name(&self) -> &'static str {
            "mock"
        }

        fn press_key(&mut self, code: KeyCode) -> Result<(), InputError> {
            self.pressed
                .insert(code, Instant::now() - Duration::from_millis(300));
            Ok(())
        }

        fn release_key(&mut self, code: KeyCode) -> Result<(), InputError> {
            self.pressed.remove(&code);
            self.released.push(code);
            Ok(())
        }

        fn release_all(&mut self) -> Result<(), InputError> {
            self.released.extend(self.pressed.keys().copied());
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
            released: Vec::new(),
        };
        let mut mgr = InputManager::with_backend(Box::new(mock), false);
        mgr.execute(&[
            InputCommand::Press(KeyCode::A),
            InputCommand::Press(KeyCode::LEFTSHIFT),
        ]);

        // Shift is actively held, A is orphaned for >250ms -> watchdog must release A only
        let released = mgr.watchdog_tick(&[KeyCode::LEFTSHIFT], Instant::now());
        assert_eq!(released, 1);
    }
}
