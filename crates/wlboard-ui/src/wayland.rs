//! Native Wayland client runtime using `smithay-client-toolkit` + `calloop`:
//! - Dual Mode:
//!   1. Floating draggable & 8-edge resizable `zwlr_layer_shell_v1` overlay (`Layer::Overlay`, `KeyboardInteractivity::None`)
//!   2. Wayland Input Method mode (`--input-method`: `zwp_input_method_v1` + `zwp_input_panel_v1`) for KDE Plasma Lockscreen & SDDM Login Screen
//! - Built-in themed `xdg_toplevel` **Customize wlboard** Settings window (draggable + 8-edge resizable)
//! - Built-in themed 3-pane `xdg_toplevel` **Panel Editor** window (`960×590` draggable + 8-edge resizable)
//! - True `wl_touch` multi-touch + `wl_pointer` + raw `wl_keyboard`

use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use calloop::channel::{self, Channel};
use calloop::generic::Generic;
use calloop::timer::{TimeoutAction, Timer};
use calloop::{EventLoop, Interest, LoopHandle, Mode, PostAction, RegistrationToken};
use smithay_client_toolkit::compositor::{
    CompositorHandler, CompositorState, FrameCallbackData, Region,
};
use smithay_client_toolkit::dispatch2::Dispatch2;
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::reexports::protocols::xdg::shell::client::xdg_toplevel::ResizeEdge as XdgResizeEdge;
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::pointer::{
    CursorIcon, PointerEvent, PointerEventKind, PointerHandler, ThemeSpec, ThemedPointer,
};
use smithay_client_toolkit::seat::touch::TouchHandler;
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
    LayerSurfaceConfigure,
};
use smithay_client_toolkit::shell::xdg::XdgShell;
use smithay_client_toolkit::shell::xdg::window::{
    Window, WindowConfigure, WindowDecorations, WindowHandler,
};
use smithay_client_toolkit::shm::slot::SlotPool;
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{delegate_dispatch2, delegate_registry, registry_handlers};
use tiny_skia::Pixmap;
use tracing::{debug, info, warn};
use wayland_client::backend::ObjectData;
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::{
    wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface, wl_touch,
};
use wayland_client::{Connection, Proxy, QueueHandle};
use wayland_protocols::wp::input_method::zv1::client::{
    zwp_input_method_context_v1::{self, ZwpInputMethodContextV1},
    zwp_input_method_v1::{self, ZwpInputMethodV1},
    zwp_input_panel_surface_v1::{self, Position as ImPanelPosition, ZwpInputPanelSurfaceV1},
    zwp_input_panel_v1::ZwpInputPanelV1,
};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};
use wlboard_core::{
    AppConfig, ContactId, GridKeyAction, KeyCode, KeyRole, KeyboardState, KeyboardStyle,
    MIN_WINDOW_HEIGHT, MIN_WINDOW_WIDTH, OutputBounds, PanelStore, Point, PresetStore, ResizeEdge,
    WindowGeometry, apply_resize, clamp_move,
};
use wlboard_platform::{
    InputManager, PlatformEvent, PlatformHandle, TrayState, disable_kde_lockscreen_mouse_override,
    disable_kde_lockscreen_support, enable_kde_lockscreen_support, is_lockscreen_or_greeter_active,
    open_directory_in_file_manager,
};

use crate::canvas::{UiRenderer, copy_pixmap_offset_to_argb8888, copy_pixmap_to_wayland_argb8888};
use crate::clipboard::ClipboardBridge;
use crate::keyboard_view::{KeyboardHit, KeyboardViewLayout};
use crate::panel_editor::{
    PANEL_EDITOR_DEFAULT_HEIGHT, PANEL_EDITOR_DEFAULT_WIDTH, PanelEditorAction, PanelEditorView,
};
use crate::settings_view::{
    SETTINGS_DEFAULT_HEIGHT, SETTINGS_DEFAULT_WIDTH, SettingsAction, SettingsView,
};

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const TOPLEVEL_RESIZE_BORDER: f32 = 7.0;

const REPEAT_INITIAL_DELAY: Duration = Duration::from_millis(420);
const REPEAT_INTERVAL: Duration = Duration::from_millis(55);
const WATCHDOG_CHECK_DELAY: Duration = Duration::from_millis(400);

/// Typed error from `run_wayland_app` enabling seamless XWayland/X11 auto-fallback.
#[derive(Debug)]
pub enum WaylandRunError {
    /// `WAYLAND_DISPLAY` missing or compositor connection refused.
    NoDisplay(String),
    /// Connected to Wayland (e.g. GNOME/Mutter), but `zwlr_layer_shell_v1` is not advertised.
    LayerShellUnavailable,
    /// Fatal runtime error after initialization.
    Runtime(String),
}

impl std::fmt::Display for WaylandRunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoDisplay(e) => write!(f, "{e}"),
            Self::LayerShellUnavailable => {
                write!(
                    f,
                    "zwlr_layer_shell_v1 unavailable on this Wayland compositor"
                )
            }
            Self::Runtime(e) => write!(f, "{e}"),
        }
    }
}

/// Launch configuration for `run_app` / `run_wayland_app`.
#[derive(Debug, Clone, Default)]
pub struct LaunchOptions {
    pub open_settings: bool,
    pub open_panel_editor: bool,
    pub input_method_mode: bool,
    pub force_show: bool,
    pub initial_panel: Option<String>,
}

#[derive(Debug, Clone, Copy)]
enum KeyboardDragMode {
    Move {
        anchor_local: Point,
        pending_frame: bool,
    },
    Resize {
        edge: ResizeEdge,
        anchor_local: Point,
        start_geom: WindowGeometry,
        start_im_offset_x: i32,
        pending_frame: bool,
    },
}

#[derive(Debug, Clone, Copy)]
struct RepeatState {
    keycode: KeyCode,
    next_fire: Instant,
}

enum KeyboardSurfaceKind {
    Layer(LayerSurface),
    InputPanel {
        surface: wl_surface::WlSurface,
        panel_surface: ZwpInputPanelSurfaceV1,
    },
}

impl KeyboardSurfaceKind {
    fn wl_surface(&self) -> &wl_surface::WlSurface {
        match self {
            Self::Layer(l) => l.wl_surface(),
            Self::InputPanel { surface, .. } => surface,
        }
    }
}

struct KeyboardSurfaceState {
    kind: KeyboardSurfaceKind,
    configured: bool,
    scale: i32,
    dirty: bool,
    layout: KeyboardViewLayout,
}

struct ToplevelSurfaceState {
    window: Window,
    configured: bool,
    scale: i32,
    dirty: bool,
}

struct WlboardRuntime {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    compositor_state: CompositorState,
    shm: Shm,
    layer_shell: Option<LayerShell>,
    xdg_shell: Option<XdgShell>,
    pool: Option<SlotPool>,

    pointer: Option<ThemedPointer>,
    touch: Option<wl_touch::WlTouch>,
    wl_keyboard: Option<wl_keyboard::WlKeyboard>,
    primary_seat: Option<wl_seat::WlSeat>,
    primary_output: Option<wl_output::WlOutput>,

    vk_manager: Option<ZwpVirtualKeyboardManagerV1>,
    input_method_global: Option<ZwpInputMethodV1>,
    input_panel_global: Option<ZwpInputPanelV1>,
    input_method_mode: bool,
    pending_im_show: bool,
    im_offset_x: i32,
    im_lift_y: i32,

    input: InputManager,
    input_initialized: bool,

    renderer: UiRenderer,
    clipboard: ClipboardBridge,
    state: KeyboardState,
    preset_store: PresetStore,
    panel_store: PanelStore,
    platform: PlatformHandle,
    output_bounds: OutputBounds,

    kb_surface: Option<KeyboardSurfaceState>,
    settings_surface: Option<ToplevelSurfaceState>,
    settings_view: SettingsView,
    editor_surface: Option<ToplevelSurfaceState>,
    editor_view: PanelEditorView,

    hovered_toolbar: Option<KeyboardHit>,
    kb_drag: Option<KeyboardDragMode>,
    repeat: Option<RepeatState>,
    physical_shift_held: bool,
    physical_ctrl_held: bool,
    config_dirty_since: Option<Instant>,
    watchdog_deadline: Option<Instant>,
    armed_deadline: Option<Instant>,
    timer_token: Option<RegistrationToken>,
    loop_handle: Option<LoopHandle<'static, Self>>,
    running: bool,
    shutdown_flag: Arc<AtomicBool>,
}

