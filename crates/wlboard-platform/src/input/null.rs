//! Safe no-op fallback backend when neither Wayland virtual keyboard nor `/dev/uinput` is accessible.

use std::time::Instant;

use wlboard_core::KeyCode;

use crate::input::{InputBackend, InputError};

pub struct NullInputBackend {
    reason: String,
}

impl NullInputBackend {
    #[must_use]
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }

    #[must_use]
    pub fn reason(&self) -> &str {
        &self.reason
    }
}

impl InputBackend for NullInputBackend {
    fn name(&self) -> &'static str {
        "Read-only preview (no input backend available)"
    }

    fn status_detail(&self) -> String {
        format!("No active input backend ({})", self.reason)
    }

    fn press_key(&mut self, _code: KeyCode) -> Result<(), InputError> {
        Ok(())
    }

    fn release_key(&mut self, _code: KeyCode) -> Result<(), InputError> {
        Ok(())
    }

    fn release_all(&mut self) -> Result<(), InputError> {
        Ok(())
    }

    fn watchdog_tick(
        &mut self,
        _active_held: &[KeyCode],
        _now: Instant,
    ) -> Result<usize, InputError> {
        Ok(0)
    }
}
