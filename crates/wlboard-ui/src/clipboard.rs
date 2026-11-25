//! Lightweight Wayland & X11 system clipboard bridge for inline Settings & Panel Editor fields.
//!
//! Works with zero extra C dependencies:
//! - Maintains an in-process clipboard buffer so copying/pasting between fields always works instantly.
//! - Syncs bidirectionally with the Wayland/X11 desktop clipboard via native protocol hooks (when provided
//!   by the Wayland/X11 runtime) plus automatic CLI fallback (`wl-copy`/`wl-paste`, `xclip`, `xsel`) so
//!   copying to external desktop apps works reliably even when an unfocused surface copies via mouse click.

use std::io::Write;
use std::process::{Command, Stdio};

/// In-process + system desktop clipboard cache.
#[derive(Debug, Default, Clone)]
pub struct ClipboardBridge {
    last_text: String,
}

impl ClipboardBridge {
    #[must_use]
    pub fn new() -> Self {
        Self {
            last_text: String::new(),
        }
    }

    /// Store `text` locally and push it to external desktop clipboards on a best-effort basis.
    pub fn set_text(&mut self, text: &str) {
        self.last_text = text.to_owned();
        let owned = self.last_text.clone();
        // Best-effort external clipboard helpers so copied text survives even after wlboard closes
        // or when copying via mouse click without an active Wayland keyboard serial.
        let _ = std::thread::Builder::new()
            .name("wlboard-clip-set".to_owned())
            .spawn(move || {
                if try_pipe_cmd("wl-copy", &[], &owned) {
                    return;
                }
                if try_pipe_cmd("xclip", &["-selection", "clipboard"], &owned) {
                    return;
                }
                let _ = try_pipe_cmd("xsel", &["--clipboard", "--input"], &owned);
            });
    }

    /// Return the currently stored in-process text (used when serving Wayland/X11 selection requests).
    #[must_use]
    pub fn cached_text(&self) -> &str {
        &self.last_text
    }

    /// Convenience alias for `set_text`.
    pub fn copy(&mut self, text: &str) {
        self.set_text(text);
    }

    /// Read text from the desktop clipboard (falling back to the cached in-process buffer).
    #[must_use]
    pub fn get_text(&mut self) -> String {
        if let Some(txt) = try_read_cmd("wl-paste", &["--no-newline"])
            .or_else(|| try_read_cmd("xclip", &["-selection", "clipboard", "-o"]))
            .or_else(|| try_read_cmd("xsel", &["--clipboard", "--output"]))
            && !txt.is_empty()
        {
            self.last_text = txt;
        }
        self.last_text.clone()
    }

    /// Return non-empty clipboard text if available.
    #[must_use]
    pub fn paste(&mut self) -> Option<String> {
        let s = self.get_text();
        if s.is_empty() { None } else { Some(s) }
    }
}

fn try_pipe_cmd(program: &str, args: &[&str], text: &str) -> bool {
    let Ok(mut child) = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(text.as_bytes());
    }
    // Wait briefly for wl-copy/xclip to fork/complete cleanly.
    let _ = child.wait();
    true
}

fn try_read_cmd(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&output.stdout).into_owned();
    let trimmed = s.trim_end_matches(['\r', '\n']).to_owned();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}