/// Launch the native Wayland `wlboard` runtime and block with zero idle wakeups until shutdown.
pub fn run_wayland_app(
    options: LaunchOptions,
    shutdown_flag: Arc<AtomicBool>,
    signal_pipe: Option<UnixStream>,
) -> Result<(), WaylandRunError> {
    let conn = Connection::connect_to_env().map_err(|e| {
        WaylandRunError::NoDisplay(format!(
            "Could not connect to Wayland display (is WAYLAND_DISPLAY set?): {e}"
        ))
    })?;

    let (globals, mut event_queue) = registry_queue_init::<WlboardRuntime>(&conn).map_err(|e| {
        WaylandRunError::Runtime(format!("Failed to initialize Wayland registry: {e}"))
    })?;
    let qh = event_queue.handle();

    let registry_state = RegistryState::new(&globals);
    let seat_state = SeatState::new(&globals, &qh);
    let output_state = OutputState::new(&globals, &qh);
    let compositor_state = CompositorState::bind(&globals, &qh)
        .map_err(|e| WaylandRunError::Runtime(format!("wl_compositor unavailable: {e}")))?;
    let shm = Shm::bind(&globals, &qh)
        .map_err(|e| WaylandRunError::Runtime(format!("wl_shm unavailable: {e}")))?;

    let layer_shell = LayerShell::bind(&globals, &qh).ok();
    let xdg_shell = XdgShell::bind(&globals, &qh).ok();

    let input_method_global = if options.input_method_mode {
        globals
            .bind::<ZwpInputMethodV1, _, _>(&qh, 1..=1, InputMethodData)
            .ok()
    } else {
        None
    };
    let input_panel_global = if options.input_method_mode {
        globals
            .bind::<ZwpInputPanelV1, _, _>(&qh, 1..=1, InputMethodData)
            .ok()
    } else {
        None
    };

    let active_im_mode =
        options.input_method_mode && input_method_global.is_some() && input_panel_global.is_some();

    if options.input_method_mode && !active_im_mode {
        warn!(
            "Compositor did not advertise zwp_input_method_v1 / zwp_input_panel_v1 — falling back to floating layer-shell mode"
        );
    }
    if !active_im_mode && layer_shell.is_none() {
        return Err(WaylandRunError::LayerShellUnavailable);
    }

    let vk_manager = if !active_im_mode {
        globals
            .bind::<ZwpVirtualKeyboardManagerV1, _, _>(&qh, 1..=1, VirtualKeyboardData)
            .ok()
    } else {
        None
    };

    let config = AppConfig::load_default();
    let preset_store = PresetStore::default_store();
    let panel_store = PanelStore::default_store();
    let mut kb_state = KeyboardState::new(config, &preset_store, &panel_store);
    kb_state.input_method_mode = active_im_mode;
    if let Some(ref pid) = options.initial_panel {
        kb_state.set_panel(pid);
    }
    if active_im_mode {
        kb_state.visible = false;
    } else if options.force_show {
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
    let platform = PlatformHandle::spawn(
        initial_tray,
        kb_state.registry.clone(),
        !active_im_mode,
        move |ev| {
            let _ = platform_tx.send(ev);
        },
    );

    let input = if active_im_mode {
        InputManager::for_input_method()
    } else {
        InputManager::auto_select(None, kb_state.active_layout())
    };
    kb_state.input_backend_status = input.status_detail();

    let mut settings_view = SettingsView::new(&kb_state.config.appearance);
    settings_view.display_backend_label = if active_im_mode {
        "Wayland Input Method (Lockscreen / SDDM)".to_owned()
    } else {
        "Wayland (wlr-layer-shell)".to_owned()
    };
    let editor_view = PanelEditorView::new(&kb_state);

    let mut runtime = WlboardRuntime {
        registry_state,
        seat_state,
        output_state,
        compositor_state,
        shm,
        layer_shell,
        xdg_shell,
        pool: None,
        pointer: None,
        touch: None,
        wl_keyboard: None,
        primary_seat: None,
        primary_output: None,
        vk_manager,
        input_method_global,
        input_panel_global,
        input_method_mode: active_im_mode,
        pending_im_show: false,
        im_offset_x: 0,
        im_lift_y: 0,
        input,
        input_initialized: active_im_mode,
        renderer: UiRenderer::new(),
        clipboard: ClipboardBridge::new(),
        state: kb_state,
        preset_store,
        panel_store,
        platform,
        output_bounds: OutputBounds::default(),
        kb_surface: None,
        settings_surface: None,
        settings_view,
        editor_surface: None,
        editor_view,
        hovered_toolbar: None,
        kb_drag: None,
        repeat: None,
        physical_shift_held: false,
        physical_ctrl_held: false,
        config_dirty_since: None,
        watchdog_deadline: None,
        armed_deadline: None,
        timer_token: None,
        loop_handle: None,
        running: true,
        shutdown_flag,
    };

    // Perform an initial Wayland roundtrip so `wl_output` dimensions and `wl_seat` globals
    // are populated BEFORE creating surfaces or binding seats.
    let _ = event_queue.roundtrip(&mut runtime);

    // SCTK `SeatState::new` binds pre-existing `wl_seat` globals synchronously without invoking
    // `SeatHandler::new_seat`. Capture the pre-existing seat immediately so `xdg_toplevel` move/resize
    // and `zwp_virtual_keyboard_v1` work from the very first frame!
    runtime.ensure_primary_seat(&qh);

    if !active_im_mode {
        runtime
            .state
            .config
            .window
            .clamp_to_output(runtime.output_bounds);
        if runtime.state.visible {
            runtime.create_keyboard_surface(&qh);
        }
        if runtime.state.settings_open {
            runtime.create_settings_surface(&qh);
        }
        if runtime.state.panel_editor_open {
            runtime.create_editor_surface(&qh);
        }
    } else {
        info!(
            "wlboard listening in Wayland Input Method mode (output {}x{}, awaiting Lockscreen/SDDM activation)",
            runtime.output_bounds.width, runtime.output_bounds.height
        );
    }
    let _ = conn.flush();

    let mut event_loop: EventLoop<WlboardRuntime> = EventLoop::try_new().map_err(|e| {
        WaylandRunError::Runtime(format!("Failed creating calloop event loop: {e}"))
    })?;
    let loop_handle = event_loop.handle();
    runtime.loop_handle = Some(loop_handle.clone());

    WaylandSource::new(conn.clone(), event_queue)
        .insert(loop_handle.clone())
        .map_err(|e| WaylandRunError::Runtime(format!("Failed inserting WaylandSource: {e}")))?;

    let qh_platform = qh.clone();
    loop_handle
        .insert_source(platform_rx, move |event, _, app| {
            if let channel::Event::Msg(ev) = event {
                app.handle_platform_event(ev, &qh_platform);
            }
        })
        .map_err(|e| {
            WaylandRunError::Runtime(format!("Failed registering platform event channel: {e}"))
        })?;

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
        runtime.sync_dynamic_timer();
        let _ = conn.flush();
        if let Err(err) = event_loop.dispatch(None, &mut runtime) {
            warn!("Event loop dispatch error: {err}");
            break;
        }
        let _ = conn.flush();
    }

    info!("Shutting down wlboard cleanly — releasing all virtual keys");
    runtime.release_all_keys();
    runtime.save_config_now();
    runtime.platform.shutdown();
    runtime.input_method_global = None;
    runtime.input_panel_global = None;
    Ok(())
}

impl WlboardRuntime {
    fn ensure_primary_seat(&mut self, qh: &QueueHandle<Self>) {
        if self.primary_seat.is_none()
            && let Some(seat) = self.seat_state.seats().next()
        {
            self.primary_seat = Some(seat.clone());
        }
        if !self.input_initialized
            && let Some(seat) = self.primary_seat.clone()
        {
            self.input_initialized = true;
            if let Some(ref mgr) = self.vk_manager {
                let vk = mgr.create_virtual_keyboard(&seat, qh, VirtualKeyboardData);
                self.input = InputManager::auto_select(Some(vk), self.state.active_layout());
                self.state.input_backend_status = self.input.status_detail();
                self.mark_settings_dirty();
            }
        }
    }

    fn resolve_seat(&mut self, qh: &QueueHandle<Self>) -> Option<wl_seat::WlSeat> {
        if self.primary_seat.is_none() {
            self.ensure_primary_seat(qh);
        }
        self.primary_seat
            .clone()
            .or_else(|| self.seat_state.seats().next())
    }

    /// Arm or disarm the single one-shot `calloop::Timer` strictly when needed
    /// (active key repeat, debounced config save, or held-key stuck watchdog).
    /// When idle, zero timers are armed so `epoll_pwait` generates **0 wakeups/sec**.
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

    fn map_kb_point(&self, p: Point) -> Point {
        if let Some(ref kb) = self.kb_surface
            && matches!(kb.kind, KeyboardSurfaceKind::InputPanel { .. })
        {
            Point::new(p.x - self.im_offset_x as f32, p.y)
        } else {
            p
        }
    }

    fn clamp_floating_keyboard_to_output(&mut self, qh: &QueueHandle<Self>) {
        if self.input_method_mode {
            return;
        }
        let prev = self.state.config.window;
        self.state.config.window.clamp_to_output(self.output_bounds);
        let next = self.state.config.window;
        if next != prev {
            if let Some(ref mut kb) = self.kb_surface
                && let KeyboardSurfaceKind::Layer(ref layer) = kb.kind
            {
                layer.set_size(next.width, next.height);
                layer.set_margin(next.y, 0, 0, next.x);
                kb.dirty = true;
                layer.commit();
            }
            self.mark_config_dirty();
            self.redraw_keyboard_if_needed(qh);
        }
    }

