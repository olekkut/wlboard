//! Native `org.kde.StatusNotifierItem` + `com.canonical.dbusmenu` system tray over `zbus`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tracing::{debug, info};
use wlboard_core::{BUILTIN_LAYOUT_CODES, KeyboardStyle};
use zbus::wire::{ObjectPath, OwnedVariant, Variant};
use zbus::{Connection, interface, proxy};

use crate::PlatformEvent;

pub const SNI_OBJECT_PATH: &str = "/StatusNotifierItem";
pub const MENU_OBJECT_PATH: &str = "/MenuBar";

/// Menu item IDs shared between `GetLayout` and `Event`.
const MENU_ROOT: i32 = 0;
const MENU_TOGGLE_VISIBLE: i32 = 1;
const MENU_OPEN_SETTINGS: i32 = 2;
const MENU_TOGGLE_COMPACT: i32 = 3;
const MENU_LAYOUT_SUBMENU: i32 = 4;
const MENU_SEPARATOR: i32 = 5;
const MENU_QUIT: i32 = 6;
const MENU_LAYOUT_BASE: i32 = 100;

/// Snapshot of state reflected in the system tray menu.
#[derive(Debug, Clone)]
pub struct TrayState {
    pub visible: bool,
    pub style: KeyboardStyle,
    pub active_layout: String,
    pub revision: u32,
}

impl Default for TrayState {
    fn default() -> Self {
        Self {
            visible: true,
            style: KeyboardStyle::Standard,
            active_layout: "en".to_owned(),
            revision: 1,
        }
    }
}

/// Callback invoked when the user activates the tray icon or clicks a menu entry.
pub type EventCallback = Arc<dyn Fn(PlatformEvent) + Send + Sync>;

/// `org.kde.StatusNotifierItem` D-Bus service implementation.
pub struct StatusNotifierItem {
    state: Arc<Mutex<TrayState>>,
    on_event: EventCallback,
    icon_pixmap: Vec<(i32, i32, Vec<u8>)>,
}

impl StatusNotifierItem {
    #[must_use]
    pub fn new(state: Arc<Mutex<TrayState>>, on_event: EventCallback) -> Self {
        Self {
            state,
            on_event,
            icon_pixmap: vec![build_tray_icon_argb32()],
        }
    }
}

type ToolTipTuple = (String, Vec<(i32, i32, Vec<u8>)>, String, String);

#[interface(name = "org.kde.StatusNotifierItem")]
impl StatusNotifierItem {
    #[zbus(property)]
    fn category(&self) -> &'static str {
        "SystemServices"
    }

    #[zbus(property)]
    fn id(&self) -> &'static str {
        "io.github.olekkut.wlboard"
    }

    #[zbus(property)]
    fn title(&self) -> &'static str {
        "wlboard"
    }

    #[zbus(property)]
    fn status(&self) -> &'static str {
        "Active"
    }

    #[zbus(property)]
    fn window_id(&self) -> i32 {
        0
    }

    #[zbus(property)]
    fn icon_name(&self) -> &'static str {
        "io.github.olekkut.wlboard"
    }

    #[zbus(property)]
    fn icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        self.icon_pixmap.clone()
    }

    #[zbus(property)]
    fn item_is_menu(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn menu(&self) -> ObjectPath<'static> {
        ObjectPath::from_static_str_unchecked(MENU_OBJECT_PATH)
    }

    #[zbus(property)]
    fn tool_tip(&self) -> ToolTipTuple {
        let layout = self
            .state
            .lock()
            .map(|s| s.active_layout.to_uppercase())
            .unwrap_or_else(|_| "EN".to_owned());
        (
            "io.github.olekkut.wlboard".to_owned(),
            Vec::new(),
            "wlboard".to_owned(),
            format!("Virtual Keyboard ({layout}) - Click to show or hide"),
        )
    }

    fn activate(&self, _x: i32, _y: i32) {
        (self.on_event)(PlatformEvent::ToggleVisible);
    }

    fn secondary_activate(&self, _x: i32, _y: i32) {
        (self.on_event)(PlatformEvent::OpenSettings);
    }

    fn context_menu(&self, _x: i32, _y: i32) {}

    fn scroll(&self, _delta: i32, _orientation: &str) {}
}

type MenuItemTuple = (i32, HashMap<String, OwnedVariant>, Vec<OwnedVariant>);

/// `com.canonical.dbusmenu` D-Bus service powering the tray's right-click menu.
pub struct DbusMenu {
    state: Arc<Mutex<TrayState>>,
    on_event: EventCallback,
}

impl DbusMenu {
    #[must_use]
    pub fn new(state: Arc<Mutex<TrayState>>, on_event: EventCallback) -> Self {
        Self { state, on_event }
    }

