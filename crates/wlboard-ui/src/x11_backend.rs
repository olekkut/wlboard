//! Pure-Rust XWayland / X11 runtime (`x11rb`) providing automatic fallback on Ubuntu GNOME
//! (where `zwlr_layer_shell_v1` is not exposed) and native X11 desktop sessions:
//! - 32-bit ARGB TrueColor transparency with crisp rounded corners
//! - Non-focus-stealing floating keyboard (`WM_HINTS { input = False }`, `_NET_WM_STATE_ABOVE`,
//!   `STICKY`, `SKIP_TASKBAR`, `SKIP_PAGER`)
//! - Borderless draggable & 8-edge resizable Settings & Panel Editor windows (`_NET_WM_MOVERESIZE`)
//! - Strictly event-driven `calloop` epoll loop (**0 wakeups/sec** while idle)

use std::os::fd::{AsFd, AsRawFd, BorrowedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use calloop::channel::{self, Channel};
use calloop::generic::Generic;
use calloop::timer::{TimeoutAction, Timer};
use calloop::{EventLoop, Interest, LoopHandle, Mode, PostAction, RegistrationToken};
use tiny_skia::Pixmap;
use tracing::{debug, info, warn};
use wlboard_core::{
    AppConfig, ContactId, GridKeyAction, KeyCode, KeyRole, KeyboardState, KeyboardStyle,
    MIN_WINDOW_HEIGHT, MIN_WINDOW_WIDTH, OutputBounds, PanelStore, Point, PresetStore, ResizeEdge,
    WindowGeometry, apply_resize, clamp_move,
};
use wlboard_platform::{
    InputManager, PlatformEvent, PlatformHandle, TrayState, disable_kde_lockscreen_mouse_override,
    disable_kde_lockscreen_support, enable_kde_lockscreen_support, open_directory_in_file_manager,
};
use x11rb::connection::Connection as X11Connection;
use x11rb::protocol::Event as X11Event;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ClientMessageEvent, ColormapAlloc, ConfigureWindowAux, ConnectionExt,
    CreateGCAux, CreateWindowAux, EventMask, Gcontext, ImageFormat, KeyButMask, PropMode,
    VisualClass, Visualid, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as WrapperConnectionExt;

use crate::canvas::{UiRenderer, copy_pixmap_to_wayland_argb8888};
use crate::clipboard::ClipboardBridge;
use crate::keyboard_view::{KeyboardHit, KeyboardViewLayout};
use crate::panel_editor::{
    PANEL_EDITOR_DEFAULT_HEIGHT, PANEL_EDITOR_DEFAULT_WIDTH, PanelEditorAction, PanelEditorView,
};
use crate::settings_view::{
    SETTINGS_DEFAULT_HEIGHT, SETTINGS_DEFAULT_WIDTH, SettingsAction, SettingsView,
};
use crate::wayland::LaunchOptions;

const TOPLEVEL_RESIZE_BORDER: f32 = 7.0;
const REPEAT_INITIAL_DELAY: Duration = Duration::from_millis(420);
const REPEAT_INTERVAL: Duration = Duration::from_millis(55);
const WATCHDOG_CHECK_DELAY: Duration = Duration::from_millis(400);

// EWMH `_NET_WM_MOVERESIZE` direction constants
const MOVERESIZE_SIZE_TOPLEFT: u32 = 0;
const MOVERESIZE_SIZE_TOP: u32 = 1;
const MOVERESIZE_SIZE_TOPRIGHT: u32 = 2;
const MOVERESIZE_SIZE_RIGHT: u32 = 3;
const MOVERESIZE_SIZE_BOTTOMRIGHT: u32 = 4;
const MOVERESIZE_SIZE_BOTTOM: u32 = 5;
const MOVERESIZE_SIZE_BOTTOMLEFT: u32 = 6;
const MOVERESIZE_SIZE_LEFT: u32 = 7;
const MOVERESIZE_MOVE: u32 = 8;

struct BorrowedRawFd(RawFd);

impl AsFd for BorrowedRawFd {
    fn as_fd(&self) -> BorrowedFd<'_> {
        // SAFETY: `RustConnection` owns the underlying socket and outlives the `EventLoop`.
        unsafe { BorrowedFd::borrow_raw(self.0) }
    }
}

struct X11Atoms {
    wm_protocols: Atom,
    wm_delete_window: Atom,
    wm_hints: Atom,
    wm_normal_hints: Atom,
    wm_class: Atom,
    motif_wm_hints: Atom,
    net_wm_name: Atom,
    utf8_string: Atom,
    net_wm_window_type: Atom,
    net_wm_window_type_utility: Atom,
    net_wm_state: Atom,
    net_wm_state_above: Atom,
    net_wm_state_sticky: Atom,
    net_wm_state_skip_taskbar: Atom,
    net_wm_state_skip_pager: Atom,
    net_wm_moveresize: Atom,
}

impl X11Atoms {
    fn intern(conn: &RustConnection) -> Result<Self, String> {
        let get = |name: &[u8]| -> Result<Atom, String> {
            conn.intern_atom(false, name)
                .map_err(|e| format!("X11 intern_atom error: {e}"))?
                .reply()
                .map(|r| r.atom)
                .map_err(|e| format!("X11 intern_atom reply error: {e}"))
        };
        Ok(Self {
            wm_protocols: get(b"WM_PROTOCOLS")?,
            wm_delete_window: get(b"WM_DELETE_WINDOW")?,
            wm_hints: get(b"WM_HINTS")?,
            wm_normal_hints: get(b"WM_NORMAL_HINTS")?,
            wm_class: get(b"WM_CLASS")?,
            motif_wm_hints: get(b"_MOTIF_WM_HINTS")?,
            net_wm_name: get(b"_NET_WM_NAME")?,
            utf8_string: get(b"UTF8_STRING")?,
            net_wm_window_type: get(b"_NET_WM_WINDOW_TYPE")?,
            net_wm_window_type_utility: get(b"_NET_WM_WINDOW_TYPE_UTILITY")?,
            net_wm_state: get(b"_NET_WM_STATE")?,
            net_wm_state_above: get(b"_NET_WM_STATE_ABOVE")?,
            net_wm_state_sticky: get(b"_NET_WM_STATE_STICKY")?,
            net_wm_state_skip_taskbar: get(b"_NET_WM_STATE_SKIP_TASKBAR")?,
            net_wm_state_skip_pager: get(b"_NET_WM_STATE_SKIP_PAGER")?,
            net_wm_moveresize: get(b"_NET_WM_MOVERESIZE")?,
        })
    }
}

#[derive(Debug, Clone, Copy)]
enum X11KbDrag {
    Move {
        start_root_x: i32,
        start_root_y: i32,
        start_geom: WindowGeometry,
    },
    Resize {
        edge: ResizeEdge,
        start_root_x: i32,
        start_root_y: i32,
        start_geom: WindowGeometry,
    },
}

