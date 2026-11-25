//! Pure, dependency-free core logic for `wlboard`.

pub mod config;
pub mod keycode;
pub mod layout;
pub mod modifiers;
pub mod presets;
pub mod state;
pub mod theme;
pub mod touch;

pub use config::{AppConfig, BehaviorSettings, LayoutSettings, WindowGeometry};
pub use keycode::{IconId, KeyCode, KeyRole, ModifierKind};
pub use layout::{
    BUILTIN_LAYOUT_CODES, CompactPage, KeyAction, KeySpec, KeyboardStyle, LayoutDefinition,
    LayoutRegistry, media_control_row,
};
pub use modifiers::{ModifierLevel, ModifierTracker};
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
