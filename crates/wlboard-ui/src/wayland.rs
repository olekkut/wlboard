//! Native Wayland client runtime using `smithay-client-toolkit` + `calloop`:
//! - Floating draggable & 8-edge resizable `zwlr_layer_shell_v1` overlay (`Layer::Overlay`, `KeyboardInteractivity::None`)
//! - Built-in themed `xdg_toplevel` **Customize wlboard** Settings window
//! - True `wl_touch` multi-touch + `wl_pointer` + raw `wl_keyboard` (for Settings inputs)
//! - Automatic `zwp_virtual_keyboard_v1` -> `/dev/uinput` fallback + 250 ms stuck-key watchdog + key repeat

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use calloop::EventLoop;
use calloop::channel::{self, Channel};
use calloop::timer::{TimeoutAction, Timer};
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState, FrameCallbackData};
use smithay_client_toolkit::dispatch2::Dispatch2;
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
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
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::{
    wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface, wl_touch,
};
use wayland_client::{Connection, Proxy, QueueHandle};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};
use wlboard_core::{
    AppConfig, ContactId, KeyCode, KeyboardState, KeyboardStyle, MIN_WINDOW_HEIGHT,
    MIN_WINDOW_WIDTH, OutputBounds, Point, PresetStore, ResizeEdge, WindowGeometry, apply_resize,
    clamp_move,
};
use wlboard_platform::{InputManager, PlatformEvent, PlatformHandle, TrayState};

use crate::canvas::{UiRenderer, copy_pixmap_to_wayland_argb8888};
use crate::keyboard_view::{KeyboardHit, KeyboardViewLayout};
use crate::settings_view::{
    SETTINGS_DEFAULT_HEIGHT, SETTINGS_DEFAULT_WIDTH, SettingsAction, SettingsView,
};

/// Wayland Linux button codes (`BTN_LEFT = 272`, `BTN_RIGHT = 273`).
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;

/// Key repeat timing constants.
const REPEAT_INITIAL_DELAY: Duration = Duration::from_millis(420);
const REPEAT_INTERVAL: Duration = Duration::from_millis(55);

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
        pending_frame: bool,
    },
}

#[derive(Debug, Clone, Copy)]
struct RepeatState {
    keycode: KeyCode,
    next_fire: Instant,
}

struct KeyboardSurfaceState {
    layer: LayerSurface,
    configured: bool,
    scale: i32,
    dirty: bool,
    layout: KeyboardViewLayout,
}

struct SettingsSurfaceState {
    window: Window,
    configured: bool,
    scale: i32,
    dirty: bool,
}

/// Complete Wayland application runtime state.
struct WlboardRuntime {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    compositor_state: CompositorState,
    shm: Shm,
    layer_shell: LayerShell,
    xdg_shell: XdgShell,
    pool: SlotPool,

    pointer: Option<ThemedPointer>,
    touch: Option<wl_touch::WlTouch>,
    wl_keyboard: Option<wl_keyboard::WlKeyboard>,
    primary_seat: Option<wl_seat::WlSeat>,
    vk_manager: Option<ZwpVirtualKeyboardManagerV1>,
    input: InputManager,
    input_initialized: bool,

    renderer: UiRenderer,
    state: KeyboardState,
    preset_store: PresetStore,
    platform: PlatformHandle,
    output_bounds: OutputBounds,

    kb_surface: Option<KeyboardSurfaceState>,
    settings_surface: Option<SettingsSurfaceState>,
    settings_view: SettingsView,

    hovered_toolbar: Option<KeyboardHit>,
    kb_drag: Option<KeyboardDragMode>,
    repeat: Option<RepeatState>,
    physical_shift_held: bool,
    config_dirty_since: Option<Instant>,
    running: bool,
    shutdown_flag: Arc<AtomicBool>,
}

