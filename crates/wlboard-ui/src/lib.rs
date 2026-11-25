//! Unified `tiny-skia` + `cosmic-text` + `resvg` UI engine and Wayland surface runtime for `wlboard`.

pub mod canvas;
pub mod icons;
pub mod keyboard_view;
pub mod preview;
pub mod settings_view;
pub mod wayland;

pub use canvas::UiRenderer;
pub use keyboard_view::{KeyboardHit, KeyboardViewLayout};
pub use preview::{generate_preview_suite, render_keyboard_pixmap, render_settings_pixmap};
pub use settings_view::{
    SETTINGS_DEFAULT_HEIGHT, SETTINGS_DEFAULT_WIDTH, SettingsAction, SettingsTab, SettingsView,
};
pub use wayland::run_wayland_app;
