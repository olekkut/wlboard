# wlboard

**wlboard** is a lightweight, deeply customizable native Wayland virtual keyboard and custom panel studio written in Rust.

Built directly on `smithay-client-toolkit`, `tiny-skia`, `cosmic-text`, and `resvg`, it renders crisp anti-aliased keycaps and monochrome vector icons with zero GTK, Qt, or GPU-driver dependencies.

## Highlights

- **Dual-Mode Wayland Support (Floating Desktop Overlay + Lockscreen / SDDM)**:
  - **Floating Overlay (`wlboard --toggle`)**: Uses `zwlr_layer_shell_v1` (`Layer::Overlay`, `KeyboardInteractivity::None`) so the keyboard stays above windows without stealing focus on **KDE Plasma**, **Hyprland**, and **Sway**, with smooth drag and 8-edge resize.
  - **Native Input Method (`wlboard --input-method`)**: Implements `zwp_input_method_v1` + `zwp_input_panel_v1` and installs `io.github.olekkut.wlboard-im.desktop` (`X-KDE-Wayland-VirtualKeyboard=true`) so `wlboard` registers directly in **KDE System Settings → Keyboard → Virtual Keyboard** and works on the **KDE Plasma Lockscreen** (plus optional SDDM login screen setup).
- **Built-in Visual Panel Editor (`0.5u` Snap Grid)**: Design, duplicate, and customize keyboard panels visually with drag-to-move, corner-handle resize, and a full Key Inspector supporting layout keys, fixed keys, multi-key shortcuts (`Ctrl+C`, `Alt+Tab`, `Super+D`), text snippets, panel switchers, and per-key color tints.
- **4 Starter Panels + Quick Panel Switcher**: Switch instantly between **Standard** (with half-height Inverted-T arrows and forward `Del`), **Compact TV** (`ABC` + `123` / `F1–F12` / Navigation), **Numpad & Nav**, and **Shortcuts & Media**, plus your own custom JSON panels (`~/.config/wlboard/panels/*.json`).
- **Keycap Style Toggle**: Choose between crisp **Text Labels** (`Tab`, `Caps`, `Shift`, `Return`, `Del`) or clean **Monochrome SVG Symbols**.
- **Customizable Fast Language Switcher & Automatic KDE Layout Registration**: Built-in **English (`EN`)**, **Polish (`PL`)**, **Ukrainian (`UA`)**, **Russian (`RU`)**, and **Arabic (`AR`)** layouts with HarfBuzz/BiDi shaping, optional AltGr amber diacritic hints, per-language show/hide and top-bar reordering, two-way KDE Plasma D-Bus layout sync, and optional automatic `kxkbrc` layout registration so newly picked languages work immediately without manual KDE setup.
- **Automatic Input Injection & True Multi-Touch Chording**: Native `zwp_input_method_v1` (`commit_string` + `keysym`) in Lockscreen/IM mode, `zwp_virtual_keyboard_v1` on Hyprland/Sway, and pure-Rust `/dev/uinput` fallback with a **250 ms stuck-key watchdog** on KDE Plasma desktop. Full `wl_touch` chording lets you hold `Shift`, `Ctrl`, `Alt`, or `AltGr` with one finger while tapping keys with another.
- **Live-Themed Settings & User Presets Window**: 4 theme bases (**Graphite Dark** with crisp white outline border, **Frost Light**, **High Contrast TV**, **Slate**), quick accent swatches, an inline HSV/Hex color picker for all 7 color slots, border/opacity/font/radius/spacing sliders, and saveable JSON **User Presets**.
- **Native System Tray & Instant CLI**: Built-in `StatusNotifierItem` + D-Bus menu tray icon with panel switching and single-instance D-Bus IPC (`wlboard --toggle`, `wlboard --settings`, `wlboard --panel-editor`).

---

## Screenshots

### Graphite Dark — Standard Layout (Inverted-T Arrows, Forward `Del`, Monochrome SVG Symbols & Media Row)
![Graphite Dark Standard Layout with Media Row](screenshots/graphite_dark_standard_media.png)

### Built-in Visual Panel Editor (3-Pane `0.5u` Snap-Grid Studio)
![Visual Panel Editor Window](screenshots/panel_editor_window.png)

### Built-in Settings — Layout, Panels & Fast Language Switcher Filtering
![Settings Layout and Panels Tab](screenshots/settings_layout_panels_tab.png)

### Starter Panels — Shortcuts & Media and Numpad & Nav
![Shortcuts and Media Panel](screenshots/shortcuts_media_panel.png)
![Numpad and Nav Panel](screenshots/numpad_nav_panel.png)

### Graphite Dark — Compact for TV (Ukrainian)
![Graphite Dark Compact for TV Ukrainian](screenshots/graphite_dark_compact_tv_ua.png)

### Frost Light — Polish Layout with AltGr Amber Diacritic Hints
![Frost Light Polish AltGr](screenshots/frost_light_pl_altgr.png)

### Custom User Preset ("Midnight Cyan") — Arabic Layout
![Custom User Preset Midnight Cyan Arabic](screenshots/custom_preset_midnight_cyan_ar.png)

### Built-in Customize wlboard — Appearance & HSV Color Slots
![Customize wlboard Appearance Window](screenshots/customize_settings_window.png)

---

## Installation

### Ubuntu / Kubuntu / Debian (`.deb` Package — `amd64`)

