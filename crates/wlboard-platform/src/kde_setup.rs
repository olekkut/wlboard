//! KDE Plasma Lockscreen & SDDM configuration helpers for `wlboard`.
//!
//! Configures KWin's `[Wayland] InputMethod` in `~/.config/kwinrc` and sets
//! `KWIN_IM_SHOW_ALWAYS=1` in `~/.config/environment.d/70-wlboard-kwin.conf` so the
//! Virtual Keyboard button works on the KDE Plasma Lockscreen even on desktops/VMs
//! without a physical touchscreen.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const IM_DESKTOP_PATH: &str = "/usr/share/applications/io.github.olekkut.wlboard-im.desktop";
const ENV_CONF_FILENAME: &str = "70-wlboard-kwin.conf";
const SDDM_CONF_PATH: &str = "/etc/sddm.conf.d/70-wlboard.conf";

/// Detected KDE Plasma Lockscreen & SDDM integration status.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KdeLockscreenStatus {
    pub kwin_im_configured: bool,
    pub kwin_show_always: bool,
    pub sddm_configured: bool,
}

impl KdeLockscreenStatus {
    #[must_use]
    pub fn summary(&self) -> String {
        match (
            self.kwin_im_configured,
            self.kwin_show_always,
            self.sddm_configured,
        ) {
            (true, true, true) => "Lockscreen + Mouse + SDDM Ready".to_owned(),
            (true, true, false) => "Lockscreen Ready (SDDM script optional)".to_owned(),
            (true, false, _) => {
                "Lockscreen Registered (Touch only — click Enable for Mouse)".to_owned()
            }
            (false, _, true) => "SDDM Ready (click Enable for KDE Lockscreen)".to_owned(),
            (false, _, false) => "Not enabled yet — click Enable KDE Lockscreen".to_owned(),
        }
    }
}

fn config_base_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
}

fn kwinrc_path() -> Option<PathBuf> {
    Some(config_base_dir()?.join("kwinrc"))
}

fn env_d_path() -> Option<PathBuf> {
    Some(
        config_base_dir()?
            .join("environment.d")
            .join(ENV_CONF_FILENAME),
    )
}

/// Inspect current KDE Plasma Lockscreen & SDDM configuration without modifying files.
#[must_use]
pub fn detect_kde_lockscreen_status() -> KdeLockscreenStatus {
    let kwin_im_configured = kwinrc_path()
        .and_then(|p| fs::read_to_string(p).ok())
        .is_some_and(|txt| txt.contains("wlboard-im.desktop"));

    let kwin_show_always = std::env::var("KWIN_IM_SHOW_ALWAYS")
        .map(|v| v == "1")
        .unwrap_or(false)
        || env_d_path()
            .and_then(|p| fs::read_to_string(p).ok())
            .is_some_and(|txt| txt.contains("KWIN_IM_SHOW_ALWAYS=1"))
        || fs::read_to_string("/etc/environment")
            .ok()
            .is_some_and(|txt| txt.contains("KWIN_IM_SHOW_ALWAYS=1"));

    let sddm_configured = Path::new(SDDM_CONF_PATH).exists();

    KdeLockscreenStatus {
        kwin_im_configured,
        kwin_show_always,
        sddm_configured,
    }
}

/// Enable `wlboard Virtual Keyboard` in `~/.config/kwinrc` and write
/// `~/.config/environment.d/70-wlboard-kwin.conf` (`KWIN_IM_SHOW_ALWAYS=1`) so
/// clicking the Lockscreen Virtual Keyboard button works with both touchscreens and mouse!
pub fn enable_kde_lockscreen_support() -> Result<String, String> {
    let kwinrc = kwinrc_path().ok_or_else(|| "Could not locate ~/.config directory".to_owned())?;
    let env_conf =
        env_d_path().ok_or_else(|| "Could not locate ~/.config/environment.d".to_owned())?;

    set_kwin_input_method_in_file(&kwinrc, IM_DESKTOP_PATH)
        .map_err(|e| format!("Failed updating {}: {e}", kwinrc.display()))?;

    if let Some(parent) = env_conf.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed creating {}: {e}", parent.display()))?;
    }
    fs::write(
        &env_conf,
        "# Managed by wlboard: allows KDE Plasma Lockscreen Virtual Keyboard with mouse/keyboard.\nKWIN_IM_SHOW_ALWAYS=1\n",
    )
    .map_err(|e| format!("Failed writing {}: {e}", env_conf.display()))?;

    // Best-effort KDE cache & KWin reconfigure so KWin loads wlboard-im immediately
    let _ = Command::new("kbuildsycoca6").status();
    let _ = Command::new("dbus-send")
        .args([
            "--session",
            "--type=method_call",
            "--dest=org.kde.KWin",
            "/KWin",
            "org.kde.KWin.reconfigure",
        ])
        .status();

    Ok("Enabled! Log out & back in once for mouse Lockscreen support.".to_owned())
}

