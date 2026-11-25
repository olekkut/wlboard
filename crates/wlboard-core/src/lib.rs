//! Pure, dependency-free core logic for `wlboard`.

pub mod catalog;
pub mod config;
pub mod keycode;
pub mod layout;
pub mod modifiers;
pub mod panel;
pub mod presets;
pub mod state;
pub mod theme;
pub mod touch;

pub use config::{AppConfig, BehaviorSettings, KeycapStyle, LayoutSettings, WindowGeometry};
pub use keycode::{IconId, KeyCode, KeyRole, ModifierKind};
pub use layout::{
    BUILTIN_LAYOUT_CODES, CompactPage, DEFAULT_VISIBLE_LAYOUT_CODES, KeyAction, KeySpec,
    KeyboardStyle, LayoutDefinition, LayoutRegistry, media_control_row,
};
pub use modifiers::{ModifierLevel, ModifierTracker};
pub use panel::{
    GridKeyAction, GridKeySpec, PANEL_COMPACT_TV, PANEL_NUMPAD_NAV, PANEL_SHORTCUTS_MEDIA,
    PANEL_STANDARD, PanelDefinition, PanelError, PanelStore, ResolvedGridKey,
    build_compact_tv_panel, build_numpad_nav_panel, build_shortcuts_media_panel,
    build_standard_panel,
};
pub use presets::{PresetError, PresetStore, UserPreset};
pub use state::{InputCommand, KeyboardState};
pub use theme::{
    ACCENT_SWATCHES, AccentSwatch, AppearanceSettings, Color, ColorSlot, ResolvedPalette,
    ThemeBase, ThemeColors,
};
pub use touch::{
    ActiveContact, ContactId, MIN_WINDOW_HEIGHT, MIN_WINDOW_WIDTH, MultiTouchTracker, OutputBounds,
    Point, Rect, ResizeEdge, apply_resize, clamp_move, detect_resize_edge,
};
