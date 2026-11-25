//! Native Wayland `zwp_virtual_keyboard_v1` input backend (Hyprland, Sway, wlroots compositors).

use std::collections::HashMap;
use std::ffi::CString;
use std::io::Write;
use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use std::time::Instant;

use wayland_client::protocol::wl_keyboard::KeymapFormat;
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1;
use wlboard_core::{KeyCode, LayoutDefinition};

use crate::input::xkb_keymap::build_xkb_keymap;
use crate::input::{InputBackend, InputError, STUCK_KEY_TIMEOUT};

pub struct WaylandVirtualKeyboardBackend {
    vk: ZwpVirtualKeyboardV1,
    started_at: Instant,
    pressed: HashMap<KeyCode, Instant>,
    depressed: u32,
    locked: u32,
    active_layout: String,
}

impl WaylandVirtualKeyboardBackend {
    /// Wrap a newly created `zwp_virtual_keyboard_v1` and upload the initial layout's XKB keymap.
    pub fn new(
        vk: ZwpVirtualKeyboardV1,
        initial_layout: &LayoutDefinition,
    ) -> Result<Self, InputError> {
        let mut backend = Self {
            vk,
            started_at: Instant::now(),
            pressed: HashMap::new(),
            depressed: 0,
            locked: 0,
            active_layout: String::new(),
        };
        backend.apply_layout(initial_layout)?;
        Ok(backend)
    }

    fn timestamp_ms(&self) -> u32 {
        self.started_at.elapsed().as_millis() as u32
    }

    fn upload_keymap(&self, keymap_text: &str) -> Result<(), InputError> {
        let mut bytes = keymap_text.as_bytes().to_vec();
        bytes.push(0);

        let name = CString::new("wlboard-keymap").expect("static name has no NUL");
        // SAFETY: memfd_create is a standard Linux syscall; return value >= 0 is a valid owned FD.
        let raw_fd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) };
        if raw_fd < 0 {
            return Err(InputError::Io(std::io::Error::last_os_error()));
        }
        // SAFETY: raw_fd was just returned by memfd_create and is owned exclusively here.
        let fd = unsafe { OwnedFd::from_raw_fd(raw_fd) };
        let mut file = std::fs::File::from(fd);
        file.write_all(&bytes)?;
        file.flush()?;

        self.vk
            .keymap(KeymapFormat::XkbV1.into(), file.as_fd(), bytes.len() as u32);
        Ok(())
    }
}

impl InputBackend for WaylandVirtualKeyboardBackend {
    fn name(&self) -> &'static str {
        "Wayland Virtual Keyboard (zwp_virtual_keyboard_v1)"
    }

    fn press_key(&mut self, code: KeyCode) -> Result<(), InputError> {
        let ts = self.timestamp_ms();
        self.vk.key(ts, u32::from(code.raw()), 1);
        self.pressed.insert(code, Instant::now());
        Ok(())
    }

    fn release_key(&mut self, code: KeyCode) -> Result<(), InputError> {
        let ts = self.timestamp_ms();
        self.vk.key(ts, u32::from(code.raw()), 0);
        self.pressed.remove(&code);
        Ok(())
    }

    fn sync_modifiers(&mut self, depressed: u32, locked: u32) -> Result<(), InputError> {
        if self.depressed != depressed || self.locked != locked {
            self.depressed = depressed;
            self.locked = locked;
            self.vk.modifiers(depressed, 0, locked, 0);
        }
        Ok(())
    }

    fn apply_layout(&mut self, layout: &LayoutDefinition) -> Result<(), InputError> {
        if self.active_layout == layout.code {
            return Ok(());
        }
        let keymap = build_xkb_keymap(layout);
        self.upload_keymap(&keymap)?;
        self.active_layout = layout.code.clone();
        Ok(())
    }

    fn release_all(&mut self) -> Result<(), InputError> {
        let ts = self.timestamp_ms();
        let keys: Vec<KeyCode> = self.pressed.keys().copied().collect();
        for code in keys {
            self.vk.key(ts, u32::from(code.raw()), 0);
        }
        self.pressed.clear();
        if self.depressed != 0 || self.locked != 0 {
            self.depressed = 0;
            self.locked = 0;
            self.vk.modifiers(0, 0, 0, 0);
        }
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

impl Drop for WaylandVirtualKeyboardBackend {
    fn drop(&mut self) {
        let _ = self.release_all();
        self.vk.destroy();
    }
}