#[derive(Debug, Clone, Copy)]
struct RepeatState {
    keycode: KeyCode,
    next_fire: Instant,
}

struct X11WindowSlot {
    window: Window,
    gc: Gcontext,
    width: u32,
    height: u32,
    dirty: bool,
}

struct X11KbWindow {
    slot: X11WindowSlot,
    layout: KeyboardViewLayout,
}

struct X11Runtime {
    conn: Arc<RustConnection>,
    root: Window,
    depth: u8,
    visual: Visualid,
    colormap: u32,
    atoms: X11Atoms,
    output_bounds: OutputBounds,

    input: InputManager,
    renderer: UiRenderer,
    clipboard: ClipboardBridge,
    state: KeyboardState,
    preset_store: PresetStore,
    panel_store: PanelStore,
    platform: PlatformHandle,

    kb_win: Option<X11KbWindow>,
    settings_win: Option<X11WindowSlot>,
    settings_view: SettingsView,
    editor_win: Option<X11WindowSlot>,
    editor_view: PanelEditorView,

    hovered_toolbar: Option<KeyboardHit>,
    kb_drag: Option<X11KbDrag>,
    repeat: Option<RepeatState>,
    config_dirty_since: Option<Instant>,
    watchdog_deadline: Option<Instant>,
    armed_deadline: Option<Instant>,
    timer_token: Option<RegistrationToken>,
    loop_handle: Option<LoopHandle<'static, Self>>,
    running: bool,
    shutdown_flag: Arc<AtomicBool>,
}

/// Launch the pure-Rust `x11rb` XWayland/X11 runtime (Ubuntu GNOME & X11 desktops) with 0/s idle wakeups.
pub fn run_x11_app(
    options: LaunchOptions,
    shutdown_flag: Arc<AtomicBool>,
    signal_pipe: Option<UnixStream>,
) -> Result<(), String> {
    let (conn, screen_num) = RustConnection::connect(None)
        .map_err(|e| format!("Could not connect to X11/XWayland display (is DISPLAY set?): {e}"))?;
    let conn = Arc::new(conn);

    let screen = &conn.setup().roots[screen_num];
    let root = screen.root;
    let output_bounds = OutputBounds {
        width: u32::from(screen.width_in_pixels).max(800),
        height: u32::from(screen.height_in_pixels).max(600),
    };

    // Prefer a 32-bit ARGB TrueColor visual for crisp rounded corners & window transparency.
    let (depth, visual, colormap) = find_argb_visual(&conn, screen_num).unwrap_or((
        screen.root_depth,
        screen.root_visual,
        screen.default_colormap,
    ));
    let atoms = X11Atoms::intern(&conn)?;

    let config = AppConfig::load_default();
    let preset_store = PresetStore::default_store();
    let panel_store = PanelStore::default_store();
    let mut kb_state = KeyboardState::new(config, &preset_store, &panel_store);
    kb_state.config.window.clamp_to_output(output_bounds);
    if let Some(ref pid) = options.initial_panel {
        kb_state.set_panel(pid);
    }
    if options.force_show {
        kb_state.visible = true;
    }
    if options.open_settings {
        kb_state.settings_open = true;
    }
    if options.open_panel_editor {
        kb_state.panel_editor_open = true;
    }

    let (platform_tx, platform_rx): (channel::Sender<PlatformEvent>, Channel<PlatformEvent>) =
        channel::channel();

    let initial_tray = TrayState {
        visible: kb_state.visible,
        style: kb_state.config.layout.style,
        active_layout: kb_state.config.layout.active.clone(),
        active_panel: kb_state.config.layout.active_panel.clone(),
        panels: kb_state
            .panels
            .iter()
            .map(|p| (p.id.clone(), p.name.clone()))
            .collect(),
        languages: kb_state
            .visible_layouts()
            .into_iter()
            .map(|l| (l.code.clone(), l.display_name.clone()))
            .collect(),
        revision: 1,
    };
    let platform =
        PlatformHandle::spawn(initial_tray, kb_state.registry.clone(), true, move |ev| {
            let _ = platform_tx.send(ev);
        });

    let input = InputManager::auto_select(None, kb_state.active_layout());
    kb_state.input_backend_status = input.status_detail();

    let mut settings_view = SettingsView::new(&kb_state.config.appearance);
    settings_view.display_backend_label = if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        "XWayland (GNOME / Non-Focus-Stealing Overlay)".to_owned()
    } else {
        "X11 (Non-Focus-Stealing Overlay)".to_owned()
    };
    let editor_view = PanelEditorView::new(&kb_state);

    let mut runtime = X11Runtime {
        conn: Arc::clone(&conn),
        root,
        depth,
        visual,
        colormap,
        atoms,
        output_bounds,
        input,
        renderer: UiRenderer::new(),
        clipboard: ClipboardBridge::new(),
        state: kb_state,
        preset_store,
        panel_store,
        platform,
        kb_win: None,
        settings_win: None,
        settings_view,
        editor_win: None,
        editor_view,
        hovered_toolbar: None,
        kb_drag: None,
        repeat: None,
        config_dirty_since: None,
        watchdog_deadline: None,
        armed_deadline: None,
        timer_token: None,
        loop_handle: None,
        running: true,
        shutdown_flag,
    };

    if runtime.state.visible {
        runtime.create_keyboard_window();
    }
    if runtime.state.settings_open {
        runtime.create_settings_window();
    }
    if runtime.state.panel_editor_open {
        runtime.create_editor_window();
    }
    runtime.redraw_dirty();
    let _ = runtime.conn.flush();

    info!(
        "wlboard running via {} (screen {}x{}, depth {})",
        runtime.settings_view.display_backend_label,
        output_bounds.width,
        output_bounds.height,
        depth
    );

    let mut event_loop: EventLoop<X11Runtime> =
        EventLoop::try_new().map_err(|e| format!("Failed creating calloop event loop: {e}"))?;
    let loop_handle = event_loop.handle();
    runtime.loop_handle = Some(loop_handle.clone());

    let x11_raw_fd = conn.stream().as_raw_fd();
    loop_handle
        .insert_source(
            Generic::new(BorrowedRawFd(x11_raw_fd), Interest::READ, Mode::Level),
            |_, _, app| {
                app.drain_x11_events();
                Ok(PostAction::Continue)
            },
        )
        .map_err(|e| format!("Failed registering X11 socket with calloop: {e}"))?;

    loop_handle
        .insert_source(platform_rx, |event, _, app| {
            if let channel::Event::Msg(ev) = event {
                app.handle_platform_event(ev);
            }
        })
        .map_err(|e| format!("Failed registering platform channel: {e}"))?;

    if let Some(sig_rx) = signal_pipe {
        let _ = sig_rx.set_nonblocking(true);
        let _ = loop_handle.insert_source(
            Generic::new(sig_rx, Interest::READ, Mode::Level),
            |_, stream, app| {
                let mut buf = [0u8; 32];
                let _ = std::io::Read::read(&mut &**stream, &mut buf);
                app.running = false;
                Ok(PostAction::Remove)
            },
        );
    }

    // Strictly event-driven loop: blocks indefinitely in epoll_pwait when idle (0 wakeups/sec!).
    while runtime.running && !runtime.shutdown_flag.load(Ordering::Relaxed) {
        runtime.drain_x11_events();
        runtime.redraw_dirty();
        runtime.sync_dynamic_timer();
        let _ = runtime.conn.flush();
        if let Err(err) = event_loop.dispatch(None, &mut runtime) {
            warn!("X11 event loop dispatch error: {err}");
            break;
        }
    }

    info!("Shutting down wlboard (X11/XWayland) cleanly — releasing all virtual keys");
    runtime.release_all_keys();
    runtime.save_config_now();
    runtime.platform.shutdown();
    Ok(())
}