    fn prop_str(v: &str) -> OwnedVariant {
        OwnedVariant::try_from(Variant::from(v)).expect("string variant")
    }

    fn prop_bool(v: bool) -> OwnedVariant {
        OwnedVariant::try_from(Variant::from(v)).expect("bool variant")
    }

    fn prop_i32(v: i32) -> OwnedVariant {
        OwnedVariant::try_from(Variant::from(v)).expect("i32 variant")
    }

    fn item_value(
        id: i32,
        props: HashMap<String, OwnedVariant>,
        children: Vec<OwnedVariant>,
    ) -> OwnedVariant {
        let tuple: MenuItemTuple = (id, props, children);
        OwnedVariant::try_from(Variant::from(tuple)).expect("menu tuple variant")
    }

    fn build_root_menu(&self) -> (u32, MenuItemTuple) {
        let snapshot = self.state.lock().map(|s| s.clone()).unwrap_or_default();

        let toggle_label = if snapshot.visible {
            "Hide Keyboard"
        } else {
            "Show Keyboard"
        };

        let mut p_toggle = HashMap::new();
        p_toggle.insert("label".to_owned(), Self::prop_str(toggle_label));
        p_toggle.insert("enabled".to_owned(), Self::prop_bool(true));
        p_toggle.insert("visible".to_owned(), Self::prop_bool(true));

        let mut p_settings = HashMap::new();
        p_settings.insert("label".to_owned(), Self::prop_str("Customize wlboard..."));
        p_settings.insert("enabled".to_owned(), Self::prop_bool(true));
        p_settings.insert("visible".to_owned(), Self::prop_bool(true));

        let mut p_compact = HashMap::new();
        p_compact.insert("label".to_owned(), Self::prop_str("Compact for TV"));
        p_compact.insert("toggle-type".to_owned(), Self::prop_str("checkmark"));
        p_compact.insert(
            "toggle-state".to_owned(),
            Self::prop_i32(i32::from(snapshot.style == KeyboardStyle::CompactTv)),
        );
        p_compact.insert("enabled".to_owned(), Self::prop_bool(true));
        p_compact.insert("visible".to_owned(), Self::prop_bool(true));

        let layout_labels = [
            ("en", "English (EN)"),
            ("pl", "Polish (PL)"),
            ("uk", "Ukrainian (UA)"),
            ("ru", "Russian (RU)"),
            ("ar", "Arabic (AR)"),
        ];
        let layout_children: Vec<OwnedVariant> = layout_labels
            .iter()
            .enumerate()
            .map(|(idx, (code, label))| {
                let mut lp = HashMap::new();
                lp.insert("label".to_owned(), Self::prop_str(label));
                lp.insert("toggle-type".to_owned(), Self::prop_str("radio"));
                lp.insert(
                    "toggle-state".to_owned(),
                    Self::prop_i32(i32::from(snapshot.active_layout == *code)),
                );
                lp.insert("enabled".to_owned(), Self::prop_bool(true));
                lp.insert("visible".to_owned(), Self::prop_bool(true));
                Self::item_value(MENU_LAYOUT_BASE + idx as i32, lp, Vec::new())
            })
            .collect();

        let mut p_layouts = HashMap::new();
        p_layouts.insert("label".to_owned(), Self::prop_str("Layout"));
        p_layouts.insert("children-display".to_owned(), Self::prop_str("submenu"));
        p_layouts.insert("enabled".to_owned(), Self::prop_bool(true));
        p_layouts.insert("visible".to_owned(), Self::prop_bool(true));

        let mut p_sep = HashMap::new();
        p_sep.insert("type".to_owned(), Self::prop_str("separator"));
        p_sep.insert("visible".to_owned(), Self::prop_bool(true));

        let mut p_quit = HashMap::new();
        p_quit.insert("label".to_owned(), Self::prop_str("Quit"));
        p_quit.insert("enabled".to_owned(), Self::prop_bool(true));
        p_quit.insert("visible".to_owned(), Self::prop_bool(true));

        let mut root_props = HashMap::new();
        root_props.insert("children-display".to_owned(), Self::prop_str("submenu"));

        let children = vec![
            Self::item_value(MENU_TOGGLE_VISIBLE, p_toggle, Vec::new()),
            Self::item_value(MENU_OPEN_SETTINGS, p_settings, Vec::new()),
            Self::item_value(MENU_TOGGLE_COMPACT, p_compact, Vec::new()),
            Self::item_value(MENU_LAYOUT_SUBMENU, p_layouts, layout_children),
            Self::item_value(MENU_SEPARATOR, p_sep, Vec::new()),
            Self::item_value(MENU_QUIT, p_quit, Vec::new()),
        ];

        (snapshot.revision, (MENU_ROOT, root_props, children))
    }
}