    fn create_keyboard_surface(&mut self, qh: &QueueHandle<Self>) {
        if self.kb_surface.is_some() {
            return;
        }

        if self.input_method_mode
            && let Some(ref panel_mgr) = self.input_panel_global
        {
            let output = self
                .primary_output
                .clone()
                .or_else(|| self.output_state.outputs().next());
            let Some(out) = output else {
                info!("Deferring input-panel creation until wl_output is announced");
                self.pending_im_show = true;
                return;
            };
            self.pending_im_show = false;

            let max_w = self
                .output_bounds
                .width
                .saturating_sub(24)
                .max(MIN_WINDOW_WIDTH);
            let max_h = ((self.output_bounds.height * 48) / 100)
                .max(240)
                .max(MIN_WINDOW_HEIGHT);
            let im_w = self
                .state
                .config
                .window
                .width
                .clamp(MIN_WINDOW_WIDTH, max_w);
            let im_h = self
                .state
                .config
                .window
                .height
                .clamp(MIN_WINDOW_HEIGHT, max_h);
            self.state.config.window.width = im_w;
            self.state.config.window.height = im_h;
            self.im_offset_x = ((self.output_bounds.width as i32 - im_w as i32) / 2).max(0);
            self.im_lift_y = 0;

            let surface = self.compositor_state.create_surface(qh);
            let panel_surface = panel_mgr.get_input_panel_surface(&surface, qh, InputMethodData);
            panel_surface.set_toplevel(&out, ImPanelPosition::CenterBottom as u32);

            let layout = KeyboardViewLayout::compute(&self.state, im_w as f32, im_h as f32);
            self.kb_surface = Some(KeyboardSurfaceState {
                kind: KeyboardSurfaceKind::InputPanel {
                    surface,
                    panel_surface,
                },
                configured: true,
                scale: 1,
                dirty: true,
                layout,
            });
            self.state.visible = true;
            // Attach SHM buffer + click-through input_region and commit ONCE atomically.
            self.redraw_keyboard_if_needed(qh);
            info!("Committed draggable input-panel surface ({im_w}x{im_h}) at CenterBottom");
            return;
        }

        self.state.config.window.clamp_to_output(self.output_bounds);
        let geom = self.state.config.window;

        let Some(ref layer_shell) = self.layer_shell else {
            return;
        };
        let surface = self.compositor_state.create_surface(qh);
        let layer =
            layer_shell.create_layer_surface(qh, surface, Layer::Overlay, Some("wlboard"), None);
        layer.set_anchor(Anchor::TOP | Anchor::LEFT);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_exclusive_zone(-1);
        layer.set_size(geom.width, geom.height);
        layer.set_margin(geom.y, 0, 0, geom.x);
        layer.commit();

        let layout =
            KeyboardViewLayout::compute(&self.state, geom.width as f32, geom.height as f32);
        self.kb_surface = Some(KeyboardSurfaceState {
            kind: KeyboardSurfaceKind::Layer(layer),
            configured: false,
            scale: 1,
            dirty: true,
            layout,
        });
        self.state.visible = true;
        self.sync_tray_state();
    }

    fn release_pool_if_hidden(&mut self) {
        if self.kb_surface.is_none()
            && self.settings_surface.is_none()
            && self.editor_surface.is_none()
        {
            self.pool = None;
        }
    }

    fn destroy_keyboard_surface(&mut self) {
        self.pending_im_show = false;
        self.release_all_keys();
        self.kb_drag = None;
        self.hovered_toolbar = None;
        self.state.hovered_key_id = None;
        if let Some(kb) = self.kb_surface.take()
            && let KeyboardSurfaceKind::InputPanel {
                surface,
                panel_surface,
            } = kb.kind
        {
            surface.attach(None, 0, 0);
            surface.commit();
            drop(panel_surface);
            surface.destroy();
        }
        self.state.visible = false;
        self.release_pool_if_hidden();
        self.sync_tray_state();
    }

    fn create_settings_surface(&mut self, qh: &QueueHandle<Self>) {
        if self.settings_surface.is_some() {
            return;
        }
        let Some(ref xdg_shell) = self.xdg_shell else {
            return;
        };
        self.state.reload_presets(&self.preset_store);
        self.state.reload_panels(&self.panel_store);
        self.settings_view
            .sync_from_appearance(&self.state.config.appearance);
        self.settings_view.refresh_system_status();

        let max_w = self.output_bounds.width.saturating_sub(32).max(640);
        let max_h = self.output_bounds.height.saturating_sub(64).max(540);
        let init_w = SETTINGS_DEFAULT_WIDTH.min(max_w);
        let init_h = SETTINGS_DEFAULT_HEIGHT.min(max_h);

        let surface = self.compositor_state.create_surface(qh);
        let window = xdg_shell.create_window(surface, WindowDecorations::None, qh);
        window.set_title("Customize wlboard");
        window.set_app_id("io.github.olekkut.wlboard");
        window.set_min_size(Some((640, 540)));
        window.commit();

        self.settings_view.width = init_w as f32;
        self.settings_view.height = init_h as f32;
        self.settings_surface = Some(ToplevelSurfaceState {
            window,
            configured: false,
            scale: 1,
            dirty: true,
        });
        self.state.settings_open = true;
        self.mark_keyboard_dirty();
    }

    fn destroy_settings_surface(&mut self) {
        self.settings_surface = None;
        self.state.settings_open = false;
        self.settings_view.focused_field = None;
        self.release_pool_if_hidden();
        self.mark_keyboard_dirty();
    }

    fn create_editor_surface(&mut self, qh: &QueueHandle<Self>) {
        if self.editor_surface.is_some() {
            return;
        }
        let Some(ref xdg_shell) = self.xdg_shell else {
            return;
        };
        self.state.reload_panels(&self.panel_store);
        self.editor_view.load_panel(self.state.active_panel_def());

        let max_w = self.output_bounds.width.saturating_sub(32).max(780);
        let max_h = self.output_bounds.height.saturating_sub(64).max(500);
        let init_w = PANEL_EDITOR_DEFAULT_WIDTH.min(max_w);
        let init_h = PANEL_EDITOR_DEFAULT_HEIGHT.min(max_h);

        let surface = self.compositor_state.create_surface(qh);
        let window = xdg_shell.create_window(surface, WindowDecorations::None, qh);
        window.set_title("wlboard Panel Editor");
        window.set_app_id("io.github.olekkut.wlboard");
        window.set_min_size(Some((780, 500)));
        window.commit();

        self.editor_view.width = init_w as f32;
        self.editor_view.height = init_h as f32;
        self.editor_surface = Some(ToplevelSurfaceState {
            window,
            configured: false,
            scale: 1,
            dirty: true,
        });
        self.state.panel_editor_open = true;
        self.mark_keyboard_dirty();
    }

    fn destroy_editor_surface(&mut self) {
        self.editor_surface = None;
        self.state.panel_editor_open = false;
        self.editor_view.focused_field = None;
        self.release_pool_if_hidden();
        self.mark_keyboard_dirty();
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
        if let Some(ref mut kb) = self.kb_surface {
            kb.dirty = true;
        }
    }

    fn mark_settings_dirty(&mut self) {
        if let Some(ref mut s) = self.settings_surface {
            s.dirty = true;
        }
    }