fn find_argb_visual(conn: &RustConnection, screen_num: usize) -> Option<(u8, Visualid, u32)> {
    let screen = &conn.setup().roots[screen_num];
    for d in &screen.allowed_depths {
        if d.depth == 32 {
            for v in &d.visuals {
                if v.class == VisualClass::TRUE_COLOR {
                    let cmap = conn.generate_id().ok()?;
                    conn.create_colormap(ColormapAlloc::NONE, cmap, screen.root, v.visual_id)
                        .ok()?;
                    return Some((32, v.visual_id, cmap));
                }
            }
        }
    }
    None
}

impl X11Runtime {
    fn sync_dynamic_timer(&mut self) {
        let Some(ref handle) = self.loop_handle else {
            return;
        };

        let repeat_at = self.repeat.map(|r| r.next_fire);
        let save_at = self
            .config_dirty_since
            .map(|t| t + Duration::from_millis(250));
        let watchdog_at = if self.input.has_held_keys() {
            Some(
                *self
                    .watchdog_deadline
                    .get_or_insert_with(|| Instant::now() + WATCHDOG_CHECK_DELAY),
            )
        } else {
            self.watchdog_deadline = None;
            None
        };

        let desired = [repeat_at, save_at, watchdog_at]
            .into_iter()
            .flatten()
            .min();

        if desired == self.armed_deadline && (desired.is_none() == self.timer_token.is_none()) {
            return;
        }

        if let Some(tok) = self.timer_token.take() {
            handle.remove(tok);
        }
        self.armed_deadline = desired;

        if let Some(at) = desired {
            let dur = at.saturating_duration_since(Instant::now());
            if let Ok(tok) = handle.insert_source(Timer::from_duration(dur), |_, _, app| {
                app.timer_token = None;
                app.armed_deadline = None;
                app.periodic_tick();
                TimeoutAction::Drop
            }) {
                self.timer_token = Some(tok);
            }
        }
    }