#[interface(name = "com.canonical.dbusmenu")]
impl DbusMenu {
    #[zbus(property)]
    fn version(&self) -> u32 {
        3
    }

    #[zbus(property)]
    fn text_direction(&self) -> &'static str {
        "ltr"
    }

    #[zbus(property)]
    fn status(&self) -> &'static str {
        "normal"
    }

    fn get_layout(
        &self,
        _parent_id: i32,
        _recursion_depth: i32,
        _property_names: Vec<String>,
    ) -> (u32, MenuItemTuple) {
        self.build_root_menu()
    }

    fn get_group_properties(
        &self,
        _ids: Vec<i32>,
        _property_names: Vec<String>,
    ) -> Vec<(i32, HashMap<String, OwnedVariant>)> {
        Vec::new()
    }

    fn get_property(&self, _id: i32, _name: &str) -> OwnedVariant {
        Self::prop_str("")
    }

    fn event(&self, id: i32, event_id: &str, _data: Variant<'_>, _timestamp: u32) {
        if event_id != "clicked" {
            return;
        }
        match id {
            MENU_TOGGLE_VISIBLE => (self.on_event)(PlatformEvent::ToggleVisible),
            MENU_OPEN_SETTINGS => (self.on_event)(PlatformEvent::OpenSettings),
            MENU_TOGGLE_COMPACT => (self.on_event)(PlatformEvent::ToggleCompactStyle),
            MENU_QUIT => (self.on_event)(PlatformEvent::Quit),
            n if n >= MENU_LAYOUT_BASE => {
                let idx = (n - MENU_LAYOUT_BASE) as usize;
                if let Some(code) = BUILTIN_LAYOUT_CODES.get(idx) {
                    (self.on_event)(PlatformEvent::SetLayout((*code).to_owned()));
                }
            }
            _ => {}
        }
    }

    fn about_to_show(&self, _id: i32) -> bool {
        true
    }
}

#[proxy(
    interface = "org.kde.StatusNotifierWatcher",
    default_service = "org.kde.StatusNotifierWatcher",
    default_path = "/StatusNotifierWatcher"
)]
trait StatusNotifierWatcher {
    fn register_status_notifier_item(&self, service: &str) -> zbus::Result<()>;
}

/// Register the `StatusNotifierItem` object with the desktop's watcher (Plasma, Waybar, etc.).
pub async fn register_with_watcher(conn: &Connection) {
    let Ok(proxy) = StatusNotifierWatcherProxy::new(conn).await else {
        return;
    };
    if let Err(err) = proxy.register_status_notifier_item(SNI_OBJECT_PATH).await {
        debug!("StatusNotifierWatcher registration unavailable: {err}");
    } else {
        info!("Registered wlboard StatusNotifierItem tray icon");
    }
}

/// Generate a crisp 24x24 ARGB32 network-byte-order fallback tray icon matching `wlboard.svg`.
fn build_tray_icon_argb32() -> (i32, i32, Vec<u8>) {
    const SIZE: i32 = 24;
    let mut buf = vec![0u8; (SIZE * SIZE * 4) as usize];

    let mut set_px = |x: i32, y: i32, a: u8, r: u8, g: u8, b: u8| {
        if (0..SIZE).contains(&x) && (0..SIZE).contains(&y) {
            let idx = ((y * SIZE + x) * 4) as usize;
            buf[idx] = a;
            buf[idx + 1] = r;
            buf[idx + 2] = g;
            buf[idx + 3] = b;
        }
    };

    // Draw dark rounded chassis (2..22, 4..20) with crisp white 1px outer frame
    for y in 4..20 {
        for x in 2..22 {
            let corner = (x == 2 || x == 21) && (y == 4 || y == 19);
            if corner {
                continue;
            }
            let border = x == 2 || x == 21 || y == 4 || y == 19;
            if border {
                set_px(x, y, 255, 255, 255, 255);
            } else {
                set_px(x, y, 245, 0x1E, 0x20, 0x26);
            }
        }
    }

    // Two rows of subtle light keycap blocks
    for &ky in &[7, 11] {
        for kx in [5, 9, 13, 17] {
            for dy in 0..2 {
                for dx in 0..2 {
                    set_px(kx + dx, ky + dy, 255, 0xD8, 0xDE, 0xE9);
                }
            }
        }
    }

    // Cobalt blue accent spacebar
    for y in 15..17 {
        for x in 6..18 {
            set_px(x, y, 255, 0x2F, 0x81, 0xF7);
        }
    }

    (SIZE, SIZE, buf)
}
