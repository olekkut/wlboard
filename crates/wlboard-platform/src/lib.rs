//! OS and desktop integration adapters for `wlboard`:
//! - Auto-selected Wayland `zwp_input_method_v1` / `zwp_virtual_keyboard_v1` / pure-Rust `/dev/uinput` input backends
//! - KDE Plasma two-way keyboard layout sync + auto-registration of missing layouts in `kxkbrc`
//! - Native StatusNotifierItem system tray + D-Bus menu
//! - Single-instance IPC service (`io.github.olekkut.wlboard`)

pub mod input;
pub mod ipc;
pub mod layout_sync;
pub mod tray;

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use tracing::{debug, info, warn};
use wlboard_core::{KeyboardStyle, LayoutRegistry};
use zbus::connection::Builder as ConnectionBuilder;

pub use input::{
    InputBackend, InputError, InputManager, STUCK_KEY_TIMEOUT, WaylandInputMethodBackend,
};
pub use ipc::{APP_BUS_NAME, APP_OBJECT_PATH, IpcCommand, try_send_ipc_command};
pub use layout_sync::{PlasmaLayoutSync, ensure_layout_in_kxkbrc};
pub use tray::{EventCallback, TrayState};

/// Events pushed from the background D-Bus platform worker into the Wayland UI event loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlatformEvent {
    ToggleVisible,
    Show,
    Hide,
    OpenSettings,
    OpenPanelEditor,
    ToggleCompactStyle,
    SetLayout(String),
    SetPanel(String),
    ExternalLayoutChanged(String),
    LayoutSyncStatus(String),
    Quit,
}

/// Commands sent from the Wayland UI loop to the background D-Bus platform worker.
#[derive(Debug, Clone)]
pub enum PlatformCommand {
    UpdateTray {
        visible: bool,
        style: KeyboardStyle,
        active_layout: String,
        active_panel: String,
        panels: Vec<(String, String)>,
    },
    SyncSystemLayout {
        code: String,
        auto_register: bool,
    },
    Shutdown,
}

/// Handle to the running D-Bus background service (IPC + Tray + Plasma Layout Sync).
pub struct PlatformHandle {
    cmd_tx: Sender<PlatformCommand>,
}

impl PlatformHandle {
    /// Start the session D-Bus worker thread (IPC + Tray + KDE Plasma layout sync).
    pub fn spawn<F>(
        initial_tray: TrayState,
        registry: LayoutRegistry,
        register_tray_and_ipc: bool,
        on_event: F,
    ) -> Self
    where
        F: Fn(PlatformEvent) + Send + Sync + 'static,
    {
        let (cmd_tx, cmd_rx) = mpsc::channel::<PlatformCommand>();
        let callback: EventCallback = Arc::new(on_event);
        let tray_state = Arc::new(Mutex::new(initial_tray));

        let _ = thread::Builder::new()
            .name("wlboard-dbus".to_owned())
            .spawn(move || {
                run_dbus_worker(
                    tray_state,
                    registry,
                    register_tray_and_ipc,
                    callback,
                    cmd_rx,
                );
            });

        Self { cmd_tx }
    }

    pub fn update_tray(
        &self,
        visible: bool,
        style: KeyboardStyle,
        active_layout: String,
        active_panel: String,
        panels: Vec<(String, String)>,
    ) {
        let _ = self.cmd_tx.send(PlatformCommand::UpdateTray {
            visible,
            style,
            active_layout,
            active_panel,
            panels,
        });
    }

    pub fn sync_system_layout(&self, code: String, auto_register: bool) {
        let _ = self.cmd_tx.send(PlatformCommand::SyncSystemLayout {
            code,
            auto_register,
        });
    }

    pub fn shutdown(&self) {
        let _ = self.cmd_tx.send(PlatformCommand::Shutdown);
    }
}

fn run_dbus_worker(
    tray_state: Arc<Mutex<TrayState>>,
    registry: LayoutRegistry,
    register_tray_and_ipc: bool,
    on_event: EventCallback,
    cmd_rx: Receiver<PlatformCommand>,
) {
    futures_lite::future::block_on(async move {
        let conn = if register_tray_and_ipc {
            let ipc_service = ipc::WlboardIpcService::new(Arc::clone(&on_event));
            let sni = tray::StatusNotifierItem::new(Arc::clone(&tray_state), Arc::clone(&on_event));
            let menu = tray::DbusMenu::new(Arc::clone(&tray_state), Arc::clone(&on_event));

            match ConnectionBuilder::session()
                .name(APP_BUS_NAME)
                .serve_at(APP_OBJECT_PATH, ipc_service)
                .serve_at(tray::SNI_OBJECT_PATH, sni)
                .serve_at(tray::MENU_OBJECT_PATH, menu)
                .build()
                .await
            {
                Ok(c) => {
                    info!("D-Bus session services registered under {APP_BUS_NAME}");
                    tray::register_with_watcher(&c).await;
                    c
                }
                Err(err) => {
                    warn!(
                        "Session D-Bus unavailable ({err}); running without tray/IPC/layout sync"
                    );
                    return;
                }
            }
        } else {
            // In --input-method mode (e.g. Lockscreen / SDDM), avoid colliding with a running desktop tray instance.
            match zbus::Connection::session().await {
                Ok(c) => c,
                Err(_) => return,
            }
        };

        let (mut plasma, initial_code) =
            layout_sync::PlasmaLayoutSync::connect(&conn, &registry).await;
        on_event(PlatformEvent::LayoutSyncStatus(plasma.status_text()));
        if let Some(code) = initial_code {
            on_event(PlatformEvent::ExternalLayoutChanged(code));
        }

        if plasma.is_connected() {
            let conn_watch = conn.clone();
            let reg_watch = registry.clone();
            let sync_watch = plasma.clone();
            let cb_watch = Arc::clone(&on_event);
            let _ = thread::Builder::new()
                .name("wlboard-plasma-sync".to_owned())
                .spawn(move || {
                    futures_lite::future::block_on(layout_sync::watch_plasma_layouts(
                        conn_watch,
                        reg_watch,
                        sync_watch,
                        move |code| cb_watch(PlatformEvent::ExternalLayoutChanged(code)),
                    ));
                });
        }

        loop {
            match cmd_rx.recv_timeout(Duration::from_millis(100)) {
                Ok(PlatformCommand::UpdateTray {
                    visible,
                    style,
                    active_layout,
                    active_panel,
                    panels,
                }) => {
                    if let Ok(mut guard) = tray_state.lock() {
                        guard.visible = visible;
                        guard.style = style;
                        guard.active_layout = active_layout;
                        guard.active_panel = active_panel;
                        guard.panels = panels;
                        guard.revision = guard.revision.wrapping_add(1);
                    }
                }
                Ok(PlatformCommand::SyncSystemLayout {
                    code,
                    auto_register,
                }) => {
                    let ok = plasma
                        .set_system_layout(&conn, &registry, &code, auto_register)
                        .await;
                    debug!("SyncSystemLayout({code}, auto_register={auto_register}) -> {ok}");
                }
                Ok(PlatformCommand::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    break;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    });
}