/// Safely disable KDE Plasma Lockscreen virtual keyboard integration for `wlboard`
/// without affecting any other KWin or Plasma settings.
pub fn disable_kde_lockscreen_support() -> Result<String, String> {
    if let Some(kwinrc) = kwinrc_path()
        && kwinrc.exists()
    {
        remove_kwin_input_method_in_file(&kwinrc)
            .map_err(|e| format!("Failed updating {}: {e}", kwinrc.display()))?;
    }
    if let Some(env_conf) = env_d_path()
        && env_conf.exists()
    {
        fs::remove_file(&env_conf)
            .map_err(|e| format!("Failed removing {}: {e}", env_conf.display()))?;
    }

    let _ = Command::new("kbuildsycoca6").status();
    let _ = Command::new("dbus-send")
        .args([
            "--session",
            "--type=method_call",
            "--dest=org.kde.KWin",
            "/KWin",
            "org.kde.KWin.reconfigure",
        ])
        .status();

    Ok("Disabled KDE Lockscreen integration (system restored cleanly).".to_owned())
}

/// Remove the `KWIN_IM_SHOW_ALWAYS=1` override file if the user wants touch-only IM activation.
pub fn disable_kde_lockscreen_mouse_override() -> Result<String, String> {
    let env_conf =
        env_d_path().ok_or_else(|| "Could not locate ~/.config/environment.d".to_owned())?;
    if env_conf.exists() {
        fs::remove_file(&env_conf)
            .map_err(|e| format!("Failed removing {}: {e}", env_conf.display()))?;
    }
    Ok("Removed KWIN_IM_SHOW_ALWAYS override (log out & back in to apply).".to_owned())
}

fn find_sddm_script() -> Option<PathBuf> {
    let installed = PathBuf::from("/usr/share/wlboard/scripts/setup-sddm.sh");
    if installed.exists() {
        return Some(installed);
    }
    let local = PathBuf::from("packaging/deb/setup-sddm.sh");
    if local.exists() {
        return Some(local);
    }
    None
}

/// Enable `wlboard` on the SDDM Wayland login screen via standard GUI Polkit (`pkexec`) prompt.
pub fn enable_sddm_support_gui() -> Result<String, String> {
    let Some(script) = find_sddm_script() else {
        return Err("setup-sddm.sh not found (install the wlboard .deb package first)".to_owned());
    };
    let output = Command::new("pkexec")
        .arg(&script)
        .output()
        .map_err(|e| format!("Could not launch pkexec admin prompt: {e}"))?;
    if output.status.success() {
        Ok("SDDM Login Screen enabled! Reboot or log out to test.".to_owned())
    } else if output.status.code() == Some(126) || output.status.code() == Some(127) {
        Err("Admin authentication canceled.".to_owned())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        Err(if stderr.is_empty() {
            "SDDM setup script returned an error.".to_owned()
        } else {
            format!("SDDM setup failed: {stderr}")
        })
    }
}

/// Safely disable `wlboard` on the SDDM Wayland login screen via GUI Polkit (`pkexec`) prompt.
pub fn disable_sddm_support_gui() -> Result<String, String> {
    let Some(script) = find_sddm_script() else {
        return Err("setup-sddm.sh not found.".to_owned());
    };
    let output = Command::new("pkexec")
        .arg(&script)
        .arg("--remove")
        .output()
        .map_err(|e| format!("Could not launch pkexec admin prompt: {e}"))?;
    if output.status.success() {
        Ok("SDDM Login Screen config removed cleanly.".to_owned())
    } else if output.status.code() == Some(126) || output.status.code() == Some(127) {
        Err("Admin authentication canceled.".to_owned())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        Err(if stderr.is_empty() {
            "Failed removing SDDM config.".to_owned()
        } else {
            format!("SDDM removal failed: {stderr}")
        })
    }
}

/// Returns `true` when running inside the SDDM login greeter OR when the desktop
/// lockscreen (`org.freedesktop.ScreenSaver`) is currently active.
///
/// Used by `wlboard --input-method` so it NEVER duplicates over the floating
/// desktop overlay while the user is working on an unlocked desktop.
#[must_use]
pub fn is_lockscreen_or_greeter_active() -> bool {
    if std::env::var("XDG_SESSION_CLASS")
        .map(|v| v.eq_ignore_ascii_case("greeter"))
        .unwrap_or(false)
    {
        return true;
    }
    if std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .map(|u| u == "sddm" || u == "gdm" || u == "greeter")
        .unwrap_or(false)
    {
        return true;
    }
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
        return true;
    }

    futures_lite::future::block_on(async {
        let Ok(conn) = zbus::Connection::session().await else {
            return true;
        };
        let Ok(proxy) = zbus::Proxy::new(
            &conn,
            "org.freedesktop.ScreenSaver",
            "/ScreenSaver",
            "org.freedesktop.ScreenSaver",
        )
        .await
        else {
            return true;
        };
        proxy
            .call::<_, _, bool>("GetActive", &())
            .await
            .unwrap_or(true)
    })
}