Install the latest prebuilt `.deb` package (automatically installs the `/dev/uinput` udev rule, KDE Plasma virtual keyboard desktop entry, and SDDM helper):

```bash
curl -fsSL -o /tmp/wlboard.deb https://raw.githubusercontent.com/olekkut/wlboard/deb-build/wlboard_0.2.0-1_amd64.deb \
  && sudo apt install -y /tmp/wlboard.deb \
  && rm -f /tmp/wlboard.deb
```

### Build from Source

Requires Rust 1.89+ (`cargo`):

```bash
git clone https://github.com/olekkut/wlboard.git
cd wlboard
cargo build --release
sudo install -Dm755 target/release/wlboard /usr/bin/wlboard
sudo install -Dm755 packaging/deb/wlboard-im /usr/bin/wlboard-im
sudo install -Dm644 packaging/deb/io.github.olekkut.wlboard.desktop /usr/share/applications/io.github.olekkut.wlboard.desktop
sudo install -Dm644 packaging/deb/io.github.olekkut.wlboard-im.desktop /usr/share/applications/io.github.olekkut.wlboard-im.desktop
sudo install -Dm644 assets/io.github.olekkut.wlboard.svg /usr/share/icons/hicolor/scalable/apps/io.github.olekkut.wlboard.svg
```

On **KDE Plasma Wayland** desktop (where `/dev/uinput` fallback is used in floating overlay mode), run once to grant local session access to `/dev/uinput`:

```bash
./packaging/deb/setup-uinput.sh
```

---

## KDE Plasma Lockscreen & SDDM Login Screen Setup

### KDE Plasma Lockscreen (One Click)

1. Open **System Settings → Keyboard → Virtual Keyboard**.
2. Select **wlboard Virtual Keyboard** and click **Apply**.
3. Lock your screen (`Super+L`) and click the **Virtual Keyboard** button on the lockscreen (or focus the password input on a touchscreen).

> **Tip for non-touchscreen desktops:** KWin shows the input-method virtual keyboard automatically on touchscreens and when you click the Virtual Keyboard button on the lockscreen. If you want it to pop up on mouse clicks too, add `KWIN_IM_SHOW_ALWAYS=1` to `~/.config/environment.d/wlboard.conf` and log back in.

### Optional: SDDM Wayland Login Screen

To also enable `wlboard` on the SDDM login screen before logging in, run the bundled helper script:

```bash
/usr/share/wlboard/scripts/setup-sddm.sh
```

This writes `/etc/sddm.conf.d/70-wlboard.conf` configuring `kwin_wayland --inputmethod /usr/bin/wlboard-im`. To revert at any time:

```bash
/usr/share/wlboard/scripts/setup-sddm.sh --remove
```

---

## Usage & CLI

| Command | Action |
|---|---|
| `wlboard` | Start `wlboard` floating overlay (or bring an existing instance to front) |
| `wlboard --toggle` | Toggle visibility (bind this to a panel button or compositor shortcut) |
| `wlboard --settings` | Open the built-in **Customize wlboard** window |
| `wlboard --panel-editor` | Open the built-in **Panel Editor** studio |
| `wlboard --panel <id>` | Switch active panel (`standard`, `compact_tv`, `numpad_nav`, `shortcuts_media`, or custom ID) |
| `wlboard --layout <code >` | Switch active language layout (`en`, `pl`, `uk`, `ru`, `ar`) |
| `wlboard --input-method` | Run as a Wayland `zwp_input_method_v1` / `zwp_input_panel_v1` virtual keyboard (launched automatically by KWin on Lockscreen/SDDM) |
| `wlboard --hide` | Hide the floating overlay |
| `wlboard --quit` | Release all keys and exit cleanly |

### Configuration & User Data

- Settings file: `~/.config/wlboard/config.toml`
- Saved Custom Panels: `~/.config/wlboard/panels/*.json`
- Saved User Presets: `~/.config/wlboard/presets/*.json`
- Custom Layout Overrides: `~/.config/wlboard/layouts/*.json`

---

## Workspace Architecture

| Crate | Purpose |
|---|---|
| `crates/wlboard-core` | Pure Rust domain model: unified 2D `0.5u` snap-grid panel model, 4 starter panels + JSON `PanelStore`, JSON language layouts, modifier & Sticky Lock state machine, theme palette/contrast math, TOML config, JSON User Presets, multi-touch & 8-edge resize geometry |
| `crates/wlboard-platform` | OS & desktop adapters: `zwp_input_method_v1` direct Unicode/keysym backend, `zwp_virtual_keyboard_v1` + dynamic XKB keymap compiler, pure-Rust `evdev` uinput + 250 ms watchdog, KDE Plasma two-way D-Bus layout sync + automatic `kxkbrc` layout registration, StatusNotifierItem tray, single-instance D-Bus IPC |
| `crates/wlboard-ui` | Unified `tiny-skia` + `cosmic-text` + `resvg` renderer, dual-mode `zwlr_layer_shell_v1` / `zwp_input_panel_v1` keyboard surface, 3-pane visual Panel Editor window, live-themed `Customize wlboard` window, offline HiDPI PNG preview generator |
| `crates/wlboard-app` | Thin CLI binary (`wlboard`), logging, IPC dispatch, and `SIGINT`/`SIGTERM` clean key-release shutdown |

---

## License

Copyright (C) 2026 Aleksander Kutycki (`olekkut`).

Licensed under the **GNU General Public License v3.0 or later** (`GPL-3.0-or-later`). See [LICENSE](LICENSE) for full terms.