    fn periodic_tick(&mut self) {
        let now = Instant::now();

        if let Some(ref mut rep) = self.repeat
            && now >= rep.next_fire
        {
            self.input
                .execute(&[wlboard_core::InputCommand::Tap(rep.keycode)]);
            rep.next_fire = now + REPEAT_INTERVAL;
        }

        if self.input.has_held_keys() {
            let mut active_held = Vec::new();
            for kind in [
                wlboard_core::ModifierKind::Shift,
                wlboard_core::ModifierKind::Ctrl,
                wlboard_core::ModifierKind::Alt,
                wlboard_core::ModifierKind::AltGr,
                wlboard_core::ModifierKind::Super,
            ] {
                if self.state.modifiers.is_active(kind) {
                    active_held.push(kind.keycode());
                }
            }
            if let Some(ref rep) = self.repeat {
                active_held.push(rep.keycode);
            }
            let released = self.input.watchdog_tick(&active_held, now);
            if released > 0 {
                debug!("Stuck-key watchdog released {released} orphaned key(s)");
            }
            self.watchdog_deadline = if self.input.has_held_keys() {
                Some(now + WATCHDOG_CHECK_DELAY)
            } else {
                None
            };
        } else {
            self.watchdog_deadline = None;
        }

        if let Some(since) = self.config_dirty_since
            && now.duration_since(since) >= Duration::from_millis(250)
        {
            self.save_config_now();
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn create_window_common(
        &self,
        x: i16,
        y: i16,
        w: u16,
        h: u16,
        title: &str,
        non_focus_stealing: bool,
        min_size: Option<(u32, u32)>,
    ) -> Option<X11WindowSlot> {
        let win = self.conn.generate_id().ok()?;
        let gc = self.conn.generate_id().ok()?;

        let mut events = EventMask::EXPOSURE
            | EventMask::BUTTON_PRESS
            | EventMask::BUTTON_RELEASE
            | EventMask::POINTER_MOTION
            | EventMask::LEAVE_WINDOW
            | EventMask::STRUCTURE_NOTIFY;
        if !non_focus_stealing {
            events |= EventMask::KEY_PRESS | EventMask::KEY_RELEASE;
        }

        let aux = CreateWindowAux::new()
            .background_pixel(0)
            .border_pixel(0)
            .colormap(self.colormap)
            .event_mask(events);

        self.conn
            .create_window(
                self.depth,
                win,
                self.root,
                x,
                y,
                w,
                h,
                0,
                WindowClass::INPUT_OUTPUT,
                self.visual,
                &aux,
            )
            .ok()?;

        self.conn.create_gc(gc, win, &CreateGCAux::new()).ok()?;

        // Title + WM_CLASS
        let _ = self.conn.change_property8(
            PropMode::REPLACE,
            win,
            AtomEnum::WM_NAME,
            AtomEnum::STRING,
            title.as_bytes(),
        );
        let _ = self.conn.change_property8(
            PropMode::REPLACE,
            win,
            self.atoms.net_wm_name,
            self.atoms.utf8_string,
            title.as_bytes(),
        );
        let _ = self.conn.change_property8(
            PropMode::REPLACE,
            win,
            self.atoms.wm_class,
            AtomEnum::STRING,
            b"wlboard\0io.github.olekkut.wlboard\0",
        );

        // Borderless window (`_MOTIF_WM_HINTS`: flags = 2 (decorations), decorations = 0)
        let motif_hints: [u32; 5] = [2, 0, 0, 0, 0];
        let _ = self.conn.change_property32(
            PropMode::REPLACE,
            win,
            self.atoms.motif_wm_hints,
            self.atoms.motif_wm_hints,
            &motif_hints,
        );

        // WM_PROTOCOLS -> WM_DELETE_WINDOW
        let _ = self.conn.change_property32(
            PropMode::REPLACE,
            win,
            self.atoms.wm_protocols,
            AtomEnum::ATOM,
            &[self.atoms.wm_delete_window],
        );

        // WM_HINTS: InputHint (flag 1) = 0 for non-focus-stealing keyboard, 1 for Settings/Editor
        let mut wm_hints = [0u32; 9];
        wm_hints[0] = 1; // InputHint
        wm_hints[1] = if non_focus_stealing { 0 } else { 1 };
        let _ = self.conn.change_property32(
            PropMode::REPLACE,
            win,
            self.atoms.wm_hints,
            self.atoms.wm_hints,
            &wm_hints,
        );

        if non_focus_stealing {
            let _ = self.conn.change_property32(
                PropMode::REPLACE,
                win,
                self.atoms.net_wm_window_type,
                AtomEnum::ATOM,
                &[self.atoms.net_wm_window_type_utility],
            );
            let _ = self.conn.change_property32(
                PropMode::REPLACE,
                win,
                self.atoms.net_wm_state,
                AtomEnum::ATOM,
                &[
                    self.atoms.net_wm_state_above,
                    self.atoms.net_wm_state_sticky,
                    self.atoms.net_wm_state_skip_taskbar,
                    self.atoms.net_wm_state_skip_pager,
                ],
            );
        }

        if let Some((min_w, min_h)) = min_size {
            // WM_SIZE_HINTS (18 x u32): PMinSize = 1 << 4
            let mut size_hints = [0u32; 18];
            size_hints[0] = 1 << 4;
            size_hints[5] = min_w;
            size_hints[6] = min_h;
            let _ = self.conn.change_property32(
                PropMode::REPLACE,
                win,
                self.atoms.wm_normal_hints,
                AtomEnum::WM_SIZE_HINTS,
                &size_hints,
            );
        }

        let _ = self.conn.map_window(win);
        if non_focus_stealing {
            // Re-assert position after map (some WMs ignore initial create_window x/y)
            let _ = self.conn.configure_window(
                win,
                &ConfigureWindowAux::new().x(i32::from(x)).y(i32::from(y)),
            );
        }

        Some(X11WindowSlot {
            window: win,
            gc,
            width: u32::from(w),
            height: u32::from(h),
            dirty: true,
        })
    }

    fn destroy_window_slot(&self, slot: X11WindowSlot) {
        let _ = self.conn.free_gc(slot.gc);
        let _ = self.conn.destroy_window(slot.window);
    }

    fn create_keyboard_window(&mut self) {
        if self.kb_win.is_some() {
            return;
        }
        self.state.config.window.clamp_to_output(self.output_bounds);
        let g = self.state.config.window;
        if let Some(slot) = self.create_window_common(
            g.x as i16,
            g.y as i16,
            g.width as u16,
            g.height as u16,
            "wlboard",
            true,
            Some((MIN_WINDOW_WIDTH, MIN_WINDOW_HEIGHT)),
        ) {
            let layout = KeyboardViewLayout::compute(&self.state, g.width as f32, g.height as f32);
            self.kb_win = Some(X11KbWindow { slot, layout });
            self.state.visible = true;
            self.sync_tray_state();
        }
    }

    fn destroy_keyboard_window(&mut self) {
        self.release_all_keys();
        self.kb_drag = None;
        self.hovered_toolbar = None;
        self.state.hovered_key_id = None;
        if let Some(kb) = self.kb_win.take() {
            self.destroy_window_slot(kb.slot);
        }
        self.state.visible = false;
        self.sync_tray_state();
    }

    fn create_settings_window(&mut self) {
        if self.settings_win.is_some() {
            return;
        }
        self.state.reload_presets(&self.preset_store);
        self.state.reload_panels(&self.panel_store);
        self.settings_view
            .sync_from_appearance(&self.state.config.appearance);
        self.settings_view.refresh_system_status();

        let max_w = self.output_bounds.width.saturating_sub(32).max(640);
        let max_h = self.output_bounds.height.saturating_sub(64).max(540);
        let init_w = SETTINGS_DEFAULT_WIDTH.min(max_w);
        let init_h = SETTINGS_DEFAULT_HEIGHT.min(max_h);
        let x = ((self.output_bounds.width.saturating_sub(init_w)) / 2) as i16;
        let y = ((self.output_bounds.height.saturating_sub(init_h)) / 3) as i16;

        if let Some(slot) = self.create_window_common(
            x,
            y,
            init_w as u16,
            init_h as u16,
            "Customize wlboard",
            false,
            Some((640, 540)),
        ) {
            self.settings_view.width = init_w as f32;
            self.settings_view.height = init_h as f32;
            self.settings_win = Some(slot);
            self.state.settings_open = true;
            self.mark_keyboard_dirty();
        }
    }

    fn destroy_settings_window(&mut self) {
        if let Some(slot) = self.settings_win.take() {
            self.destroy_window_slot(slot);
        }
        self.state.settings_open = false;
        self.settings_view.focused_field = None;
        self.mark_keyboard_dirty();
    }

    fn create_editor_window(&mut self) {
        if self.editor_win.is_some() {
            return;
        }
        self.state.reload_panels(&self.panel_store);
        self.editor_view.load_panel(self.state.active_panel_def());

        let max_w = self.output_bounds.width.saturating_sub(32).max(780);
        let max_h = self.output_bounds.height.saturating_sub(64).max(500);
        let init_w = PANEL_EDITOR_DEFAULT_WIDTH.min(max_w);
        let init_h = PANEL_EDITOR_DEFAULT_HEIGHT.min(max_h);
        let x = ((self.output_bounds.width.saturating_sub(init_w)) / 2) as i16;
        let y = ((self.output_bounds.height.saturating_sub(init_h)) / 3) as i16;

        if let Some(slot) = self.create_window_common(
            x,
            y,
            init_w as u16,
            init_h as u16,
            "wlboard Panel Editor",
            false,
            Some((780, 500)),
        ) {
            self.editor_view.width = init_w as f32;
            self.editor_view.height = init_h as f32;
            self.editor_win = Some(slot);
            self.state.panel_editor_open = true;
            self.mark_keyboard_dirty();
        }
    }

    fn destroy_editor_window(&mut self) {
        if let Some(slot) = self.editor_win.take() {
            self.destroy_window_slot(slot);
        }
        self.state.panel_editor_open = false;
        self.editor_view.focused_field = None;
        self.mark_keyboard_dirty();
    }

    fn send_wm_moveresize(&self, win: Window, root_x: i16, root_y: i16, direction: u32) {
        let _ = self.conn.ungrab_pointer(x11rb::CURRENT_TIME);
        let ev = ClientMessageEvent::new(
            32,
            win,
            self.atoms.net_wm_moveresize,
            [
                u32::from(root_x.max(0) as u16),
                u32::from(root_y.max(0) as u16),
                direction,
                1, // left mouse button
                1, // source: normal application
            ],
        );
        let _ = self.conn.send_event(
            false,
            self.root,
            EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
            ev,
        );
        let _ = self.conn.flush();
    }

    fn blit_pixmap(&self, win: Window, gc: Gcontext, pixmap: &Pixmap) {
        let w = pixmap.width();
        let h = pixmap.height();
        let mut bgra = vec![0u8; (w * h * 4) as usize];
        copy_pixmap_to_wayland_argb8888(pixmap, &mut bgra);

        // Send in 64-row strips so we never exceed X11 max_request_length on large windows.
        let strip_h = 64u32;
        let row_bytes = (w * 4) as usize;
        let mut y = 0u32;
        while y < h {
            let chunk_h = strip_h.min(h - y);
            let start = (y as usize) * row_bytes;
            let end = start + (chunk_h as usize) * row_bytes;
            let _ = self.conn.put_image(
                ImageFormat::Z_PIXMAP,
                win,
                gc,
                w as u16,
                chunk_h as u16,
                0,
                y as i16,
                0,
                self.depth,
                &bgra[start..end],
            );
            y += chunk_h;
        }
    }

    fn redraw_dirty(&mut self) {
        if let Some(ref mut kb) = self.kb_win
            && kb.slot.dirty
        {
            let w = kb.slot.width.max(MIN_WINDOW_WIDTH);
            let h = kb.slot.height.max(MIN_WINDOW_HEIGHT);
            kb.layout = KeyboardViewLayout::compute(&self.state, w as f32, h as f32);
            if let Some(mut pixmap) = Pixmap::new(w, h) {
                kb.layout.render(
                    &mut self.renderer,
                    &mut pixmap,
                    &self.state,
                    self.hovered_toolbar.as_ref(),
                    1.0,
                );
                let (win, gc) = (kb.slot.window, kb.slot.gc);
                self.blit_pixmap(win, gc, &pixmap);
            }
            if let Some(ref mut kb) = self.kb_win {
                kb.slot.dirty = false;
            }
        }

        if let Some(ref mut s) = self.settings_win
            && s.dirty
        {
            let w = s.width.max(320);
            let h = s.height.max(320);
            self.settings_view.width = w as f32;
            self.settings_view.height = h as f32;
            if let Some(mut pixmap) = Pixmap::new(w, h) {
                self.settings_view
                    .render(&mut self.renderer, &mut pixmap, &self.state, 1.0);
                let (win, gc) = (s.window, s.gc);
                self.blit_pixmap(win, gc, &pixmap);
            }
            if let Some(ref mut s) = self.settings_win {
                s.dirty = false;
            }
        }

        if let Some(ref mut e) = self.editor_win
            && e.dirty
        {
            let w = e.width.max(640);
            let h = e.height.max(480);
            self.editor_view.width = w as f32;
            self.editor_view.height = h as f32;
            if let Some(mut pixmap) = Pixmap::new(w, h) {
                self.editor_view.render(
                    &mut self.renderer,
                    &mut pixmap,
                    &self.state,
                    &self.panel_store,
                    1.0,
                );
                let (win, gc) = (e.window, e.gc);
                self.blit_pixmap(win, gc, &pixmap);
            }
            if let Some(ref mut e) = self.editor_win {
                e.dirty = false;
            }
        }
    }

    fn drain_x11_events(&mut self) {
        loop {
            match self.conn.poll_for_event() {
                Ok(Some(ev)) => self.handle_x11_event(ev),
                Ok(None) => break,
                Err(err) => {
                    warn!("X11 connection error: {err}");
                    self.running = false;
                    break;
                }
            }
        }
        self.redraw_dirty();
        let _ = self.conn.flush();
    }

    fn handle_x11_event(&mut self, ev: X11Event) {
        match ev {
            X11Event::Expose(e) => {
                if self
                    .kb_win
                    .as_ref()
                    .is_some_and(|k| k.slot.window == e.window)
                {
                    self.mark_keyboard_dirty();
                } else if self
                    .settings_win
                    .as_ref()
                    .is_some_and(|s| s.window == e.window)
                {
                    self.mark_settings_dirty();
                } else if self
                    .editor_win
                    .as_ref()
                    .is_some_and(|ed| ed.window == e.window)
                {
                    self.mark_editor_dirty();
                }
            }
            X11Event::ConfigureNotify(e) => {
                let nw = u32::from(e.width);
                let nh = u32::from(e.height);
                if let Some(ref mut kb) = self.kb_win
                    && kb.slot.window == e.window
                {
                    if kb.slot.width != nw || kb.slot.height != nh {
                        kb.slot.width = nw.max(MIN_WINDOW_WIDTH);
                        kb.slot.height = nh.max(MIN_WINDOW_HEIGHT);
                        self.state.config.window.width = kb.slot.width;
                        self.state.config.window.height = kb.slot.height;
                        kb.slot.dirty = true;
                        self.mark_config_dirty();
                    }
                } else if let Some(ref mut s) = self.settings_win
                    && s.window == e.window
                {
                    if s.width != nw || s.height != nh {
                        s.width = nw.max(640);
                        s.height = nh.max(540);
                        s.dirty = true;
                    }
                } else if let Some(ref mut ed) = self.editor_win
                    && ed.window == e.window
                    && (ed.width != nw || ed.height != nh)
                {
                    ed.width = nw.max(780);
                    ed.height = nh.max(500);
                    ed.dirty = true;
                }
            }
            X11Event::ClientMessage(e) => {
                if e.type_ == self.atoms.wm_protocols
                    && e.data.as_data32()[0] == self.atoms.wm_delete_window
                {
                    if self
                        .kb_win
                        .as_ref()
                        .is_some_and(|k| k.slot.window == e.window)
                    {
                        self.destroy_keyboard_window();
                    } else if self
                        .settings_win
                        .as_ref()
                        .is_some_and(|s| s.window == e.window)
                    {
                        self.destroy_settings_window();
                    } else if self
                        .editor_win
                        .as_ref()
                        .is_some_and(|ed| ed.window == e.window)
                    {
                        self.destroy_editor_window();
                    }
                }
            }
            X11Event::ButtonPress(e) => {
                let p = Point::new(f32::from(e.event_x), f32::from(e.event_y));
                let right = e.detail == 3;
                let left = e.detail == 1;
                if !left && !right {
                    return;
                }
                if self
                    .kb_win
                    .as_ref()
                    .is_some_and(|k| k.slot.window == e.event)
                {
                    self.handle_kb_press(p, i32::from(e.root_x), i32::from(e.root_y), right);
                } else if left
                    && self
                        .settings_win
                        .as_ref()
                        .is_some_and(|s| s.window == e.event)
                {
                    if let Some(dir) = detect_x11_resize_dir(
                        p,
                        self.settings_view.width,
                        self.settings_view.height,
                    ) {
                        self.send_wm_moveresize(e.event, e.root_x, e.root_y, dir);
                        return;
                    }
                    let action =
                        self.settings_view
                            .pointer_down(p, &mut self.state, &self.preset_store);
                    if action == SettingsAction::StartWindowMove {
                        self.send_wm_moveresize(e.event, e.root_x, e.root_y, MOVERESIZE_MOVE);
                    }
                    self.handle_settings_action(action);
                } else if left
                    && self
                        .editor_win
                        .as_ref()
                        .is_some_and(|ed| ed.window == e.event)
                {
                    if let Some(dir) =
                        detect_x11_resize_dir(p, self.editor_view.width, self.editor_view.height)
                    {
                        self.send_wm_moveresize(e.event, e.root_x, e.root_y, dir);
                        return;
                    }
                    let action =
                        self.editor_view
                            .pointer_down(p, &mut self.state, &self.panel_store);
                    if action == PanelEditorAction::StartWindowMove {
                        self.send_wm_moveresize(e.event, e.root_x, e.root_y, MOVERESIZE_MOVE);
                    }
                    self.handle_editor_action(action);
                }
            }
            X11Event::ButtonRelease(e) => {
                if e.detail != 1 && e.detail != 3 {
                    return;
                }
                if self
                    .kb_win
                    .as_ref()
                    .is_some_and(|k| k.slot.window == e.event)
                {
                    if self.kb_drag.take().is_some() {
                        self.mark_config_dirty();
                    }
                    self.repeat = None;
                    let cmds = self
                        .state
                        .release_contact(ContactId::Pointer, Instant::now());
                    self.input.execute(&cmds);
                    self.mark_keyboard_dirty();
                } else if self
                    .settings_win
                    .as_ref()
                    .is_some_and(|s| s.window == e.event)
                {
                    self.settings_view.pointer_up();
                } else if self
                    .editor_win
                    .as_ref()
                    .is_some_and(|ed| ed.window == e.event)
                {
                    let action = self
                        .editor_view
                        .pointer_up(&mut self.state, &self.panel_store);
                    self.handle_editor_action(action);
                }
            }
            X11Event::MotionNotify(e) => {
                let p = Point::new(f32::from(e.event_x), f32::from(e.event_y));
                if self
                    .kb_win
                    .as_ref()
                    .is_some_and(|k| k.slot.window == e.event)
                {
                    self.handle_kb_motion(p, i32::from(e.root_x), i32::from(e.root_y));
                } else if self
                    .settings_win
                    .as_ref()
                    .is_some_and(|s| s.window == e.event)
                {
                    let action = self.settings_view.pointer_motion(p, &mut self.state);
                    self.handle_settings_action(action);
                } else if self
                    .editor_win
                    .as_ref()
                    .is_some_and(|ed| ed.window == e.event)
                {
                    let action = self.editor_view.pointer_motion(p);
                    self.handle_editor_action(action);
                }
            }
            X11Event::LeaveNotify(e) => {
                if self
                    .kb_win
                    .as_ref()
                    .is_some_and(|k| k.slot.window == e.event)
                    && self.kb_drag.is_none()
                {
                    self.state.hovered_key_id = None;
                    self.hovered_toolbar = None;
                    self.repeat = None;
                    let cmds = self
                        .state
                        .release_contact(ContactId::Pointer, Instant::now());
                    self.input.execute(&cmds);
                    self.mark_keyboard_dirty();
                }
            }
            X11Event::KeyPress(e) => {
                // On Linux Xorg & XWayland, X11 keycode = evdev keycode + 8.
                let kc = KeyCode(u16::from(e.detail).saturating_sub(8));
                let shift = e.state.contains(KeyButMask::SHIFT);
                let ctrl = e.state.contains(KeyButMask::CONTROL);
                let clip = if ctrl && kc == KeyCode::V {
                    self.clipboard.paste()
                } else {
                    None
                };

                if self
                    .editor_win
                    .as_ref()
                    .is_some_and(|ed| ed.window == e.event)
                {
                    let action = self.editor_view.handle_key_input(
                        kc,
                        shift,
                        ctrl,
                        clip.as_deref(),
                        &mut self.state,
                        &self.panel_store,
                    );
                    self.handle_editor_action(action);
                } else if self
                    .settings_win
                    .as_ref()
                    .is_some_and(|s| s.window == e.event)
                {
                    let action = self.settings_view.handle_key_input(
                        kc,
                        shift,
                        ctrl,
                        clip.as_deref(),
                        &mut self.state,
                        &self.preset_store,
                    );
                    self.handle_settings_action(action);
                }
            }
            _ => {}
        }
    }

    fn handle_kb_press(&mut self, p: Point, root_x: i32, root_y: i32, right_click: bool) {
        let hit = self.kb_win.as_ref().and_then(|kb| kb.layout.hit_test(p));
        let Some(hit) = hit else {
            return;
        };

        match hit {
            KeyboardHit::Resize(edge) => {
                self.kb_drag = Some(X11KbDrag::Resize {
                    edge,
                    start_root_x: root_x,
                    start_root_y: root_y,
                    start_geom: self.state.config.window,
                });
            }
            KeyboardHit::DragHeader => {
                self.kb_drag = Some(X11KbDrag::Move {
                    start_root_x: root_x,
                    start_root_y: root_y,
                    start_geom: self.state.config.window,
                });
            }
            KeyboardHit::ToolbarEsc => {
                let esc = KeyboardViewLayout::esc_key_spec();
                let cmds = self.state.press_grid_key(ContactId::Pointer, &esc, false);
                self.input.execute(&cmds);
                self.mark_keyboard_dirty();
            }
            KeyboardHit::ToolbarLang(code) => {
                self.apply_layout_change(&code, true);
            }
            KeyboardHit::ToolbarPanelCycle => {
                if right_click {
                    self.create_editor_window();
                } else {
                    self.state.cycle_panel();
                    self.sync_tray_state();
                    self.mark_config_dirty();
                    self.mark_keyboard_dirty();
                    self.mark_settings_dirty();
                    self.mark_editor_dirty();
                }
            }
            KeyboardHit::ToolbarOptions => {
                if self.settings_win.is_some() {
                    self.destroy_settings_window();
                } else {
                    self.create_settings_window();
                }
            }
            KeyboardHit::ToolbarHide => {
                self.destroy_keyboard_window();
            }
            KeyboardHit::Key(idx) => {
                let resolved = self
                    .kb_win
                    .as_ref()
                    .and_then(|kb| kb.layout.keys.get(idx))
                    .map(|k| k.resolved.clone());
                if let Some(resolved) = resolved {
                    if let GridKeyAction::LayoutKey { keycode: kc }
                    | GridKeyAction::FixedKey { keycode: kc } = resolved.grid.action
                        && !matches!(resolved.grid.role, KeyRole::Modifier(_))
                    {
                        let shift = self.state.modifiers.effective_uppercase() || right_click;
                        let ctrl = self
                            .state
                            .modifiers
                            .is_active(wlboard_core::ModifierKind::Ctrl);
                        let clip = if ctrl && kc == KeyCode::V {
                            self.clipboard.paste()
                        } else {
                            None
                        };
                        if self.settings_win.is_some() && self.settings_view.focused_field.is_some()
                        {
                            let action = self.settings_view.handle_key_input(
                                kc,
                                shift,
                                ctrl,
                                clip.as_deref(),
                                &mut self.state,
                                &self.preset_store,
                            );
                            self.handle_settings_action(action);
                            return;
                        }
                        if self.editor_win.is_some() && self.editor_view.focused_field.is_some() {
                            let action = self.editor_view.handle_key_input(
                                kc,
                                shift,
                                ctrl,
                                clip.as_deref(),
                                &mut self.state,
                                &self.panel_store,
                            );
                            self.handle_editor_action(action);
                            return;
                        }
                    }

                    let cmds =
                        self.state
                            .press_grid_key(ContactId::Pointer, &resolved, right_click);
                    self.input.execute(&cmds);

                    if let GridKeyAction::LayoutKey { keycode: kc }
                    | GridKeyAction::FixedKey { keycode: kc } = resolved.grid.action
                        && !matches!(resolved.grid.role, KeyRole::Modifier(_))
                    {
                        self.repeat = Some(RepeatState {
                            keycode: kc,
                            next_fire: Instant::now() + REPEAT_INITIAL_DELAY,
                        });
                    }
                    self.sync_tray_state();
                    self.mark_keyboard_dirty();
                }
            }
        }
    }

    fn handle_kb_motion(&mut self, p: Point, root_x: i32, root_y: i32) {
        if let Some(drag) = self.kb_drag {
            match drag {
                X11KbDrag::Move {
                    start_root_x,
                    start_root_y,
                    start_geom,
                } => {
                    let dx = root_x - start_root_x;
                    let dy = root_y - start_root_y;
                    let next = clamp_move(start_geom, dx, dy, self.output_bounds);
                    if next != self.state.config.window {
                        self.state.config.window = next;
                        if let Some(ref kb) = self.kb_win {
                            let _ = self.conn.configure_window(
                                kb.slot.window,
                                &ConfigureWindowAux::new().x(next.x).y(next.y),
                            );
                        }
                        self.mark_config_dirty();
                    }
                    return;
                }
                X11KbDrag::Resize {
                    edge,
                    start_root_x,
                    start_root_y,
                    start_geom,
                } => {
                    let dx = root_x - start_root_x;
                    let dy = root_y - start_root_y;
                    let next = apply_resize(start_geom, edge, dx, dy, self.output_bounds);
                    if next != self.state.config.window {
                        self.state.config.window = next;
                        if let Some(ref mut kb) = self.kb_win {
                            kb.slot.width = next.width;
                            kb.slot.height = next.height;
                            kb.slot.dirty = true;
                            let _ = self.conn.configure_window(
                                kb.slot.window,
                                &ConfigureWindowAux::new()
                                    .x(next.x)
                                    .y(next.y)
                                    .width(next.width)
                                    .height(next.height),
                            );
                        }
                        self.mark_config_dirty();
                    }
                    return;
                }
            }
        }

        let hit = self.kb_win.as_ref().and_then(|kb| kb.layout.hit_test(p));
        let new_hover_key = match &hit {
            Some(KeyboardHit::Key(idx)) => self
                .kb_win
                .as_ref()
                .and_then(|kb| kb.layout.keys.get(*idx))
                .map(|k| k.resolved.grid.id.clone()),
            _ => None,
        };
        let new_hover_tb = match hit {
            Some(
                h @ (KeyboardHit::ToolbarEsc
                | KeyboardHit::ToolbarLang(_)
                | KeyboardHit::ToolbarPanelCycle
                | KeyboardHit::ToolbarOptions
                | KeyboardHit::ToolbarHide),
            ) => Some(h),
            _ => None,
        };
        if new_hover_key != self.state.hovered_key_id || new_hover_tb != self.hovered_toolbar {
            self.state.hovered_key_id = new_hover_key;
            self.hovered_toolbar = new_hover_tb;
            self.mark_keyboard_dirty();
        }
    }

    fn handle_platform_event(&mut self, ev: PlatformEvent) {
        match ev {
            PlatformEvent::ToggleVisible => {
                if self.kb_win.is_some() {
                    self.destroy_keyboard_window();
                } else {
                    self.create_keyboard_window();
                }
            }
            PlatformEvent::Show => self.create_keyboard_window(),
            PlatformEvent::Hide => self.destroy_keyboard_window(),
            PlatformEvent::OpenSettings => self.create_settings_window(),
            PlatformEvent::OpenPanelEditor => self.create_editor_window(),
            PlatformEvent::ToggleCompactStyle => {
                let next = match self.state.config.layout.style {
                    KeyboardStyle::Standard => KeyboardStyle::CompactTv,
                    KeyboardStyle::CompactTv => KeyboardStyle::Standard,
                };
                self.state.set_style(next);
                self.sync_tray_state();
                self.mark_config_dirty();
                self.mark_keyboard_dirty();
                self.mark_settings_dirty();
                self.mark_editor_dirty();
            }
            PlatformEvent::SetLayout(code) => self.apply_layout_change(&code, true),
            PlatformEvent::SetPanel(pid) => {
                if self.state.set_panel(&pid) {
                    self.sync_tray_state();
                    self.mark_config_dirty();
                    self.mark_keyboard_dirty();
                    self.mark_settings_dirty();
                    self.mark_editor_dirty();
                }
            }
            PlatformEvent::ExternalLayoutChanged(code) => {
                if self.state.config.layout.system_layout_sync {
                    self.apply_layout_change(&code, false);
                }
            }
            PlatformEvent::LayoutSyncStatus(status) => {
                self.state.layout_sync_status = status;
                self.mark_settings_dirty();
            }
            PlatformEvent::SystemActionFinished(msg) => {
                self.settings_view.status_message = Some(msg);
                self.settings_view.status_folder = None;
                self.settings_view.refresh_system_status();
                self.mark_settings_dirty();
            }
            PlatformEvent::Quit => {
                self.running = false;
            }
        }
        self.redraw_dirty();
        let _ = self.conn.flush();
    }

    fn handle_settings_action(&mut self, action: SettingsAction) {
        match action {
            SettingsAction::None | SettingsAction::StartWindowMove => {}
            SettingsAction::Redraw => self.mark_settings_dirty(),
            SettingsAction::ConfigChanged => {
                self.mark_config_dirty();
                self.sync_tray_state();
                self.mark_keyboard_dirty();
                self.mark_settings_dirty();
                self.mark_editor_dirty();
            }
            SettingsAction::LayoutChanged(code) => self.apply_layout_change(&code, true),
            SettingsAction::CopyText(txt) => {
                self.clipboard.copy(&txt);
                self.mark_settings_dirty();
            }
            SettingsAction::OpenFolder(dir) => {
                if let Err(err) = open_directory_in_file_manager(&dir) {
                    self.settings_view.status_message = Some(format!("Open folder failed: {err}"));
                }
                self.mark_settings_dirty();
            }
            SettingsAction::OpenPanelEditor => self.create_editor_window(),
            SettingsAction::EnableKdeLockscreen => {
                self.settings_view.status_message = Some(
                    enable_kde_lockscreen_support()
                        .unwrap_or_else(|e| format!("Setup failed: {e}")),
                );
                self.settings_view.status_folder = None;
                self.settings_view.refresh_system_status();
                self.mark_settings_dirty();
            }
            SettingsAction::DisableKdeLockscreen => {
                self.settings_view.status_message = Some(
                    disable_kde_lockscreen_support()
                        .unwrap_or_else(|e| format!("Disable failed: {e}")),
                );
                self.settings_view.status_folder = None;
                self.settings_view.refresh_system_status();
                self.mark_settings_dirty();
            }
            SettingsAction::DisableKdeMouseOverride => {
                self.settings_view.status_message = Some(
                    disable_kde_lockscreen_mouse_override()
                        .unwrap_or_else(|e| format!("Update failed: {e}")),
                );
                self.settings_view.status_folder = None;
                self.settings_view.refresh_system_status();
                self.mark_settings_dirty();
            }
            SettingsAction::EnableSddm => {
                self.settings_view.status_message =
                    Some("Requesting administrator permission for SDDM setup...".to_owned());
                self.settings_view.status_folder = None;
                self.platform.enable_sddm_gui();
                self.mark_settings_dirty();
            }
            SettingsAction::DisableSddm => {
                self.settings_view.status_message =
                    Some("Requesting administrator permission to disable SDDM...".to_owned());
                self.settings_view.status_folder = None;
                self.platform.disable_sddm_gui();
                self.mark_settings_dirty();
            }
            SettingsAction::CloseSettings => self.destroy_settings_window(),
            SettingsAction::QuitApp => self.running = false,
        }
    }

    fn handle_editor_action(&mut self, action: PanelEditorAction) {
        match action {
            PanelEditorAction::None | PanelEditorAction::StartWindowMove => {}
            PanelEditorAction::Redraw => self.mark_editor_dirty(),
            PanelEditorAction::PanelsChanged => {
                self.mark_config_dirty();
                self.sync_tray_state();
                self.mark_keyboard_dirty();
                self.mark_settings_dirty();
                self.mark_editor_dirty();
            }
            PanelEditorAction::CopyText(txt) => {
                self.clipboard.copy(&txt);
                self.mark_editor_dirty();
            }
            PanelEditorAction::OpenFolder(dir) => {
                if let Err(err) = open_directory_in_file_manager(&dir) {
                    self.editor_view.status_message = Some(format!("Open folder failed: {err}"));
                }
                self.mark_editor_dirty();
            }
            PanelEditorAction::CloseEditor => self.destroy_editor_window(),
        }
    }

    fn apply_layout_change(&mut self, code: &str, push_to_system: bool) {
        if self.state.set_layout(code) {
            self.input.apply_layout(self.state.active_layout());
            if push_to_system && self.state.config.layout.system_layout_sync {
                self.platform.sync_system_layout(
                    self.state.config.layout.active.clone(),
                    self.state.config.layout.auto_register_kde_layouts,
                );
            }
            self.sync_tray_state();
            self.mark_config_dirty();
            self.mark_keyboard_dirty();
            self.mark_settings_dirty();
            self.mark_editor_dirty();
        }
    }

    fn sync_tray_state(&self) {
        self.platform.update_tray(
            self.state.visible,
            self.state.config.layout.style,
            self.state.config.layout.active.clone(),
            self.state.config.layout.active_panel.clone(),
            self.state
                .panels
                .iter()
                .map(|p| (p.id.clone(), p.name.clone()))
                .collect(),
            self.state
                .visible_layouts()
                .into_iter()
                .map(|l| (l.code.clone(), l.display_name.clone()))
                .collect(),
        );
    }

    fn mark_keyboard_dirty(&mut self) {
        if let Some(ref mut kb) = self.kb_win {
            kb.slot.dirty = true;
        }
    }

    fn mark_settings_dirty(&mut self) {
        if let Some(ref mut s) = self.settings_win {
            s.dirty = true;
        }
    }

    fn mark_editor_dirty(&mut self) {
        if let Some(ref mut e) = self.editor_win {
            e.dirty = true;
        }
    }

    fn mark_config_dirty(&mut self) {
        self.config_dirty_since = Some(Instant::now());
    }

    fn save_config_now(&mut self) {
        self.config_dirty_since = None;
        if let Err(err) = self.state.config.save_default() {
            warn!("Failed saving config.toml: {err}");
        }
    }

    fn release_all_keys(&mut self) {
        self.repeat = None;
        let cmds = self.state.release_all();
        self.input.execute(&cmds);
    }
}

fn detect_x11_resize_dir(p: Point, w: f32, h: f32) -> Option<u32> {
    let b = TOPLEVEL_RESIZE_BORDER;
    let left = p.x <= b;
    let right = p.x >= w - b;
    let top = p.y <= b;
    let bottom = p.y >= h - b;
    match (top, bottom, left, right) {
        (true, _, true, _) => Some(MOVERESIZE_SIZE_TOPLEFT),
        (true, _, _, true) => Some(MOVERESIZE_SIZE_TOPRIGHT),
        (_, true, true, _) => Some(MOVERESIZE_SIZE_BOTTOMLEFT),
        (_, true, _, true) => Some(MOVERESIZE_SIZE_BOTTOMRIGHT),
        (true, _, _, _) => Some(MOVERESIZE_SIZE_TOP),
        (_, true, _, _) => Some(MOVERESIZE_SIZE_BOTTOM),
        (_, _, true, _) => Some(MOVERESIZE_SIZE_LEFT),
        (_, _, _, true) => Some(MOVERESIZE_SIZE_RIGHT),
        _ => None,
    }
}
