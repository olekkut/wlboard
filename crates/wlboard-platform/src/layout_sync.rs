//! Two-way KDE Plasma keyboard layout synchronization over D-Bus (`org.kde.keyboard /Layouts`)
//! with automatic registration of missing XKB layouts in `~/.config/kxkbrc`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

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

/// D-Bus proxy for KWin reconfigure (reloads `~/.config/kxkbrc` immediately after adding a layout).
#[proxy(
    interface = "org.kde.KWin",
    default_service = "org.kde.KWin",
    default_path = "/KWin"
)]
trait KWinReconfigure {
    fn reconfigure(&self) -> zbus::Result<()>;
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

    /// Push a layout change from `wlboard` back to KDE Plasma (`setLayout`), optionally adding
    /// the missing XKB layout into `~/.config/kxkbrc` and reconfiguring KWin so typing works immediately!
    pub async fn set_system_layout(
        &mut self,
        conn: &Connection,
        registry: &LayoutRegistry,
        code: &str,
        auto_register_missing: bool,
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

        self.refresh_list(&proxy, registry).await;

        if let Some(idx) = self.layouts.iter().position(|c| c == &target) {
            return match proxy.set_layout(idx as u32).await {
                Ok(ok) => ok,
                Err(err) => {
                    warn!("Plasma setLayout({idx}) failed: {err}");
                    false
                }
            };
        }

        // Layout not yet added in KDE System Settings: auto-register it cleanly when enabled!
        if !auto_register_missing {
            debug!("Layout '{target}' not present in KDE system layouts (auto-register disabled)");
            return false;
        }

        let xkb_name = registry.get(&target).xkb_name.as_str();
        if let Some(kxkbrc) = default_kxkbrc_path() {
            match ensure_layout_in_kxkbrc(&kxkbrc, xkb_name) {
                Ok(true) => {
                    info!(
                        "Auto-added XKB layout '{xkb_name}' to {} — reloading KDE Keyboard",
                        kxkbrc.display()
                    );
                    reload_kde_keyboard_config(conn).await;
                    // Give KWin a brief moment to reload the updated layout table before selecting it.
                    std::thread::sleep(std::time::Duration::from_millis(120));
                    self.refresh_list(&proxy, registry).await;
                    if let Some(idx) = self.layouts.iter().position(|c| c == &target) {
                        return proxy.set_layout(idx as u32).await.unwrap_or(false);
                    }
                }
                Ok(false) => {}
                Err(err) => warn!("Failed updating kxkbrc for '{xkb_name}': {err}"),
            }
        }
        false
    }

    async fn refresh_list(
        &mut self,
        proxy: &PlasmaKeyboardLayoutsProxy<'_>,
        registry: &LayoutRegistry,
    ) {
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
        if let Ok(proxy) = PlasmaKeyboardLayoutsProxy::new(conn).await {
            self.refresh_list(&proxy, registry).await;
            return self
                .layouts
                .get(index as usize)
                .and_then(|c| registry.normalize_code(c));
        }
        None
    }
}

fn default_kxkbrc_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("kxkbrc"))
}

/// Safely add `xkb_code` (e.g. `"us"`, `"pl"`, `"ua"`, `"ru"`, `"ara"`) to `[Layout] LayoutList=`
/// inside KDE's `kxkbrc` while preserving every unrelated line and setting.
pub fn ensure_layout_in_kxkbrc(path: &Path, xkb_code: &str) -> std::io::Result<bool> {
    let content = if path.exists() {
        fs::read_to_string(path)?
    } else {
        String::new()
    };

    let mut lines: Vec<String> = content.lines().map(ToOwned::to_owned).collect();
    let mut in_layout = false;
    let mut found_section = false;
    let mut found_list = false;
    let mut found_use = false;
    let mut changed = false;
    let mut section_end_idx = lines.len();

    for (idx, line) in lines.iter_mut().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if in_layout {
                section_end_idx = idx;
                in_layout = false;
            }
            if trimmed == "[Layout]" {
                in_layout = true;
                found_section = true;
            }
            continue;
        }
        if !in_layout {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("LayoutList=") {
            found_list = true;
            let mut items: Vec<String> = rest
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToOwned::to_owned)
                .collect();
            if !items.iter().any(|item| item.eq_ignore_ascii_case(xkb_code)) {
                items.push(xkb_code.to_owned());
                *line = format!("LayoutList={}", items.join(","));
                changed = true;
            }
        } else if trimmed.starts_with("Use=") {
            found_use = true;
            if trimmed != "Use=true" {
                *line = "Use=true".to_owned();
                changed = true;
            }
        }
    }

    if !found_section {
        if !lines.is_empty() && !lines.last().is_some_and(String::is_empty) {
            lines.push(String::new());
        }
        lines.push("[Layout]".to_owned());
        lines.push(format!("LayoutList=us,{xkb_code}"));
        lines.push("Use=true".to_owned());
        changed = true;
    } else {
        let mut insert_pos = section_end_idx;
        if !found_list {
            lines.insert(insert_pos, format!("LayoutList={xkb_code}"));
            insert_pos += 1;
            changed = true;
        }
        if !found_use {
            lines.insert(insert_pos, "Use=true".to_owned());
            changed = true;
        }
    }

    if changed {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut out = lines.join("\n");
        out.push('\n');
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, out)?;
        fs::rename(&tmp, path)?;
    }

    Ok(changed)
}

async fn reload_kde_keyboard_config(conn: &Connection) {
    // 1. Trigger KWin D-Bus reconfigure (reloads kxkbrc on Plasma 6 Wayland)
    if let Ok(kwin) = KWinReconfigureProxy::new(conn).await {
        let _ = kwin.reconfigure().await;
    }
    // 2. Also emit KDE's keyboard config-changed D-Bus signal if qdbus6/dbus-send is present
    let _ = Command::new("dbus-send")
        .args([
            "--session",
            "--type=signal",
            "/Layouts",
            "org.kde.keyboard.reloadConfig",
        ])
        .status();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_missing_layout_to_kxkbrc_preserving_existing_entries() {
        let dir = std::env::temp_dir().join(format!("wlboard_kxkb_test_{}", std::process::id()));
        let path = dir.join("kxkbrc");
        fs::create_dir_all(&dir).expect("mkdir");
        fs::write(
            &path,
            "[Layout]\nDisplayNames=,\nLayoutList=ua,pl\nUse=true\nVariantList=,\n",
        )
        .expect("write initial kxkbrc");

        assert!(ensure_layout_in_kxkbrc(&path, "us").expect("add us"));
        // Calling a second time must be a no-op
        assert!(!ensure_layout_in_kxkbrc(&path, "us").expect("idempotent"));

        let updated = fs::read_to_string(&path).expect("read updated");
        assert!(updated.contains("LayoutList=ua,pl,us"));
        assert!(updated.contains("VariantList=,"));
        let _ = fs::remove_dir_all(&dir);
    }
}