/// Launch the Wayland `wlboard` runtime and block until shutdown.
pub fn run_wayland_app(
    initial_settings_open: bool,
    shutdown_flag: Arc<AtomicBool>,
) -> Result<(), String> {
    let conn = Connection::connect_to_env().map_err(|e| {
        format!("Could not connect to Wayland display (is WAYLAND_DISPLAY set?): {e}")
    })?;

    let (globals, event_queue) = registry_queue_init::<WlboardRuntime>(&conn)
        .map_err(|e| format!("Failed to initialize Wayland registry: {e}"))?;
    let qh = event_queue.handle();

    let registry_state = RegistryState::new(&globals);
    let seat_state = SeatState::new(&globals, &qh);
    let output_state = OutputState::new(&globals, &qh);
    let compositor_state = CompositorState::bind(&globals, &qh)
        .map_err(|e| format!("wl_compositor unavailable: {e}"))?;
    let shm = Shm::bind(&globals, &qh).map_err(|e| format!("wl_shm unavailable: {e}"))?;
    let layer_shell = LayerShell::bind(&globals, &qh)
        .map_err(|e| format!("zwlr_layer_shell_v1 unavailable on this Wayland compositor: {e}"))?;
    let xdg_shell =
        XdgShell::bind(&globals, &qh).map_err(|e| format!("xdg_wm_base unavailable: {e}"))?;

    let vk_manager = globals
        .bind::<ZwpVirtualKeyboardManagerV1, _, _>(&qh, 1..=1, VirtualKeyboardData)
        .ok();
    if vk_manager.is_some() {
        info!("Compositor advertises zwp_virtual_keyboard_v1");
    } else {
        info!("Compositor does not expose zwp_virtual_keyboard_v1; will use /dev/uinput fallback");
    }

    let pool = SlotPool::new(1920 * 1080 * 4, &shm)
        .map_err(|e| format!("Failed to create Wayland SHM pool: {e}"))?;

    let config = AppConfig::load_default();
    let preset_store = PresetStore::default_store();
    let mut kb_state = KeyboardState::new(config, &preset_store);
    if initial_settings_open {
        kb_state.settings_open = true;
    }

    let (platform_tx, platform_rx): (channel::Sender<PlatformEvent>, Channel<PlatformEvent>) =
        channel::channel();

    let initial_tray = TrayState {
        visible: kb_state.visible,
        style: kb_state.config.layout.style,
        active_layout: kb_state.config.layout.active.clone(),
        revision: 1,
    };
    let platform = PlatformHandle::spawn(initial_tray, kb_state.registry.clone(), move |ev| {
        let _ = platform_tx.send(ev);
    });

    // Placeholder input manager until first seat is bound (then auto-selects wayland_vk -> uinput)
    let input = InputManager::auto_select(None, kb_state.active_layout());
    kb_state.input_backend_status = input.status_detail();

    let settings_view = SettingsView::new(&kb_state.config.appearance);

    let mut runtime = WlboardRuntime {
        registry_state,
        seat_state,
        output_state,
        compositor_state,
        shm,
        layer_shell,
        xdg_shell,
        pool,
        pointer: None,
        touch: None,
        wl_keyboard: None,
        primary_seat: None,
        vk_manager,
        input,
        input_initialized: false,
        renderer: UiRenderer::new(),
        state: kb_state,
        preset_store,
        platform,
        output_bounds: OutputBounds::default(),
        kb_surface: None,
        settings_surface: None,
        settings_view,
        hovered_toolbar: None,
        kb_drag: None,
        repeat: None,
        physical_shift_held: false,
        config_dirty_since: None,
        running: true,
        shutdown_flag,
    };

    if runtime.state.visible {
        runtime.create_keyboard_surface(&qh);
    }
    if runtime.state.settings_open {
        runtime.create_settings_surface(&qh);
    }

    let mut event_loop: EventLoop<WlboardRuntime> =
        EventLoop::try_new().map_err(|e| format!("Failed creating calloop event loop: {e}"))?;
    let loop_handle = event_loop.handle();

    WaylandSource::new(conn.clone(), event_queue)
        .insert(loop_handle.clone())
        .map_err(|e| format!("Failed inserting WaylandSource: {e}"))?;

    let qh_platform = qh.clone();
    loop_handle
        .insert_source(platform_rx, move |event, _, app| {
            if let channel::Event::Msg(ev) = event {
                app.handle_platform_event(ev, &qh_platform);
            }
        })
        .map_err(|e| format!("Failed registering platform event channel: {e}"))?;

    // 40 ms periodic tick: handles key-repeat, 250 ms stuck-key watchdog, debounced config save, & signal exit
    loop_handle
        .insert_source(
            Timer::from_duration(Duration::from_millis(40)),
            move |_, _, app| {
                app.periodic_tick();
                TimeoutAction::ToDuration(Duration::from_millis(40))
            },
        )
        .map_err(|e| format!("Failed registering periodic watchdog timer: {e}"))?;

    while runtime.running && !runtime.shutdown_flag.load(Ordering::Relaxed) {
        if let Err(err) = event_loop.dispatch(Some(Duration::from_millis(50)), &mut runtime) {
            warn!("Event loop dispatch error: {err}");
            break;
        }
        let _ = conn.flush();
    }

    info!("Shutting down wlboard cleanly — releasing all virtual keys");
    runtime.release_all_keys();
    runtime.save_config_now();
    runtime.platform.shutdown();
    Ok(())
}