    fn mark_editor_dirty(&mut self) {
        if let Some(ref mut e) = self.editor_surface {
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

    fn apply_layout_change(&mut self, code: &str, push_to_system: bool) {
        if self.state.set_layout(code) {
            self.input.apply_layout(self.state.active_layout());
            if push_to_system
                && !self.input_method_mode
                && self.state.config.layout.system_layout_sync
            {
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

    fn handle_platform_event(&mut self, ev: PlatformEvent, qh: &QueueHandle<Self>) {
        match ev {
            PlatformEvent::ToggleVisible => {
                if self.kb_surface.is_some() {
                    self.destroy_keyboard_surface();
                } else {
                    self.create_keyboard_surface(qh);
                }
            }
            PlatformEvent::Show => {
                self.create_keyboard_surface(qh);
            }
            PlatformEvent::Hide => {
                self.destroy_keyboard_surface();
            }
            PlatformEvent::OpenSettings => {
                self.create_settings_surface(qh);
            }
            PlatformEvent::OpenPanelEditor => {
                self.create_editor_surface(qh);
            }
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
                self.redraw_dirty(qh);
            }
            PlatformEvent::SetLayout(code) => {
                self.apply_layout_change(&code, true);
                self.redraw_dirty(qh);
            }
            PlatformEvent::SetPanel(pid) => {
                if self.state.set_panel(&pid) {
                    self.sync_tray_state();
                    self.mark_config_dirty();
                    self.mark_keyboard_dirty();
                    self.mark_settings_dirty();
                    self.mark_editor_dirty();
                    self.redraw_dirty(qh);
                }
            }
            PlatformEvent::ExternalLayoutChanged(code) => {
                if self.state.config.layout.system_layout_sync {
                    self.apply_layout_change(&code, false);
                    self.redraw_dirty(qh);
                }
            }
            PlatformEvent::LayoutSyncStatus(status) => {
                self.state.layout_sync_status = status;
                self.mark_settings_dirty();
                self.redraw_dirty(qh);
            }
            PlatformEvent::SystemActionFinished(msg) => {
                self.settings_view.status_message = Some(msg);
                self.settings_view.status_folder = None;
                self.settings_view.refresh_system_status();
                self.mark_settings_dirty();
                self.redraw_dirty(qh);
            }
            PlatformEvent::Quit => {
                self.running = false;
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

    fn redraw_dirty(&mut self, qh: &QueueHandle<Self>) {
        self.redraw_keyboard_if_needed(qh);
        self.redraw_settings_if_needed(qh);
        self.redraw_editor_if_needed(qh);
    }

    fn redraw_keyboard_if_needed(&mut self, _qh: &QueueHandle<Self>) {
        let Some(ref mut kb) = self.kb_surface else {
            return;
        };
        if !kb.configured || !kb.dirty {
            return;
        }
        let geom = self.state.config.window;
        let scale = kb.scale.max(1);
        let kb_phys_w = (geom.width * scale as u32) as i32;
        let kb_phys_h = (geom.height * scale as u32) as i32;

        kb.layout = KeyboardViewLayout::compute(&self.state, geom.width as f32, geom.height as f32);

        let Some(mut pixmap) = Pixmap::new(kb_phys_w as u32, kb_phys_h as u32) else {
            return;
        };
        kb.layout.render(
            &mut self.renderer,
            &mut pixmap,
            &self.state,
            self.hovered_toolbar.as_ref(),
            scale as f32,
        );

        let is_im_panel = matches!(kb.kind, KeyboardSurfaceKind::InputPanel { .. });
        if is_im_panel {
            // Transparent full-width bottom-anchored canvas so the user can drag horizontally and
            // lift upward on Lockscreen/SDDM while all surrounding pixels stay 100% click-through!
            let surf_w = self.output_bounds.width.max(geom.width);
            let max_lift = (self.output_bounds.height / 3).max(40) as i32;
            self.im_lift_y = self.im_lift_y.clamp(0, max_lift);
            self.im_offset_x = self
                .im_offset_x
                .clamp(0, surf_w.saturating_sub(geom.width) as i32);
            let surf_h = geom.height + self.im_lift_y as u32;

            let phys_w = (surf_w * scale as u32) as i32;
            let phys_h = (surf_h * scale as u32) as i32;
            let stride = phys_w * 4;

            let Some(pool) = ensure_shm_pool(
                &mut self.pool,
                &self.shm,
                (stride as usize) * (phys_h as usize) * 2,
            ) else {
                warn!("Failed creating Wayland SHM pool for input-panel keyboard");
                return;
            };
            let Ok((buffer, canvas)) =
                pool.create_buffer(phys_w, phys_h, stride, wl_shm::Format::Argb8888)
            else {
                warn!("Failed allocating Wayland SHM buffer for input-panel keyboard");
                return;
            };
            copy_pixmap_offset_to_argb8888(
                &pixmap,
                canvas,
                phys_w as usize,
                phys_h as usize,
                (self.im_offset_x * scale) as usize,
                0,
            );

            let wl_surf = kb.kind.wl_surface();
            if let Ok(region) = Region::new(&self.compositor_state) {
                region.add(self.im_offset_x, 0, geom.width as i32, geom.height as i32);
                wl_surf.set_input_region(Some(region.wl_region()));
            }
            wl_surf.set_buffer_scale(scale);
            wl_surf.damage_buffer(0, 0, phys_w, phys_h);
            if let Err(err) = buffer.attach_to(wl_surf) {
                warn!("Failed attaching input-panel keyboard buffer: {err}");
                return;
            }
            wl_surf.commit();
            kb.dirty = false;
            return;
        }

        let stride = kb_phys_w * 4;
        let Some(pool) = ensure_shm_pool(
            &mut self.pool,
            &self.shm,
            (stride as usize) * (kb_phys_h as usize) * 2,
        ) else {
            warn!("Failed creating Wayland SHM pool for keyboard");
            return;
        };
        let Ok((buffer, canvas)) =
            pool.create_buffer(kb_phys_w, kb_phys_h, stride, wl_shm::Format::Argb8888)
        else {
            warn!("Failed allocating Wayland SHM buffer for keyboard");
            return;
        };
        copy_pixmap_to_wayland_argb8888(&pixmap, canvas);

        let wl_surf = kb.kind.wl_surface();
        wl_surf.set_buffer_scale(scale);
        wl_surf.damage_buffer(0, 0, kb_phys_w, kb_phys_h);
        if let Err(err) = buffer.attach_to(wl_surf) {
            warn!("Failed attaching keyboard buffer: {err}");
            return;
        }
        wl_surf.commit();
        kb.dirty = false;
    }

    fn redraw_settings_if_needed(&mut self, _qh: &QueueHandle<Self>) {
        let Some(ref mut s) = self.settings_surface else {
            return;
        };
        if !s.configured || !s.dirty {
            return;
        }
        let scale = s.scale.max(1);
        let logical_w = self.settings_view.width.round().max(320.0) as u32;
        let logical_h = self.settings_view.height.round().max(320.0) as u32;
        let phys_w = (logical_w * scale as u32) as i32;
        let phys_h = (logical_h * scale as u32) as i32;
        let stride = phys_w * 4;

        let Some(mut pixmap) = Pixmap::new(phys_w as u32, phys_h as u32) else {
            return;
        };
        self.settings_view
            .render(&mut self.renderer, &mut pixmap, &self.state, scale as f32);

        let Some(pool) = ensure_shm_pool(
            &mut self.pool,
            &self.shm,
            (stride as usize) * (phys_h as usize) * 2,
        ) else {
            warn!("Failed creating Wayland SHM pool for Settings window");
            return;
        };
        let Ok((buffer, canvas)) =
            pool.create_buffer(phys_w, phys_h, stride, wl_shm::Format::Argb8888)
        else {
            warn!("Failed allocating Wayland SHM buffer for Settings window");
            return;
        };
        copy_pixmap_to_wayland_argb8888(&pixmap, canvas);

        let wl_surf = s.window.wl_surface();
        wl_surf.set_buffer_scale(scale);
        wl_surf.damage_buffer(0, 0, phys_w, phys_h);
        if let Err(err) = buffer.attach_to(wl_surf) {
            warn!("Failed attaching Settings buffer: {err}");
            return;
        }
        wl_surf.commit();
        s.dirty = false;
    }

    fn redraw_editor_if_needed(&mut self, _qh: &QueueHandle<Self>) {
        let Some(ref mut e) = self.editor_surface else {
            return;
        };
        if !e.configured || !e.dirty {
            return;
        }
        let scale = e.scale.max(1);
        let logical_w = self.editor_view.width.round().max(640.0) as u32;
        let logical_h = self.editor_view.height.round().max(480.0) as u32;
        let phys_w = (logical_w * scale as u32) as i32;
        let phys_h = (logical_h * scale as u32) as i32;
        let stride = phys_w * 4;

        let Some(mut pixmap) = Pixmap::new(phys_w as u32, phys_h as u32) else {
            return;
        };
        self.editor_view.render(
            &mut self.renderer,
            &mut pixmap,
            &self.state,
            &self.panel_store,
            scale as f32,
        );

        let Some(pool) = ensure_shm_pool(
            &mut self.pool,
            &self.shm,
            (stride as usize) * (phys_h as usize) * 2,
        ) else {
            warn!("Failed creating Wayland SHM pool for Panel Editor");
            return;
        };
        let Ok((buffer, canvas)) =
            pool.create_buffer(phys_w, phys_h, stride, wl_shm::Format::Argb8888)
        else {
            warn!("Failed allocating Wayland SHM buffer for Panel Editor");
            return;
        };
        copy_pixmap_to_wayland_argb8888(&pixmap, canvas);

        let wl_surf = e.window.wl_surface();
        wl_surf.set_buffer_scale(scale);
        wl_surf.damage_buffer(0, 0, phys_w, phys_h);
        if let Err(err) = buffer.attach_to(wl_surf) {
            warn!("Failed attaching Panel Editor buffer: {err}");
            return;
        }
        wl_surf.commit();
        e.dirty = false;
    }

    fn is_keyboard_surface(&self, surface: &wl_surface::WlSurface) -> bool {
        self.kb_surface
            .as_ref()
            .is_some_and(|kb| kb.kind.wl_surface() == surface)
    }

    fn is_settings_surface(&self, surface: &wl_surface::WlSurface) -> bool {
        self.settings_surface
            .as_ref()
            .is_some_and(|s| s.window.wl_surface() == surface)
    }

    fn is_editor_surface(&self, surface: &wl_surface::WlSurface) -> bool {
        self.editor_surface
            .as_ref()
            .is_some_and(|e| e.window.wl_surface() == surface)
    }

    fn handle_kb_press(
        &mut self,
        contact: ContactId,
        raw_p: Point,
        right_click: bool,
        qh: &QueueHandle<Self>,
    ) {
        let p = self.map_kb_point(raw_p);
        let hit = self
            .kb_surface
            .as_ref()
            .and_then(|kb| kb.layout.hit_test(p));
        let Some(hit) = hit else {
            return;
        };

        match hit {
            KeyboardHit::Resize(edge) => {
                if contact == ContactId::Pointer {
                    self.kb_drag = Some(KeyboardDragMode::Resize {
                        edge,
                        anchor_local: p,
                        start_geom: self.state.config.window,
                        start_im_offset_x: self.im_offset_x,
                        pending_frame: false,
                    });
                }
            }
            KeyboardHit::DragHeader => {
                self.kb_drag = Some(KeyboardDragMode::Move {
                    anchor_local: p,
                    pending_frame: false,
                });
            }
            KeyboardHit::ToolbarEsc => {
                let esc = KeyboardViewLayout::esc_key_spec();
                let cmds = self.state.press_grid_key(contact, &esc, false);
                self.input.execute(&cmds);
                self.mark_keyboard_dirty();
            }
            KeyboardHit::ToolbarLang(code) => {
                self.apply_layout_change(&code, true);
            }
            KeyboardHit::ToolbarPanelCycle => {
                if right_click && !self.input_method_mode {
                    self.create_editor_surface(qh);
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
                if !self.input_method_mode {
                    if self.settings_surface.is_some() {
                        self.destroy_settings_surface();
                    } else {
                        self.create_settings_surface(qh);
                    }
                }
            }
            KeyboardHit::ToolbarHide => {
                self.destroy_keyboard_surface();
                return;
            }
            KeyboardHit::Key(idx) => {
                let resolved = self
                    .kb_surface
                    .as_ref()
                    .and_then(|kb| kb.layout.keys.get(idx))
                    .map(|k| k.resolved.clone());
                if let Some(resolved) = resolved {
                    // Allow wlboard itself to type (and Ctrl+C/V/X/A) into focused inline Settings or Panel Editor fields!
                    if let GridKeyAction::LayoutKey { keycode: kc }
                    | GridKeyAction::FixedKey { keycode: kc } = resolved.grid.action
                        && !matches!(resolved.grid.role, KeyRole::Modifier(_))
                    {
                        let shift = self.state.modifiers.effective_uppercase() || right_click;
                        let ctrl = self
                            .state
                            .modifiers
                            .is_active(wlboard_core::ModifierKind::Ctrl)
                            || self.physical_ctrl_held;
                        let clip = if ctrl && kc == KeyCode::V {
                            self.clipboard.paste()
                        } else {
                            None
                        };
                        if self.settings_surface.is_some()
                            && self.settings_view.focused_field.is_some()
                        {
                            let action = self.settings_view.handle_key_input(
                                kc,
                                shift,
                                ctrl,
                                clip.as_deref(),
                                &mut self.state,
                                &self.preset_store,
                            );
                            self.handle_settings_action(action, qh);
                            return;
                        }
                        if self.editor_surface.is_some() && self.editor_view.focused_field.is_some()
                        {
                            let action = self.editor_view.handle_key_input(
                                kc,
                                shift,
                                ctrl,
                                clip.as_deref(),
                                &mut self.state,
                                &self.panel_store,
                            );
                            self.handle_editor_action(action, qh);
                            return;
                        }
                    }

                    let cmds = self.state.press_grid_key(contact, &resolved, right_click);
                    self.input.execute(&cmds);

                    if let GridKeyAction::LayoutKey { keycode: kc }
                    | GridKeyAction::FixedKey { keycode: kc } = resolved.grid.action
                        && !matches!(resolved.grid.role, KeyRole::Modifier(_))
                        && (!self.input_method_mode || resolved.grid.role != KeyRole::Character)
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
        self.redraw_dirty(qh);
    }

    fn handle_kb_release(&mut self, contact: ContactId, qh: &QueueHandle<Self>) {
        if self.kb_drag.take().is_some() {
            self.mark_config_dirty();
        }
        self.repeat = None;
        let cmds = self.state.release_contact(contact, Instant::now());
        self.input.execute(&cmds);
        self.mark_keyboard_dirty();
        self.redraw_dirty(qh);
    }

    fn handle_kb_motion(&mut self, raw_p: Point, conn: &Connection, qh: &QueueHandle<Self>) {
        let p = self.map_kb_point(raw_p);
        if let Some(drag) = self.kb_drag {
            match drag {
                KeyboardDragMode::Move {
                    anchor_local,
                    pending_frame,
                } => {
                    if pending_frame {
                        return;
                    }
                    let dx = (p.x - anchor_local.x).round() as i32;
                    let dy = (p.y - anchor_local.y).round() as i32;
                    if dx != 0 || dy != 0 {
                        if self.input_method_mode {
                            let max_x = self
                                .output_bounds
                                .width
                                .saturating_sub(self.state.config.window.width)
                                as i32;
                            let max_lift = (self.output_bounds.height / 3).max(40) as i32;
                            let next_x = (self.im_offset_x + dx).clamp(0, max_x.max(0));
                            let next_lift = (self.im_lift_y - dy).clamp(0, max_lift);
                            if next_x != self.im_offset_x || next_lift != self.im_lift_y {
                                self.im_offset_x = next_x;
                                self.im_lift_y = next_lift;
                                if let Some(ref mut kb) = self.kb_surface {
                                    let surf = kb.kind.wl_surface().clone();
                                    surf.frame(qh, FrameCallbackData(surf.clone()));
                                    kb.dirty = true;
                                }
                                self.kb_drag = Some(KeyboardDragMode::Move {
                                    anchor_local,
                                    pending_frame: true,
                                });
                                self.redraw_keyboard_if_needed(qh);
                            }
                            return;
                        }

                        let next = clamp_move(self.state.config.window, dx, dy, self.output_bounds);
                        self.state.config.window = next;
                        if let Some(ref kb) = self.kb_surface
                            && let KeyboardSurfaceKind::Layer(ref layer) = kb.kind
                        {
                            layer.set_margin(next.y, 0, 0, next.x);
                            let surf = layer.wl_surface();
                            surf.frame(qh, FrameCallbackData(surf.clone()));
                            layer.commit();
                        }
                        self.kb_drag = Some(KeyboardDragMode::Move {
                            anchor_local,
                            pending_frame: true,
                        });
                        self.mark_config_dirty();
                    }
                    return;
                }
                KeyboardDragMode::Resize {
                    edge,
                    anchor_local,
                    start_geom,
                    start_im_offset_x,
                    pending_frame,
                } => {
                    if pending_frame && (edge.moves_left() || edge.moves_top()) {
                        return;
                    }
                    let dx = (p.x - anchor_local.x).round() as i32;
                    let dy = (p.y - anchor_local.y).round() as i32;

                    if self.input_method_mode {
                        let max_w = self
                            .output_bounds
                            .width
                            .saturating_sub(16)
                            .max(MIN_WINDOW_WIDTH);
                        let max_h = ((self.output_bounds.height * 52) / 100)
                            .max(240)
                            .max(MIN_WINDOW_HEIGHT);
                        let mut new_w = start_geom.width as i32;
                        let mut new_h = start_geom.height as i32;
                        let mut new_x = start_im_offset_x;
                        if edge.moves_right() {
                            new_w = (start_geom.width as i32 + dx)
                                .clamp(MIN_WINDOW_WIDTH as i32, max_w as i32);
                        } else if edge.moves_left() {
                            let cur_w = self.state.config.window.width as i32;
                            new_w = (cur_w - dx).clamp(MIN_WINDOW_WIDTH as i32, max_w as i32);
                            let applied_dx = cur_w - new_w;
                            new_x = (self.im_offset_x + applied_dx).clamp(
                                0,
                                self.output_bounds.width.saturating_sub(new_w as u32) as i32,
                            );
                        }
                        if edge.moves_bottom() {
                            new_h = (start_geom.height as i32 + dy)
                                .clamp(MIN_WINDOW_HEIGHT as i32, max_h as i32);
                        } else if edge.moves_top() {
                            let cur_h = self.state.config.window.height as i32;
                            new_h = (cur_h - dy).clamp(MIN_WINDOW_HEIGHT as i32, max_h as i32);
                        }
                        if new_w as u32 != self.state.config.window.width
                            || new_h as u32 != self.state.config.window.height
                            || new_x != self.im_offset_x
                        {
                            self.state.config.window.width = new_w as u32;
                            self.state.config.window.height = new_h as u32;
                            self.im_offset_x = new_x;
                            if let Some(ref mut kb) = self.kb_surface {
                                let surf = kb.kind.wl_surface().clone();
                                surf.frame(qh, FrameCallbackData(surf.clone()));
                                kb.dirty = true;
                            }
                            self.kb_drag = Some(KeyboardDragMode::Resize {
                                edge,
                                anchor_local,
                                start_geom,
                                start_im_offset_x,
                                pending_frame: true,
                            });
                            self.mark_config_dirty();
                            self.redraw_keyboard_if_needed(qh);
                        }
                        return;
                    }

                    let base = if edge.moves_left() || edge.moves_top() {
                        self.state.config.window
                    } else {
                        start_geom
                    };
                    let next = apply_resize(base, edge, dx, dy, self.output_bounds);
                    if next != self.state.config.window {
                        self.state.config.window = next;
                        if let Some(ref mut kb) = self.kb_surface
                            && let KeyboardSurfaceKind::Layer(ref layer) = kb.kind
                        {
                            layer.set_size(next.width, next.height);
                            layer.set_margin(next.y, 0, 0, next.x);
                            let surf = layer.wl_surface();
                            surf.frame(qh, FrameCallbackData(surf.clone()));
                            kb.dirty = true;
                            layer.commit();
                        }
                        self.kb_drag = Some(KeyboardDragMode::Resize {
                            edge,
                            anchor_local,
                            start_geom,
                            start_im_offset_x,
                            pending_frame: true,
                        });
                        self.mark_config_dirty();
                        self.redraw_keyboard_if_needed(qh);
                    }
                    return;
                }
            }
        }

        let hit = self
            .kb_surface
            .as_ref()
            .and_then(|kb| kb.layout.hit_test(p));
        let cursor = match hit {
            Some(KeyboardHit::Resize(ResizeEdge::North | ResizeEdge::South)) => {
                CursorIcon::NsResize
            }
            Some(KeyboardHit::Resize(ResizeEdge::East | ResizeEdge::West)) => CursorIcon::EwResize,
            Some(KeyboardHit::Resize(ResizeEdge::NorthWest | ResizeEdge::SouthEast)) => {
                CursorIcon::NwseResize
            }
            Some(KeyboardHit::Resize(ResizeEdge::NorthEast | ResizeEdge::SouthWest)) => {
                CursorIcon::NeswResize
            }
            Some(KeyboardHit::DragHeader) => CursorIcon::Grab,
            Some(_) => CursorIcon::Pointer,
            None => CursorIcon::Default,
        };
        if let Some(ref ptr) = self.pointer {
            let _ = ptr.set_cursor(conn, cursor);
        }

        let new_hover_key = match &hit {
            Some(KeyboardHit::Key(idx)) => self
                .kb_surface
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
            self.redraw_keyboard_if_needed(qh);
        }
    }

    fn handle_settings_action(&mut self, action: SettingsAction, qh: &QueueHandle<Self>) {
        match action {
            SettingsAction::None | SettingsAction::StartWindowMove => {}
            SettingsAction::Redraw => {
                self.mark_settings_dirty();
                self.redraw_dirty(qh);
            }
            SettingsAction::ConfigChanged => {
                self.mark_config_dirty();
                self.sync_tray_state();
                self.mark_keyboard_dirty();
                self.mark_settings_dirty();
                self.mark_editor_dirty();
                self.redraw_dirty(qh);
            }
            SettingsAction::LayoutChanged(code) => {
                self.apply_layout_change(&code, true);
                self.redraw_dirty(qh);
            }
            SettingsAction::CopyText(txt) => {
                self.clipboard.copy(&txt);
                self.mark_settings_dirty();
                self.redraw_dirty(qh);
            }
            SettingsAction::OpenFolder(dir) => {
                if let Err(err) = open_directory_in_file_manager(&dir) {
                    warn!("Failed opening folder {}: {err}", dir.display());
                    self.settings_view.status_message = Some(format!("Open folder failed: {err}"));
                }
                self.mark_settings_dirty();
                self.redraw_dirty(qh);
            }
            SettingsAction::OpenPanelEditor => {
                self.create_editor_surface(qh);
                self.redraw_dirty(qh);
            }
            SettingsAction::EnableKdeLockscreen => {
                match enable_kde_lockscreen_support() {
                    Ok(msg) => {
                        info!("KDE lockscreen setup updated: {msg}");
                        self.settings_view.status_message = Some(msg);
                    }
                    Err(err) => {
                        warn!("KDE lockscreen setup failed: {err}");
                        self.settings_view.status_message = Some(format!("Setup failed: {err}"));
                    }
                }
                self.settings_view.status_folder = None;
                self.settings_view.refresh_system_status();
                self.mark_settings_dirty();
                self.redraw_dirty(qh);
            }
            SettingsAction::DisableKdeLockscreen => {
                match disable_kde_lockscreen_support() {
                    Ok(msg) => {
                        info!("KDE lockscreen disabled: {msg}");
                        self.settings_view.status_message = Some(msg);
                    }
                    Err(err) => {
                        warn!("Failed disabling KDE lockscreen: {err}");
                        self.settings_view.status_message = Some(format!("Disable failed: {err}"));
                    }
                }
                self.settings_view.status_folder = None;
                self.settings_view.refresh_system_status();
                self.mark_settings_dirty();
                self.redraw_dirty(qh);
            }
            SettingsAction::DisableKdeMouseOverride => {
                match disable_kde_lockscreen_mouse_override() {
                    Ok(msg) => {
                        info!("KDE lockscreen mode updated: {msg}");
                        self.settings_view.status_message = Some(msg);
                    }
                    Err(err) => {
                        warn!("Failed disabling KDE mouse override: {err}");
                        self.settings_view.status_message = Some(format!("Update failed: {err}"));
                    }
                }
                self.settings_view.status_folder = None;
                self.settings_view.refresh_system_status();
                self.mark_settings_dirty();
                self.redraw_dirty(qh);
            }
            SettingsAction::EnableSddm => {
                self.settings_view.status_message =
                    Some("Requesting administrator permission for SDDM setup...".to_owned());
                self.settings_view.status_folder = None;
                self.platform.enable_sddm_gui();
                self.mark_settings_dirty();
                self.redraw_dirty(qh);
            }
            SettingsAction::DisableSddm => {
                self.settings_view.status_message =
                    Some("Requesting administrator permission to disable SDDM...".to_owned());
                self.settings_view.status_folder = None;
                self.platform.disable_sddm_gui();
                self.mark_settings_dirty();
                self.redraw_dirty(qh);
            }
            SettingsAction::CloseSettings => {
                self.destroy_settings_surface();
                self.redraw_dirty(qh);
            }
            SettingsAction::QuitApp => {
                self.running = false;
            }
        }
    }

    fn handle_editor_action(&mut self, action: PanelEditorAction, qh: &QueueHandle<Self>) {
        match action {
            PanelEditorAction::None | PanelEditorAction::StartWindowMove => {}
            PanelEditorAction::Redraw => {
                self.mark_editor_dirty();
                self.redraw_dirty(qh);
            }
            PanelEditorAction::PanelsChanged => {
                self.mark_config_dirty();
                self.sync_tray_state();
                self.mark_keyboard_dirty();
                self.mark_settings_dirty();
                self.mark_editor_dirty();
                self.redraw_dirty(qh);
            }
            PanelEditorAction::CopyText(txt) => {
                self.clipboard.copy(&txt);
                self.mark_editor_dirty();
                self.redraw_dirty(qh);
            }
            PanelEditorAction::OpenFolder(dir) => {
                if let Err(err) = open_directory_in_file_manager(&dir) {
                    warn!("Failed opening panels folder {}: {err}", dir.display());
                    self.editor_view.status_message = Some(format!("Open folder failed: {err}"));
                }
                self.mark_editor_dirty();
                self.redraw_dirty(qh);
            }
            PanelEditorAction::CloseEditor => {
                self.destroy_editor_surface();
                self.redraw_dirty(qh);
            }
        }
    }
}

fn ensure_shm_pool<'a>(
    pool: &'a mut Option<SlotPool>,
    shm: &Shm,
    min_bytes: usize,
) -> Option<&'a mut SlotPool> {
    if pool.is_none() {
        let initial = min_bytes.max(1280 * 512 * 4);
        *pool = SlotPool::new(initial, shm).ok();
    }
    pool.as_mut()
}

fn detect_window_resize_edge(p: Point, w: f32, h: f32) -> Option<(XdgResizeEdge, CursorIcon)> {
    let b = TOPLEVEL_RESIZE_BORDER;
    let left = p.x <= b;
    let right = p.x >= w - b;
    let top = p.y <= b;
    let bottom = p.y >= h - b;
    match (top, bottom, left, right) {
        (true, _, true, _) => Some((XdgResizeEdge::TopLeft, CursorIcon::NwseResize)),
        (true, _, _, true) => Some((XdgResizeEdge::TopRight, CursorIcon::NeswResize)),
        (_, true, true, _) => Some((XdgResizeEdge::BottomLeft, CursorIcon::NeswResize)),
        (_, true, _, true) => Some((XdgResizeEdge::BottomRight, CursorIcon::NwseResize)),
        (true, _, _, _) => Some((XdgResizeEdge::Top, CursorIcon::NsResize)),
        (_, true, _, _) => Some((XdgResizeEdge::Bottom, CursorIcon::NsResize)),
        (_, _, true, _) => Some((XdgResizeEdge::Left, CursorIcon::EwResize)),
        (_, _, _, true) => Some((XdgResizeEdge::Right, CursorIcon::EwResize)),
        _ => None,
    }
}

// --- Wayland & SCTK Handler Implementations ---

impl CompositorHandler for WlboardRuntime {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        new_factor: i32,
    ) {
        if let Some(ref mut kb) = self.kb_surface
            && kb.kind.wl_surface() == surface
        {
            kb.scale = new_factor.max(1);
            kb.dirty = true;
            self.redraw_keyboard_if_needed(qh);
        }
        if let Some(ref mut s) = self.settings_surface
            && s.window.wl_surface() == surface
        {
            s.scale = new_factor.max(1);
            s.dirty = true;
            self.redraw_settings_if_needed(qh);
        }
        if let Some(ref mut e) = self.editor_surface
            && e.window.wl_surface() == surface
        {
            e.scale = new_factor.max(1);
            e.dirty = true;
            self.redraw_editor_if_needed(qh);
        }
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
        if self.is_keyboard_surface(surface)
            && let Some(ref mut drag) = self.kb_drag
        {
            match drag {
                KeyboardDragMode::Move { pending_frame, .. }
                | KeyboardDragMode::Resize { pending_frame, .. } => {
                    *pending_frame = false;
                }
            }
        }
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        output: &wl_output::WlOutput,
    ) {
        if let Some(info) = self.output_state.info(output)
            && let Some((w, h)) = info.logical_size
            && w > 0
            && h > 0
        {
            self.output_bounds = OutputBounds {
                width: w as u32,
                height: h as u32,
            };
            self.clamp_floating_keyboard_to_output(qh);
        }
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for WlboardRuntime {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        if self.primary_output.is_none() {
            self.primary_output = Some(output.clone());
        }
        if let Some(info) = self.output_state.info(&output)
            && let Some((w, h)) = info.logical_size
            && w > 0
            && h > 0
        {
            self.output_bounds = OutputBounds {
                width: w as u32,
                height: h as u32,
            };
        }
        self.clamp_floating_keyboard_to_output(qh);
        if self.input_method_mode && self.pending_im_show && self.kb_surface.is_none() {
            self.create_keyboard_surface(qh);
            let _ = conn.flush();
        }
    }

    fn update_output(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        if self.primary_output.is_none() {
            self.primary_output = Some(output.clone());
        }
        if let Some(info) = self.output_state.info(&output)
            && let Some((w, h)) = info.logical_size
            && w > 0
            && h > 0
        {
            self.output_bounds = OutputBounds {
                width: w as u32,
                height: h as u32,
            };
        }
        self.clamp_floating_keyboard_to_output(qh);
        if self.input_method_mode && self.pending_im_show && self.kb_surface.is_none() {
            self.create_keyboard_surface(qh);
            let _ = conn.flush();
        }
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        if self.primary_output.as_ref() == Some(&output) {
            self.primary_output = self.output_state.outputs().find(|o| o != &output);
        }
    }
}

impl LayerShellHandler for WlboardRuntime {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _layer: &LayerSurface) {
        self.destroy_keyboard_surface();
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        _layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        if configure.new_size.0 >= MIN_WINDOW_WIDTH {
            self.state.config.window.width = configure.new_size.0;
        }
        if configure.new_size.1 >= MIN_WINDOW_HEIGHT {
            self.state.config.window.height = configure.new_size.1;
        }
        if let Some(ref mut kb) = self.kb_surface {
            kb.configured = true;
            kb.dirty = true;
        }
        self.redraw_keyboard_if_needed(qh);
    }
}

impl WindowHandler for WlboardRuntime {
    fn request_close(&mut self, _conn: &Connection, qh: &QueueHandle<Self>, window: &Window) {
        if self
            .settings_surface
            .as_ref()
            .is_some_and(|s| s.window.wl_surface() == window.wl_surface())
        {
            self.destroy_settings_surface();
        } else if self
            .editor_surface
            .as_ref()
            .is_some_and(|e| e.window.wl_surface() == window.wl_surface())
        {
            self.destroy_editor_surface();
        }
        self.redraw_dirty(qh);
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        window: &Window,
        configure: WindowConfigure,
        _serial: u32,
    ) {
        if self
            .settings_surface
            .as_ref()
            .is_some_and(|s| s.window.wl_surface() == window.wl_surface())
        {
            if let Some(w) = configure.new_size.0 {
                self.settings_view.width = (w.get() as f32).max(640.0);
            }
            if let Some(h) = configure.new_size.1 {
                self.settings_view.height = (h.get() as f32).max(540.0);
            }
            if let Some(ref mut s) = self.settings_surface {
                s.configured = true;
                s.dirty = true;
            }
            self.redraw_settings_if_needed(qh);
        } else if self
            .editor_surface
            .as_ref()
            .is_some_and(|e| e.window.wl_surface() == window.wl_surface())
        {
            if let Some(w) = configure.new_size.0 {
                self.editor_view.width = (w.get() as f32).max(780.0);
            }
            if let Some(h) = configure.new_size.1 {
                self.editor_view.height = (h.get() as f32).max(500.0);
            }
            if let Some(ref mut e) = self.editor_surface {
                e.configured = true;
                e.dirty = true;
            }
            self.redraw_editor_if_needed(qh);
        }
    }
}

impl SeatHandler for WlboardRuntime {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _conn: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat) {
        if self.primary_seat.is_none() {
            self.primary_seat = Some(seat.clone());
        }
        if !self.input_initialized {
            self.input_initialized = true;
            if let Some(ref mgr) = self.vk_manager {
                let vk = mgr.create_virtual_keyboard(&seat, qh, VirtualKeyboardData);
                self.input = InputManager::auto_select(Some(vk), self.state.active_layout());
                self.state.input_backend_status = self.input.status_detail();
                self.mark_settings_dirty();
            }
        }
    }

    fn new_capability(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if self.primary_seat.is_none() {
            self.primary_seat = Some(seat.clone());
            self.ensure_primary_seat(qh);
        }
        if capability == Capability::Pointer && self.pointer.is_none() {
            let cursor_surface = self.compositor_state.create_surface(qh);
            if let Ok(ptr) = self.seat_state.get_pointer_with_theme::<Self, ()>(
                qh,
                &seat,
                self.shm.wl_shm(),
                cursor_surface,
                ThemeSpec::default(),
            ) {
                self.pointer = Some(ptr);
            }
        }
        if capability == Capability::Touch
            && self.touch.is_none()
            && let Ok(t) = self.seat_state.get_touch(qh, &seat)
        {
            self.touch = Some(t);
        }
        if capability == Capability::Keyboard && self.wl_keyboard.is_none() {
            let kbd = seat.get_keyboard(qh, RawKeyboardData);
            self.wl_keyboard = Some(kbd);
        }
    }

    fn remove_capability(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer {
            self.pointer = None;
        }
        if capability == Capability::Touch
            && let Some(t) = self.touch.take()
        {
            t.release();
        }
        if capability == Capability::Keyboard
            && let Some(k) = self.wl_keyboard.take()
        {
            k.release();
        }
    }

    fn remove_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: wl_seat::WlSeat) {
    }
}

impl PointerHandler for WlboardRuntime {
    fn pointer_frame(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        _pointer: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for ev in events {
            let p = Point::new(ev.position.0 as f32, ev.position.1 as f32);
            if self.is_keyboard_surface(&ev.surface) {
                match ev.kind {
                    PointerEventKind::Motion { .. } | PointerEventKind::Enter { .. } => {
                        self.handle_kb_motion(p, conn, qh);
                    }
                    PointerEventKind::Leave { .. } => {
                        if self.kb_drag.is_none() {
                            self.state.hovered_key_id = None;
                            self.hovered_toolbar = None;
                            self.handle_kb_release(ContactId::Pointer, qh);
                        }
                    }
                    PointerEventKind::Press { button, .. }
                        if button == BTN_LEFT || button == BTN_RIGHT =>
                    {
                        self.handle_kb_press(ContactId::Pointer, p, button == BTN_RIGHT, qh);
                    }
                    PointerEventKind::Release { button, .. }
                        if button == BTN_LEFT || button == BTN_RIGHT =>
                    {
                        self.handle_kb_release(ContactId::Pointer, qh);
                    }
                    _ => {}
                }
            } else if self.is_settings_surface(&ev.surface) {
                match ev.kind {
                    PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                        if let Some(ref ptr) = self.pointer {
                            let cursor = if let Some((_, c)) = detect_window_resize_edge(
                                p,
                                self.settings_view.width,
                                self.settings_view.height,
                            ) {
                                c
                            } else if self.settings_view.is_header_drag_area(p) {
                                CursorIcon::Grab
                            } else {
                                CursorIcon::Default
                            };
                            let _ = ptr.set_cursor(conn, cursor);
                        }
                        if matches!(ev.kind, PointerEventKind::Motion { .. }) {
                            let action = self.settings_view.pointer_motion(p, &mut self.state);
                            self.handle_settings_action(action, qh);
                        }
                    }
                    PointerEventKind::Press { button, serial, .. } if button == BTN_LEFT => {
                        if let Some((edge, _)) = detect_window_resize_edge(
                            p,
                            self.settings_view.width,
                            self.settings_view.height,
                        ) && let Some(seat) = self.resolve_seat(qh)
                            && let Some(ref s) = self.settings_surface
                        {
                            s.window.resize(&seat, serial, edge);
                            let _ = conn.flush();
                            continue;
                        }
                        let prev_focus = self.settings_view.focused_field;
                        let action =
                            self.settings_view
                                .pointer_down(p, &mut self.state, &self.preset_store);
                        if action == SettingsAction::StartWindowMove {
                            if let Some(seat) = self.resolve_seat(qh)
                                && let Some(ref s) = self.settings_surface
                            {
                                s.window.move_(&seat, serial);
                                let _ = conn.flush();
                            }
                            if prev_focus != self.settings_view.focused_field {
                                self.mark_settings_dirty();
                                self.redraw_settings_if_needed(qh);
                            }
                            continue;
                        }
                        self.handle_settings_action(action, qh);
                    }
                    PointerEventKind::Release { button, .. } if button == BTN_LEFT => {
                        self.settings_view.pointer_up();
                    }
                    _ => {}
                }
            } else if self.is_editor_surface(&ev.surface) {
                match ev.kind {
                    PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                        if let Some(ref ptr) = self.pointer {
                            let cursor = if let Some((_, c)) = detect_window_resize_edge(
                                p,
                                self.editor_view.width,
                                self.editor_view.height,
                            ) {
                                c
                            } else if self.editor_view.is_header_drag_area(p) {
                                CursorIcon::Grab
                            } else {
                                CursorIcon::Default
                            };
                            let _ = ptr.set_cursor(conn, cursor);
                        }
                        if matches!(ev.kind, PointerEventKind::Motion { .. }) {
                            let action = self.editor_view.pointer_motion(p);
                            self.handle_editor_action(action, qh);
                        }
                    }
                    PointerEventKind::Press { button, serial, .. } if button == BTN_LEFT => {
                        if let Some((edge, _)) = detect_window_resize_edge(
                            p,
                            self.editor_view.width,
                            self.editor_view.height,
                        ) && let Some(seat) = self.resolve_seat(qh)
                            && let Some(ref e) = self.editor_surface
                        {
                            e.window.resize(&seat, serial, edge);
                            let _ = conn.flush();
                            continue;
                        }
                        let prev_focus = self.editor_view.focused_field;
                        let action =
                            self.editor_view
                                .pointer_down(p, &mut self.state, &self.panel_store);
                        if action == PanelEditorAction::StartWindowMove {
                            if let Some(seat) = self.resolve_seat(qh)
                                && let Some(ref e) = self.editor_surface
                            {
                                e.window.move_(&seat, serial);
                                let _ = conn.flush();
                            }
                            if prev_focus != self.editor_view.focused_field {
                                self.mark_editor_dirty();
                                self.redraw_editor_if_needed(qh);
                            }
                            continue;
                        }
                        self.handle_editor_action(action, qh);
                    }
                    PointerEventKind::Release { button, .. } if button == BTN_LEFT => {
                        let action = self
                            .editor_view
                            .pointer_up(&mut self.state, &self.panel_store);
                        self.handle_editor_action(action, qh);
                    }
                    _ => {}
                }
            }
        }
    }
}

impl TouchHandler for WlboardRuntime {
    fn down(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        _touch: &wl_touch::WlTouch,
        serial: u32,
        _time: u32,
        surface: wl_surface::WlSurface,
        id: i32,
        position: (f64, f64),
    ) {
        let p = Point::new(position.0 as f32, position.1 as f32);
        if self.is_keyboard_surface(&surface) {
            self.handle_kb_press(ContactId::Touch(id), p, false, qh);
        } else if self.is_settings_surface(&surface) {
            let prev_focus = self.settings_view.focused_field;
            let action = self
                .settings_view
                .pointer_down(p, &mut self.state, &self.preset_store);
            if action == SettingsAction::StartWindowMove {
                if let Some(seat) = self.resolve_seat(qh)
                    && let Some(ref s) = self.settings_surface
                {
                    s.window.move_(&seat, serial);
                    let _ = conn.flush();
                }
                if prev_focus != self.settings_view.focused_field {
                    self.mark_settings_dirty();
                    self.redraw_settings_if_needed(qh);
                }
                return;
            }
            self.handle_settings_action(action, qh);
        } else if self.is_editor_surface(&surface) {
            let prev_focus = self.editor_view.focused_field;
            let action = self
                .editor_view
                .pointer_down(p, &mut self.state, &self.panel_store);
            if action == PanelEditorAction::StartWindowMove {
                if let Some(seat) = self.resolve_seat(qh)
                    && let Some(ref e) = self.editor_surface
                {
                    e.window.move_(&seat, serial);
                    let _ = conn.flush();
                }
                if prev_focus != self.editor_view.focused_field {
                    self.mark_editor_dirty();
                    self.redraw_editor_if_needed(qh);
                }
                return;
            }
            self.handle_editor_action(action, qh);
        }
    }

    fn up(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        _touch: &wl_touch::WlTouch,
        _serial: u32,
        _time: u32,
        id: i32,
    ) {
        self.settings_view.pointer_up();
        let ed_act = self
            .editor_view
            .pointer_up(&mut self.state, &self.panel_store);
        self.handle_editor_action(ed_act, qh);
        self.handle_kb_release(ContactId::Touch(id), qh);
    }

    fn motion(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        _touch: &wl_touch::WlTouch,
        _time: u32,
        _id: i32,
        position: (f64, f64),
    ) {
        let p = Point::new(position.0 as f32, position.1 as f32);
        if self.kb_drag.is_some() {
            self.handle_kb_motion(p, conn, qh);
        } else if self.editor_surface.is_some() {
            let action = self.editor_view.pointer_motion(p);
            self.handle_editor_action(action, qh);
        } else if self.settings_surface.is_some() {
            let action = self.settings_view.pointer_motion(p, &mut self.state);
            self.handle_settings_action(action, qh);
        }
    }

    fn shape(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _touch: &wl_touch::WlTouch,
        _id: i32,
        _major: f64,
        _minor: f64,
    ) {
    }

    fn orientation(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _touch: &wl_touch::WlTouch,
        _id: i32,
        _orientation: f64,
    ) {
    }

    fn cancel(&mut self, _conn: &Connection, qh: &QueueHandle<Self>, _touch: &wl_touch::WlTouch) {
        self.kb_drag = None;
        self.release_all_keys();
        self.mark_keyboard_dirty();
        self.redraw_dirty(qh);
    }
}

impl ShmHandler for WlboardRuntime {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for WlboardRuntime {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

struct RawKeyboardData;
struct VirtualKeyboardData;
struct InputMethodData;

impl Dispatch2<wl_keyboard::WlKeyboard, WlboardRuntime> for RawKeyboardData {
    fn event(
        &self,
        state: &mut WlboardRuntime,
        _proxy: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _conn: &Connection,
        qh: &QueueHandle<WlboardRuntime>,
    ) {
        match event {
            wl_keyboard::Event::Key {
                key,
                state: wl_state,
                ..
            } => {
                let pressed = matches!(
                    wl_state,
                    wayland_client::WEnum::Value(wl_keyboard::KeyState::Pressed)
                );
                let kc = KeyCode(key as u16);
                if kc == KeyCode::LEFTSHIFT || kc == KeyCode::RIGHTSHIFT {
                    state.physical_shift_held = pressed;
                    return;
                }
                if kc == KeyCode::LEFTCTRL || kc == KeyCode::RIGHTCTRL {
                    state.physical_ctrl_held = pressed;
                    return;
                }
                if pressed {
                    let clip = if state.physical_ctrl_held && kc == KeyCode::V {
                        state.clipboard.paste()
                    } else {
                        None
                    };
                    if state.editor_surface.is_some()
                        && (state.editor_view.focused_field.is_some() || kc == KeyCode::DELETE)
                    {
                        let action = state.editor_view.handle_key_input(
                            kc,
                            state.physical_shift_held,
                            state.physical_ctrl_held,
                            clip.as_deref(),
                            &mut state.state,
                            &state.panel_store,
                        );
                        state.handle_editor_action(action, qh);
                    } else if state.settings_surface.is_some() {
                        let action = state.settings_view.handle_key_input(
                            kc,
                            state.physical_shift_held,
                            state.physical_ctrl_held,
                            clip.as_deref(),
                            &mut state.state,
                            &state.preset_store,
                        );
                        state.handle_settings_action(action, qh);
                    }
                }
            }
            wl_keyboard::Event::Modifiers { mods_depressed, .. } => {
                state.physical_shift_held = (mods_depressed & 1) != 0;
                state.physical_ctrl_held = (mods_depressed & 4) != 0;
            }
            _ => {}
        }
    }
}

impl Dispatch2<ZwpVirtualKeyboardManagerV1, WlboardRuntime> for VirtualKeyboardData {
    fn event(
        &self,
        _state: &mut WlboardRuntime,
        _proxy: &ZwpVirtualKeyboardManagerV1,
        _event: <ZwpVirtualKeyboardManagerV1 as Proxy>::Event,
        _conn: &Connection,
        _qh: &QueueHandle<WlboardRuntime>,
    ) {
    }
}

impl Dispatch2<ZwpVirtualKeyboardV1, WlboardRuntime> for VirtualKeyboardData {
    fn event(
        &self,
        _state: &mut WlboardRuntime,
        _proxy: &ZwpVirtualKeyboardV1,
        _event: <ZwpVirtualKeyboardV1 as Proxy>::Event,
        _conn: &Connection,
        _qh: &QueueHandle<WlboardRuntime>,
    ) {
    }
}

// --- Wayland Input Method v1 Dispatchers (KDE Plasma Lockscreen & SDDM Login Screen) ---

impl Dispatch2<ZwpInputMethodV1, WlboardRuntime> for InputMethodData {
    fn event(
        &self,
        state: &mut WlboardRuntime,
        _proxy: &ZwpInputMethodV1,
        event: zwp_input_method_v1::Event,
        conn: &Connection,
        qh: &QueueHandle<WlboardRuntime>,
    ) {
        match event {
            zwp_input_method_v1::Event::Activate { id } => {
                if !is_lockscreen_or_greeter_active() {
                    info!(
                        "Ignoring zwp_input_method_v1::Activate on unlocked desktop to prevent duplicate keyboard"
                    );
                    id.destroy();
                    let _ = conn.flush();
                    return;
                }
                info!(
                    "zwp_input_method_v1::Activate on Lockscreen/SDDM — showing virtual keyboard"
                );
                // Reload latest user theme, languages, and panels so Lockscreen matches desktop settings.
                let latest_cfg = AppConfig::load_default();
                let mut refreshed =
                    KeyboardState::new(latest_cfg, &state.preset_store, &state.panel_store);
                refreshed.input_method_mode = true;
                refreshed.input_backend_status = state.input.status_detail();
                state.state = refreshed;

                state.input.activate_input_method(id);
                state.create_keyboard_surface(qh);
                state.mark_keyboard_dirty();
                state.redraw_keyboard_if_needed(qh);
                let _ = conn.flush();
            }
            zwp_input_method_v1::Event::Deactivate { context } => {
                info!("zwp_input_method_v1::Deactivate received — hiding virtual keyboard");
                context.destroy();
                state.input.deactivate_input_method();
                state.destroy_keyboard_surface();
                let _ = conn.flush();
            }
            _ => {}
        }
    }

    fn event_created_child(opcode: u16, qh: &QueueHandle<WlboardRuntime>) -> Arc<dyn ObjectData> {
        if opcode == zwp_input_method_v1::EVT_ACTIVATE_OPCODE {
            qh.make_data::<ZwpInputMethodContextV1, _>(InputMethodData)
        } else {
            panic!("Unexpected child opcode {opcode} on zwp_input_method_v1");
        }
    }
}

impl Dispatch2<ZwpInputMethodContextV1, WlboardRuntime> for InputMethodData {
    fn event(
        &self,
        state: &mut WlboardRuntime,
        _proxy: &ZwpInputMethodContextV1,
        event: zwp_input_method_context_v1::Event,
        _conn: &Connection,
        _qh: &QueueHandle<WlboardRuntime>,
    ) {
        if let zwp_input_method_context_v1::Event::CommitState { serial } = event {
            state.input.update_input_method_serial(serial);
        }
    }
}

impl Dispatch2<ZwpInputPanelV1, WlboardRuntime> for InputMethodData {
    fn event(
        &self,
        _state: &mut WlboardRuntime,
        _proxy: &ZwpInputPanelV1,
        _event: <ZwpInputPanelV1 as Proxy>::Event,
        _conn: &Connection,
        _qh: &QueueHandle<WlboardRuntime>,
    ) {
    }
}

impl Dispatch2<ZwpInputPanelSurfaceV1, WlboardRuntime> for InputMethodData {
    fn event(
        &self,
        _state: &mut WlboardRuntime,
        _proxy: &ZwpInputPanelSurfaceV1,
        _event: zwp_input_panel_surface_v1::Event,
        _conn: &Connection,
        _qh: &QueueHandle<WlboardRuntime>,
    ) {
    }
}

delegate_registry!(WlboardRuntime);
delegate_dispatch2!(WlboardRuntime);
