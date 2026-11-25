//! Two-way KDE Plasma keyboard layout synchronization over D-Bus (`org.kde.keyboard /Layouts`).

use futures_lite::StreamExt;
use tracing::{debug, info, warn};
use wlboard_core::LayoutRegistry;
use zbus::{Connection, proxy};

/// D-Bus proxy for KDE Plasma's `org.kde.keyboard` layout daemon.
#[proxy(
    interface = "org.kde.KeyboardLayouts",
    default_service = "org.kde.keyboard",
    default_path = "/Layouts"
)]
pub trait PlasmaKeyboardLayouts {
    #[zbus(name = "getLayout")]
    fn get_layout(&self) -> zbus::Result<u32>;

    #[zbus(name = "getLayoutsList")]
    fn get_layouts_list(&self) -> zbus::Result<Vec<(String, String, String)>>;

    #[zbus(name = "setLayout")]
    fn set_layout(&self, index: u32) -> zbus::Result<bool>;

    #[zbus(signal, name = "layoutChanged")]
    fn layout_changed(&self, index: u32) -> zbus::Result<()>;

    #[zbus(signal, name = "layoutListChanged")]
    fn layout_list_changed(&self) -> zbus::Result<()>;
}

/// Snapshot of system keyboard layouts exposed by KDE Plasma.
#[derive(Debug, Clone, Default)]
pub struct PlasmaLayoutSync {
    layouts: Vec<String>,
    connected: bool,
}

impl PlasmaLayoutSync {
    /// Attempt to connect to KDE Plasma's `org.kde.keyboard` service and query the current layout.
    pub async fn connect(conn: &Connection, registry: &LayoutRegistry) -> (Self, Option<String>) {
        let Ok(proxy) = PlasmaKeyboardLayoutsProxy::new(conn).await else {
            return (Self::default(), None);
        };

        let Ok(list) = proxy.get_layouts_list().await else {
            debug!("org.kde.keyboard not active on this session (non-Plasma compositor)");
            return (Self::default(), None);
        };

        let normalized: Vec<String> = list
            .iter()
            .map(|(short, _, _)| {
                registry
                    .normalize_code(short)
                    .unwrap_or_else(|| short.to_lowercase())
            })
            .collect();

        let current_code = proxy
            .get_layout()
            .await
            .ok()
            .and_then(|idx| normalized.get(idx as usize).cloned())
            .and_then(|code| registry.normalize_code(&code));

        info!(
            "KDE Plasma layout sync connected ({} system layouts, active={:?})",
            normalized.len(),
            current_code
        );

        (
            Self {
                layouts: normalized,
                connected: true,
            },
            current_code,
        )
    }

    #[must_use]
    pub const fn is_connected(&self) -> bool {
        self.connected
    }

    #[must_use]
    pub fn status_text(&self) -> String {
        if self.connected {
            format!(
                "KDE Plasma D-Bus Connected ({} layouts)",
                self.layouts.len()
            )
        } else {
            "Local / Virtual Keyboard Layout Switching".to_owned()
        }
    }

    /// Push a layout change from `wlboard` back to KDE Plasma (`setLayout`).
    pub async fn set_system_layout(
        &mut self,
        conn: &Connection,
        registry: &LayoutRegistry,
        code: &str,
    ) -> bool {
        if !self.connected {
            return false;
        }
        let Some(target) = registry.normalize_code(code) else {
            return false;
        };
        let Ok(proxy) = PlasmaKeyboardLayoutsProxy::new(conn).await else {
            return false;
        };

        // Refresh layout list in case user reordered KDE Keyboard settings.
        if let Ok(list) = proxy.get_layouts_list().await {
            self.layouts = list
                .iter()
                .map(|(short, _, _)| {
                    registry
                        .normalize_code(short)
                        .unwrap_or_else(|| short.to_lowercase())
                })
                .collect();
        }

        if let Some(idx) = self.layouts.iter().position(|c| c == &target) {
            match proxy.set_layout(idx as u32).await {
                Ok(ok) => return ok,
                Err(err) => warn!("Plasma setLayout({idx}) failed: {err}"),
            }
        }
        false
    }

    /// Resolve a `layoutChanged(index)` signal into a normalized `wlboard` layout code.
    pub async fn resolve_index(
        &mut self,
        conn: &Connection,
        registry: &LayoutRegistry,
        index: u32,
    ) -> Option<String> {
        if let Some(code) = self.layouts.get(index as usize)
            && let Some(norm) = registry.normalize_code(code)
        {
            return Some(norm);
        }
        if let Ok(proxy) = PlasmaKeyboardLayoutsProxy::new(conn).await
            && let Ok(list) = proxy.get_layouts_list().await
        {
            self.layouts = list
                .iter()
                .map(|(short, _, _)| {
                    registry
                        .normalize_code(short)
                        .unwrap_or_else(|| short.to_lowercase())
                })
                .collect();
            return self
                .layouts
                .get(index as usize)
                .and_then(|c| registry.normalize_code(c));
        }
        None
    }
}

/// Subscribe to KDE Plasma `layoutChanged` signals and forward normalized codes via callback.
pub async fn watch_plasma_layouts<F>(
    conn: Connection,
    registry: LayoutRegistry,
    mut sync: PlasmaLayoutSync,
    mut on_change: F,
) where
    F: FnMut(String) + Send + 'static,
{
    let Ok(proxy) = PlasmaKeyboardLayoutsProxy::new(&conn).await else {
        return;
    };
    let Ok(mut stream) = proxy.receive_layout_changed().await else {
        return;
    };

    while let Some(signal) = stream.next().await {
        if let Ok(args) = signal.args()
            && let Some(code) = sync.resolve_index(&conn, &registry, args.index).await
        {
            debug!("KDE Plasma switched layout -> {code}");
            on_change(code);
        }
    }
}