impl WlboardRuntime {
    fn create_keyboard_surface(&mut self, qh: &QueueHandle<Self>) {
        if self.kb_surface.is_some() {
            return;
        }
        let geom = self.state.config.window;
        let surface = self.compositor_state.create_surface(qh);
        let layer = self.layer_shell.create_layer_surface(
            qh,
            surface,
            Layer::Overlay,
            Some("wlboard"),
            None,
        );
        layer.set_anchor(Anchor::TOP | Anchor::LEFT);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_exclusive_zone(-1);
        layer.set_size(geom.width, geom.height);
        layer.set_margin(geom.y, 0, 0, geom.x);
        layer.commit();

        let layout =
            KeyboardViewLayout::compute(&self.state, geom.width as f32, geom.height as f32);
        self.kb_surface = Some(KeyboardSurfaceState {
            layer,
            configured: false,
            scale: 1,
            dirty: true,
            layout,
        });
        self.state.visible = true;
        self.sync_tray_state();
    }

    fn destroy_keyboard_surface(&mut self) {
        self.release_all_keys();
        self.kb_drag = None;
        self.hovered_toolbar = None;
        self.state.hovered_key_id = None;
        self.kb_surface = None;
        self.state.visible = false;
        self.sync_tray_state();
    }

    fn create_settings_surface(&mut self, qh: &QueueHandle<Self>) {
        if self.settings_surface.is_some() {
            return;
        }
        self.state.reload_presets(&self.preset_store);
        self.settings_view
            .sync_from_appearance(&self.state.config.appearance);

        let surface = self.compositor_state.create_surface(qh);
        let window = self
            .xdg_shell
            .create_window(surface, WindowDecorations::None, qh);
        window.set_title("Customize wlboard");
        window.set_app_id("io.github.olekkut.wlboard");
        window.set_min_size(Some((640, 620)));
        window.commit();

        self.settings_view.width = SETTINGS_DEFAULT_WIDTH as f32;
        self.settings_view.height = SETTINGS_DEFAULT_HEIGHT as f32;
        self.settings_surface = Some(SettingsSurfaceState {
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
        self.mark_keyboard_dirty();
    }

    fn sync_tray_state(&self) {
        self.platform.update_tray(
            self.state.visible,
            self.state.config.layout.style,
            self.state.config.layout.active.clone(),
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
            if push_to_system && self.state.config.layout.system_layout_sync {
                self.platform
                    .sync_system_layout(self.state.config.layout.active.clone());
            }
            self.sync_tray_state();
            self.mark_config_dirty();
            self.mark_keyboard_dirty();
            self.mark_settings_dirty();
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
                self.redraw_dirty(qh);
            }
            PlatformEvent::SetLayout(code) => {
                self.apply_layout_change(&code, true);
                self.redraw_dirty(qh);
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
            PlatformEvent::Quit => {
                self.running = false;
            }
        }
    }

    fn periodic_tick(&mut self) {
        let now = Instant::now();

        // 1. Key repeat for held Backspace / arrows / letters
        if let Some(ref mut rep) = self.repeat
            && now >= rep.next_fire
        {
            self.input
                .execute(&[wlboard_core::InputCommand::Tap(rep.keycode)]);
            rep.next_fire = now + REPEAT_INTERVAL;
        }

        // 2. 250 ms stuck-key watchdog: protect active modifiers + actively touched keys
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

        // 3. Debounced config persistence (250 ms after last drag/slider change)
        if let Some(since) = self.config_dirty_since
            && now.duration_since(since) >= Duration::from_millis(250)
        {
            self.save_config_now();
        }
    }

    fn redraw_dirty(&mut self, qh: &QueueHandle<Self>) {
        self.redraw_keyboard_if_needed(qh);
        self.redraw_settings_if_needed(qh);
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
        let phys_w = (geom.width * scale as u32) as i32;
        let phys_h = (geom.height * scale as u32) as i32;
        let stride = phys_w * 4;

        kb.layout = KeyboardViewLayout::compute(&self.state, geom.width as f32, geom.height as f32);

        let Some(mut pixmap) = Pixmap::new(phys_w as u32, phys_h as u32) else {
            return;
        };
        kb.layout.render(
            &mut self.renderer,
            &mut pixmap,
            &self.state,
            self.hovered_toolbar.as_ref(),
            scale as f32,
        );

        let Ok((buffer, canvas)) =
            self.pool
                .create_buffer(phys_w, phys_h, stride, wl_shm::Format::Argb8888)
        else {
            warn!("Failed allocating Wayland SHM buffer for keyboard");
            return;
        };
        copy_pixmap_to_wayland_argb8888(&pixmap, canvas);

        let wl_surf = kb.layer.wl_surface();
        wl_surf.set_buffer_scale(scale);
        wl_surf.damage_buffer(0, 0, phys_w, phys_h);
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

        let Ok((buffer, canvas)) =
            self.pool
                .create_buffer(phys_w, phys_h, stride, wl_shm::Format::Argb8888)
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

    fn is_keyboard_surface(&self, surface: &wl_surface::WlSurface) -> bool {
        self.kb_surface
            .as_ref()
            .is_some_and(|kb| kb.layer.wl_surface() == surface)
    }

    fn is_settings_surface(&self, surface: &wl_surface::WlSurface) -> bool {
        self.settings_surface
            .as_ref()
            .is_some_and(|s| s.window.wl_surface() == surface)
    }

    fn handle_kb_press(
        &mut self,
        contact: ContactId,
        p: Point,
        right_click: bool,
        qh: &QueueHandle<Self>,
    ) {
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
                let cmds = self.state.press_key(contact, &esc, false);
                self.input.execute(&cmds);
                self.mark_keyboard_dirty();
            }
            KeyboardHit::ToolbarLang(code) => {
                self.apply_layout_change(&code, true);
            }
            KeyboardHit::ToolbarOptions => {
                if self.settings_surface.is_some() {
                    self.destroy_settings_surface();
                } else {
                    self.create_settings_surface(qh);
                }
            }
            KeyboardHit::ToolbarHide => {
                self.destroy_keyboard_surface();
                return;
            }
            KeyboardHit::Key(idx) => {
                let spec = self
                    .kb_surface
                    .as_ref()
                    .and_then(|kb| kb.layout.keys.get(idx))
                    .map(|k| k.spec.clone());
                if let Some(spec) = spec {
                    // If Settings inline text field is focused, let wlboard type directly into it too!
                    if self.settings_surface.is_some()
                        && self.settings_view.focused_field.is_some()
                        && let wlboard_core::KeyAction::Key(kc) = spec.action
                        && !matches!(spec.role, wlboard_core::KeyRole::Modifier(_))
                    {
                        let shift = self.state.modifiers.effective_uppercase() || right_click;
                        let action = self.settings_view.handle_key_input(
                            kc,
                            shift,
                            &mut self.state,
                            &self.preset_store,
                        );
                        self.handle_settings_action(action, qh);
                        return;
                    }

                    let cmds = self.state.press_key(contact, &spec, right_click);
                    self.input.execute(&cmds);

                    // Arm key-repeat for repeatable keys (letters, backspace, arrows, space, delete)
                    if let wlboard_core::KeyAction::Key(kc) = spec.action
                        && !matches!(spec.role, wlboard_core::KeyRole::Modifier(_))
                    {
                        self.repeat = Some(RepeatState {
                            keycode: kc,
                            next_fire: Instant::now() + REPEAT_INITIAL_DELAY,
                        });
                    }
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

    fn handle_kb_motion(&mut self, p: Point, conn: &Connection, qh: &QueueHandle<Self>) {
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
                        let next = clamp_move(self.state.config.window, dx, dy, self.output_bounds);
                        self.state.config.window = next;
                        if let Some(ref kb) = self.kb_surface {
                            kb.layer.set_margin(next.y, 0, 0, next.x);
                            let surf = kb.layer.wl_surface();
                            surf.frame(qh, FrameCallbackData(surf.clone()));
                            kb.layer.commit();
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
                    pending_frame,
                } => {
                    if pending_frame && (edge.moves_left() || edge.moves_top()) {
                        return;
                    }
                    let dx = (p.x - anchor_local.x).round() as i32;
                    let dy = (p.y - anchor_local.y).round() as i32;
                    let base = if edge.moves_left() || edge.moves_top() {
                        self.state.config.window
                    } else {
                        start_geom
                    };
                    let next = apply_resize(base, edge, dx, dy, self.output_bounds);
                    if next != self.state.config.window {
                        self.state.config.window = next;
                        if let Some(ref mut kb) = self.kb_surface {
                            kb.layer.set_size(next.width, next.height);
                            kb.layer.set_margin(next.y, 0, 0, next.x);
                            let surf = kb.layer.wl_surface();
                            surf.frame(qh, FrameCallbackData(surf.clone()));
                            kb.dirty = true;
                            kb.layer.commit();
                        }
                        self.kb_drag = Some(KeyboardDragMode::Resize {
                            edge,
                            anchor_local,
                            start_geom,
                            pending_frame: true,
                        });
                        self.mark_config_dirty();
                        self.redraw_keyboard_if_needed(qh);
                    }
                    return;
                }
            }
        }

        // Hover state & cursor feedback on keyboard overlay
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
                .map(|k| k.spec.id.clone()),
            _ => None,
        };
        let new_hover_tb = match hit {
            Some(
                h @ (KeyboardHit::ToolbarEsc
                | KeyboardHit::ToolbarLang(_)
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
            SettingsAction::None => {}
            SettingsAction::Redraw => {
                self.mark_settings_dirty();
                self.redraw_dirty(qh);
            }
            SettingsAction::ConfigChanged => {
                self.mark_config_dirty();
                self.sync_tray_state();
                self.mark_keyboard_dirty();
                self.mark_settings_dirty();
                self.redraw_dirty(qh);
            }
            SettingsAction::LayoutChanged(code) => {
                self.apply_layout_change(&code, true);
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
            && kb.layer.wl_surface() == surface
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
        _qh: &QueueHandle<Self>,
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
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
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
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
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
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
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
    fn request_close(&mut self, _conn: &Connection, qh: &QueueHandle<Self>, _window: &Window) {
        self.destroy_settings_surface();
        self.redraw_dirty(qh);
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        _window: &Window,
        configure: WindowConfigure,
        _serial: u32,
    ) {
        if let Some(w) = configure.new_size.0 {
            self.settings_view.width = (w.get() as f32).max(640.0);
        }
        if let Some(h) = configure.new_size.1 {
            self.settings_view.height = (h.get() as f32).max(620.0);
        }
        if let Some(ref mut s) = self.settings_surface {
            s.configured = true;
            s.dirty = true;
        }
        self.redraw_settings_if_needed(qh);
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
        // Upgrade input backend to native Wayland virtual keyboard if advertised on this compositor.
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
                            self.mark_keyboard_dirty();
                            self.redraw_keyboard_if_needed(qh);
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
                    PointerEventKind::Enter { .. } => {
                        if let Some(ref ptr) = self.pointer {
                            let _ = ptr.set_cursor(conn, CursorIcon::Default);
                        }
                    }
                    PointerEventKind::Press { button, serial, .. } if button == BTN_LEFT => {
                        let action =
                            self.settings_view
                                .pointer_down(p, &mut self.state, &self.preset_store);
                        // Drag empty header area of Settings window using native xdg_toplevel move!
                        if action == SettingsAction::Redraw
                            && p.y <= 54.0
                            && let (Some(seat), Some(s)) =
                                (&self.primary_seat, &self.settings_surface)
                        {
                            s.window.xdg_toplevel()._move(seat, serial);
                        }
                        self.handle_settings_action(action, qh);
                    }
                    PointerEventKind::Motion { .. } => {
                        let action = self.settings_view.pointer_motion(p, &mut self.state);
                        self.handle_settings_action(action, qh);
                    }
                    PointerEventKind::Release { button, .. } if button == BTN_LEFT => {
                        self.settings_view.pointer_up();
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
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        _touch: &wl_touch::WlTouch,
        _serial: u32,
        _time: u32,
        surface: wl_surface::WlSurface,
        id: i32,
        position: (f64, f64),
    ) {
        let p = Point::new(position.0 as f32, position.1 as f32);
        if self.is_keyboard_surface(&surface) {
            self.handle_kb_press(ContactId::Touch(id), p, false, qh);
        } else if self.is_settings_surface(&surface) {
            let action = self
                .settings_view
                .pointer_down(p, &mut self.state, &self.preset_store);
            self.handle_settings_action(action, qh);
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

// Raw `wl_keyboard` dispatch for Settings inline Preset Name & #RRGGBB Hex inputs (zero libxkbcommon dep!).
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
                if pressed && state.settings_surface.is_some() {
                    let action = state.settings_view.handle_key_input(
                        kc,
                        state.physical_shift_held,
                        &mut state.state,
                        &state.preset_store,
                    );
                    state.handle_settings_action(action, qh);
                }
            }
            wl_keyboard::Event::Modifiers { mods_depressed, .. } => {
                state.physical_shift_held = (mods_depressed & 1) != 0;
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

delegate_registry!(WlboardRuntime);
delegate_dispatch2!(WlboardRuntime);
