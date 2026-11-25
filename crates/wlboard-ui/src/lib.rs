//! Unified `tiny-skia` + `cosmic-text` + `resvg` UI engine and Wayland/X11 surface runtimes for `wlboard`.

use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use tracing::{info, warn};

pub mod canvas;
pub mod clipboard;
pub mod icons;
pub mod keyboard_view;
pub mod panel_editor;
pub mod preview;
pub mod settings_view;
pub mod wayland;
pub mod x11_backend;

pub use canvas::UiRenderer;
pub use clipboard::ClipboardBridge;
pub use keyboard_view::{KeyboardHit, KeyboardViewLayout};
pub use panel_editor::{
    PANEL_EDITOR_DEFAULT_HEIGHT, PANEL_EDITOR_DEFAULT_WIDTH, PanelEditorAction, PanelEditorView,
};
pub use preview::{
    generate_preview_suite, render_keyboard_pixmap, render_panel_editor_pixmap,
    render_settings_pixmap,
};
pub use settings_view::{
    SETTINGS_DEFAULT_HEIGHT, SETTINGS_DEFAULT_WIDTH, SettingsAction, SettingsTab, SettingsView,
};
pub use wayland::{LaunchOptions, WaylandRunError, run_wayland_app};
pub use x11_backend::run_x11_app;

/// Launch `wlboard`, automatically choosing native Wayland (`zwlr_layer_shell_v1` /
/// `zwp_input_method_v1`) when supported, and seamlessly falling back to the pure-Rust
/// `x11rb` XWayland/X11 non-focus-stealing overlay on Ubuntu GNOME and X11 desktops.
pub fn run_app(
    options: LaunchOptions,
    shutdown_flag: Arc<AtomicBool>,
    signal_pipe: Option<UnixStream>,
) -> Result<(), String> {
    let has_wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let has_x11 = std::env::var_os("DISPLAY").is_some();

    if has_wayland || options.input_method_mode {
        let pipe_for_x11 = signal_pipe.as_ref().and_then(|s| s.try_clone().ok());
        match run_wayland_app(options.clone(), Arc::clone(&shutdown_flag), signal_pipe) {
            Ok(()) => return Ok(()),
            Err(WaylandRunError::LayerShellUnavailable)
                if !options.input_method_mode && has_x11 =>
            {
                info!(
                    "Wayland compositor does not advertise zwlr_layer_shell_v1 (e.g. GNOME/Mutter) — switching seamlessly to XWayland/X11 overlay backend"
                );
                return run_x11_app(options, shutdown_flag, pipe_for_x11);
            }
            Err(WaylandRunError::NoDisplay(wl_err)) if !options.input_method_mode && has_x11 => {
                warn!("{wl_err} — falling back to X11 backend");
                return run_x11_app(options, shutdown_flag, pipe_for_x11);
            }
            Err(err) => return Err(err.to_string()),
        }
    }

    if has_x11 {
        return run_x11_app(options, shutdown_flag, signal_pipe);
    }

    Err(
        "No supported display server found (neither WAYLAND_DISPLAY nor DISPLAY is set)."
            .to_owned(),
    )
}
