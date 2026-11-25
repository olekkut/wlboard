//! Pure-Rust Linux `/dev/uinput` backend with 250 ms stuck-key watchdog and release-all safety.

use std::collections::HashMap;
use std::time::Instant;

use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, EventType, InputEvent, KeyCode as EvdevKeyCode};
use tracing::{debug, warn};
use wlboard_core::KeyCode;

use crate::input::{InputBackend, InputError, STUCK_KEY_TIMEOUT};

pub struct UInputBackend {
    device: VirtualDevice,
    pressed: HashMap<KeyCode, Instant>,
}

impl UInputBackend {
    /// Open `/dev/uinput` and create a virtual keyboard supporting all standard, nav, numpad, and media keys.
    pub fn open() -> Result<Self, InputError> {
        let mut keys = AttributeSet::<EvdevKeyCode>::new();
        for kc in KeyCode::all_supported() {
            keys.insert(EvdevKeyCode::new(kc.raw()));
        }
        let device = VirtualDevice::builder()?
            .name("wlboard Virtual Keyboard")
            .with_keys(&keys)?
            .build()?;

        Ok(Self {
            device,
            pressed: HashMap::new(),
        })
    }

    fn emit_key(&mut self, code: KeyCode, value: i32) -> Result<(), InputError> {
        let event = InputEvent::new(EventType::KEY.0, code.raw(), value);
        self.device.emit(&[event])?;
        Ok(())
    }
}

impl InputBackend for UInputBackend {
    fn name(&self) -> &'static str {
        "Linux uinput (/dev/uinput) + 250 ms watchdog"
    }

    fn press_key(&mut self, code: KeyCode) -> Result<(), InputError> {
        self.emit_key(code, 1)?;
        self.pressed.insert(code, Instant::now());
        Ok(())
    }

    fn release_key(&mut self, code: KeyCode) -> Result<(), InputError> {
        self.emit_key(code, 0)?;
        self.pressed.remove(&code);
        Ok(())
    }

    fn release_all(&mut self) -> Result<(), InputError> {
        let keys: Vec<KeyCode> = self.pressed.keys().copied().collect();
        for kc in keys {
            if let Err(err) = self.emit_key(kc, 0) {
                warn!("failed releasing key {:?} on uinput: {err}", kc);
            }
        }
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
            debug!("uinput watchdog releasing orphaned key {:?}", kc);
            self.release_key(kc)?;
        }
        Ok(count)
    }
}

impl Drop for UInputBackend {
    fn drop(&mut self) {
        let _ = self.release_all();
    }
}