/// Open a local directory in the user's default file manager (Dolphin, Nautilus, etc.) via `xdg-open`.
pub fn open_directory_in_file_manager(dir: &Path) -> Result<String, String> {
    fs::create_dir_all(dir).map_err(|e| format!("Failed creating {}: {e}", dir.display()))?;
    Command::new("xdg-open")
        .arg(dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("Could not launch file manager (xdg-open): {e}"))?;
    Ok(format!("Opened {}", dir.display()))
}

/// Safely update `[Wayland]` inside `kwinrc`, setting `InputMethod[$e]` and
/// `VirtualKeyboardEnabled=true` while preserving all other sections and keys.
pub fn set_kwin_input_method_in_file(path: &Path, desktop_path: &str) -> std::io::Result<()> {
    let content = if path.exists() {
        fs::read_to_string(path)?
    } else {
        String::new()
    };

    let mut lines: Vec<String> = content.lines().map(ToOwned::to_owned).collect();
    let mut in_wayland = false;
    let mut found_section = false;
    let mut found_im = false;
    let mut found_vk_enabled = false;
    let mut section_end_idx = lines.len();

    for (idx, line) in lines.iter_mut().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if in_wayland {
                section_end_idx = idx;
                in_wayland = false;
            }
            if trimmed == "[Wayland]" {
                in_wayland = true;
                found_section = true;
            }
            continue;
        }
        if !in_wayland {
            continue;
        }
        if trimmed.starts_with("InputMethod") {
            *line = format!("InputMethod[$e]={desktop_path}");
            found_im = true;
        } else if trimmed.starts_with("VirtualKeyboardEnabled=") {
            *line = "VirtualKeyboardEnabled=true".to_owned();
            found_vk_enabled = true;
        }
    }

    if !found_section {
        if !lines.is_empty() && !lines.last().is_some_and(String::is_empty) {
            lines.push(String::new());
        }
        lines.push("[Wayland]".to_owned());
        lines.push(format!("InputMethod[$e]={desktop_path}"));
        lines.push("VirtualKeyboardEnabled=true".to_owned());
    } else {
        let mut insert_pos = section_end_idx;
        if !found_im {
            lines.insert(insert_pos, format!("InputMethod[$e]={desktop_path}"));
            insert_pos += 1;
        }
        if !found_vk_enabled {
            lines.insert(insert_pos, "VirtualKeyboardEnabled=true".to_owned());
        }
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut out = lines.join("\n");
    out.push('\n');
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, out)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Safely remove `wlboard-im.desktop` from `[Wayland]` inside `kwinrc` while preserving
/// every other KWin section and setting.
pub fn remove_kwin_input_method_in_file(path: &Path) -> std::io::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let content = fs::read_to_string(path)?;
    let mut out_lines = Vec::new();
    let mut in_wayland = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_wayland = trimmed == "[Wayland]";
            out_lines.push(line.to_owned());
            continue;
        }
        if in_wayland
            && ((trimmed.starts_with("InputMethod") && trimmed.contains("wlboard-im.desktop"))
                || trimmed == "VirtualKeyboardEnabled=true")
        {
            continue;
        }
        out_lines.push(line.to_owned());
    }

    let mut out = out_lines.join("\n");
    out.push('\n');
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, out)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn updates_and_removes_kwinrc_wayland_section_without_clobbering_other_settings() {
        let dir = std::env::temp_dir().join(format!("wlboard_kwinrc_test_{}", std::process::id()));
        let path = dir.join("kwinrc");
        fs::create_dir_all(&dir).expect("mkdir");
        fs::write(
            &path,
            "[Compositing]\nLatencyPolicy=Low\n\n[Wayland]\nEnablePrimarySelection=true\nInputMethod[$e]=/usr/share/applications/org.kde.plasma.keyboard.desktop\n\n[Windows]\nFocusPolicy=ClickToFocus\n",
        )
        .expect("write kwinrc");

        set_kwin_input_method_in_file(&path, IM_DESKTOP_PATH).expect("update kwinrc");
        let updated = fs::read_to_string(&path).expect("read updated");

        assert!(updated.contains("LatencyPolicy=Low"));
        assert!(updated.contains("EnablePrimarySelection=true"));
        assert!(updated.contains(&format!("InputMethod[$e]={IM_DESKTOP_PATH}")));
        assert!(updated.contains("VirtualKeyboardEnabled=true"));
        assert!(updated.contains("FocusPolicy=ClickToFocus"));
        assert!(!updated.contains("plasma.keyboard.desktop"));

        remove_kwin_input_method_in_file(&path).expect("remove from kwinrc");
        let cleaned = fs::read_to_string(&path).expect("read cleaned");
        assert!(cleaned.contains("LatencyPolicy=Low"));
        assert!(cleaned.contains("EnablePrimarySelection=true"));
        assert!(cleaned.contains("FocusPolicy=ClickToFocus"));
        assert!(!cleaned.contains("wlboard-im.desktop"));
        assert!(!cleaned.contains("VirtualKeyboardEnabled=true"));

        let _ = fs::remove_dir_all(&dir);
    }
}
