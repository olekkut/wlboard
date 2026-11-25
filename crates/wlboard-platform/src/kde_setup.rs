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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn updates_kwinrc_wayland_section_without_clobbering_other_settings() {
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

        let _ = fs::remove_dir_all(&dir);
    }
}
