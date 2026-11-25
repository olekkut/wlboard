//! Unified pure application state orchestrating config, layouts, 2D panels, modifiers, presets, and multi-touch input.

use std::time::Instant;

use crate::config::AppConfig;
use crate::keycode::{KeyCode, KeyRole, ModifierKind};
use crate::layout::{CompactPage, KeyboardStyle, LayoutDefinition, LayoutRegistry};
use crate::modifiers::ModifierTracker;
use crate::panel::{
    GridKeyAction, PANEL_COMPACT_TV, PANEL_STANDARD, PanelDefinition, PanelStore, ResolvedGridKey,
    build_compact_tv_panel,
};
use crate::presets::{PresetStore, UserPreset};
use crate::touch::{ActiveContact, ContactId, MultiTouchTracker};

/// Pure input commands emitted by `KeyboardState` for the platform input backend to execute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputCommand {
    Press(KeyCode),
    Release(KeyCode),
    Tap(KeyCode),
    /// Direct UTF-8 character or snippet commit (used by Wayland Input Method mode & Text Snippets).
    TypeText(String),
    /// Multi-key chord shortcut (press modifiers -> tap key -> release modifiers).
    Shortcut {
        modifiers: Vec<ModifierKind>,
        keycode: KeyCode,
    },
    SyncModifiers {
        depressed: u32,
        locked: u32,
    },
    ReleaseAll,
}

/// Complete pure state for the virtual keyboard overlay.
#[derive(Debug, Clone)]
pub struct KeyboardState {
    pub config: AppConfig,
    pub registry: LayoutRegistry,
    pub panels: Vec<PanelDefinition>,
    pub modifiers: ModifierTracker,
    pub touches: MultiTouchTracker,
    pub compact_page: CompactPage,
    pub hovered_key_id: Option<String>,
    pub visible: bool,
    pub settings_open: bool,
    pub panel_editor_open: bool,
    pub input_method_mode: bool,
    pub presets: Vec<UserPreset>,
    pub input_backend_status: String,
    pub layout_sync_status: String,
}

impl KeyboardState {
    #[must_use]
    pub fn new(
        mut config: AppConfig,
        preset_store: &PresetStore,
        panel_store: &PanelStore,
    ) -> Self {
        let mut registry = LayoutRegistry::new();
        let _ = registry.load_overrides_from(&AppConfig::layouts_dir());

        if let Some(norm) = registry.normalize_code(&config.layout.active) {
            config.layout.active = norm;
        } else {
            config.layout.active = "en".to_owned();
        }
        // Keep style & active_panel consistent if user upgraded from v0.1.0 config
        if config.layout.style == KeyboardStyle::CompactTv
            && config.layout.active_panel == PANEL_STANDARD
        {
            config.layout.active_panel = PANEL_COMPACT_TV.to_owned();
        }
        config.clamp();

        let panels = panel_store.load_all(registry.get("en"));
        if !panels.iter().any(|p| p.id == config.layout.active_panel) {
            config.layout.active_panel = PANEL_STANDARD.to_owned();
        }

        let presets = preset_store.list().unwrap_or_default();
        let visible = !config.behavior.start_minimized;

        Self {
            config,
            registry,
            panels,
            modifiers: ModifierTracker::new(),
            touches: MultiTouchTracker::new(),
            compact_page: CompactPage::Letters,
            hovered_key_id: None,
            visible,
            settings_open: false,
            panel_editor_open: false,
            input_method_mode: false,
            presets,
            input_backend_status: "Initializing...".to_owned(),
            layout_sync_status: "Local layout switching".to_owned(),
        }
    }

    #[must_use]
    pub fn active_layout(&self) -> &LayoutDefinition {
        self.registry.get(&self.config.layout.active)
    }

    /// Languages enabled in the Fast Language Switcher in the user's chosen order.
    #[must_use]
    pub fn visible_layouts(&self) -> Vec<&LayoutDefinition> {
        let mut out = Vec::new();
        for code in &self.config.layout.visible_languages {
            if let Some(norm) = self.registry.normalize_code(code) {
                let def = self.registry.get(&norm);
                if !out.iter().any(|d: &&LayoutDefinition| d.code == def.code) {
                    out.push(def);
                }
            }
        }
        if out.is_empty() {
            out.push(self.registry.get("en"));
        }
        out
    }

    /// Toggle whether a language appears in the top Fast Language Switcher bar (keeps at least 1 enabled).
    pub fn toggle_visible_language(&mut self, code: &str) -> bool {
        let Some(norm) = self.registry.normalize_code(code) else {
            return false;
        };
        if let Some(pos) = self
            .config
            .layout
            .visible_languages
            .iter()
            .position(|c| c == &norm)
        {
            if self.config.layout.visible_languages.len() <= 1 {
                return false;
            }
            self.config.layout.visible_languages.remove(pos);
            if self.config.layout.active == norm
                && let Some(first) = self.config.layout.visible_languages.first()
            {
                self.config.layout.active = first.clone();
            }
            true
        } else {
            self.config.layout.visible_languages.push(norm);
            true
        }
    }

    /// Move a visible language left (`delta = -1`) or right (`delta = 1`) in the Fast Switcher order.
    pub fn move_visible_language(&mut self, code: &str, delta: isize) -> bool {
        let Some(pos) = self
            .config
            .layout
            .visible_languages
            .iter()
            .position(|c| c == code)
        else {
            return false;
        };
        let target = pos as isize + delta;
        if target < 0 || target as usize >= self.config.layout.visible_languages.len() {
            return false;
        }
        self.config
            .layout
            .visible_languages
            .swap(pos, target as usize);
        true
    }

    /// Return the active 2D `PanelDefinition` (dynamically selecting `Compact TV` `ABC`/`123` unless customized).
    #[must_use]
    pub fn active_panel_def(&self) -> PanelDefinition {
        let id = &self.config.layout.active_panel;
        if id == PANEL_COMPACT_TV {
            // If the user has not customized compact_tv, dynamically rebuild for the active language's alpha rows & page
            if self.compact_page == CompactPage::SymbolsNav {
                return build_compact_tv_panel(self.active_layout(), CompactPage::SymbolsNav);
            }
            return build_compact_tv_panel(self.active_layout(), CompactPage::Letters);
        }
        self.panels
            .iter()
            .find(|p| &p.id == id)
            .cloned()
            .or_else(|| self.panels.first().cloned())
            .unwrap_or_else(crate::panel::build_standard_panel)
    }

    /// Return resolved language-aware 2D grid keys for the active panel.
    #[must_use]
    pub fn active_resolved_keys(&self) -> (PanelDefinition, Vec<ResolvedGridKey>) {
        let panel = self.active_panel_def();
        let layout = self.active_layout();
        let resolved = panel
            .keys
            .iter()
            .cloned()
            .map(|k| ResolvedGridKey::resolve(k, layout))
            .collect();
        (panel, resolved)
    }

    /// Switch active layout and return whether it actually changed.
    pub fn set_layout(&mut self, code: &str) -> bool {
        let Some(norm) = self.registry.normalize_code(code) else {
            return false;
        };
        if self.config.layout.active == norm {
            return false;
        }
        self.config.layout.active = norm;
        true
    }

    pub fn set_style(&mut self, style: KeyboardStyle) {
        self.config.layout.style = style;
        self.compact_page = CompactPage::Letters;
        self.config.layout.active_panel = match style {
            KeyboardStyle::Standard => PANEL_STANDARD.to_owned(),
            KeyboardStyle::CompactTv => PANEL_COMPACT_TV.to_owned(),
        };
    }

    pub fn set_panel(&mut self, panel_id: &str) -> bool {
        if !self.panels.iter().any(|p| p.id == panel_id) {
            return false;
        }
        self.config.layout.active_panel = panel_id.to_owned();
        if panel_id == PANEL_COMPACT_TV {
            self.config.layout.style = KeyboardStyle::CompactTv;
            self.compact_page = CompactPage::Letters;
        } else if panel_id == PANEL_STANDARD {
            self.config.layout.style = KeyboardStyle::Standard;
        }
        true
    }

    /// Cycle cleanly to the next panel in the collection (for top-bar Panel Switcher click).
    pub fn cycle_panel(&mut self) {
        if self.panels.is_empty() {
            return;
        }
        let cur = self
            .panels
            .iter()
            .position(|p| p.id == self.config.layout.active_panel)
            .unwrap_or(0);
        let next_id = self.panels[(cur + 1) % self.panels.len()].id.clone();
        self.set_panel(&next_id);
    }

    pub fn toggle_compact_page(&mut self) {
        self.compact_page = match self.compact_page {
            CompactPage::Letters => CompactPage::SymbolsNav,
            CompactPage::SymbolsNav => CompactPage::Letters,
        };
    }

    /// Handle key press on a resolved 2D grid key and emit pure `InputCommand`s.
    pub fn press_grid_key(
        &mut self,
        contact: ContactId,
        resolved: &ResolvedGridKey,
        right_click: bool,
    ) -> Vec<InputCommand> {
        let mut cmds = Vec::new();
        let spec = &resolved.grid;

        match &spec.action {
            GridKeyAction::ToggleCompactPage => {
                self.touches.insert(
                    contact,
                    ActiveContact {
                        key_id: spec.id.clone(),
                        keycode: None,
                        modifier: None,
                        right_click_shift: false,
                        chording_used: false,
                    },
                );
                self.toggle_compact_page();
            }
            GridKeyAction::SwitchPanel { panel_id } => {
                self.touches.insert(
                    contact,
                    ActiveContact {
                        key_id: spec.id.clone(),
                        keycode: None,
                        modifier: None,
                        right_click_shift: false,
                        chording_used: false,
                    },
                );
                self.set_panel(panel_id);
            }
            GridKeyAction::TextSnippet { text } => {
                self.touches.insert(
                    contact,
                    ActiveContact {
                        key_id: spec.id.clone(),
                        keycode: None,
                        modifier: None,
                        right_click_shift: false,
                        chording_used: false,
                    },
                );
                cmds.push(InputCommand::TypeText(text.clone()));
            }
            GridKeyAction::Shortcut { modifiers, keycode } => {
                self.touches.insert(
                    contact,
                    ActiveContact {
                        key_id: spec.id.clone(),
                        keycode: None,
                        modifier: None,
                        right_click_shift: false,
                        chording_used: false,
                    },
                );
                cmds.push(InputCommand::Shortcut {
                    modifiers: modifiers.clone(),
                    keycode: *keycode,
                });
            }
            GridKeyAction::LayoutKey { keycode } | GridKeyAction::FixedKey { keycode } => {
                let kc = *keycode;
                if let KeyRole::Modifier(mod_kind) = spec.role {
                    let tr = self.modifiers.begin_hold(mod_kind);
                    for c in tr.press_codes {
                        cmds.push(InputCommand::Press(c));
                    }
                    self.touches.insert(
                        contact,
                        ActiveContact {
                            key_id: spec.id.clone(),
                            keycode: Some(kc),
                            modifier: Some(mod_kind),
                            right_click_shift: false,
                            chording_used: false,
                        },
                    );
                } else if self.input_method_mode
                    && spec.role == KeyRole::Character
                    && !self.modifiers.is_active(ModifierKind::Ctrl)
                    && !self.modifiers.is_active(ModifierKind::Alt)
                    && !self.modifiers.is_active(ModifierKind::Super)
                {
                    // In Wayland Input Method mode (e.g. Lockscreen / Plasma Virtual Keyboard),
                    // commit exact UTF-8 glyph directly so ANY language works regardless of KDE layout!
                    let shift = self.modifiers.effective_shift() || right_click;
                    let upper = self.modifiers.effective_uppercase() || right_click;
                    let altgr = self.modifiers.effective_altgr();
                    let (text, _) = resolved.display_labels(shift, upper, altgr);
                    cmds.push(InputCommand::TypeText(text.to_owned()));
                    self.touches.insert(
                        contact,
                        ActiveContact {
                            key_id: spec.id.clone(),
                            keycode: None,
                            modifier: None,
                            right_click_shift: false,
                            chording_used: false,
                        },
                    );
                    let consumed = self.modifiers.consume_one_shot();
                    for c in consumed.release_codes {
                        cmds.push(InputCommand::Release(c));
                    }
                } else {
                    let use_rc_shift = right_click
                        && spec.role == KeyRole::Character
                        && !self.modifiers.effective_shift();
                    if use_rc_shift {
                        cmds.push(InputCommand::Press(KeyCode::LEFTSHIFT));
                    }
                    cmds.push(InputCommand::Press(kc));
                    self.touches.insert(
                        contact,
                        ActiveContact {
                            key_id: spec.id.clone(),
                            keycode: Some(kc),
                            modifier: None,
                            right_click_shift: use_rc_shift,
                            chording_used: false,
                        },
                    );
                }
            }
        }

        let (depressed, locked) = self.modifiers.xkb_masks();
        cmds.push(InputCommand::SyncModifiers { depressed, locked });
        cmds
    }

    /// Handle pointer or touch release and emit pure `InputCommand`s.
    pub fn release_contact(&mut self, contact: ContactId, now: Instant) -> Vec<InputCommand> {
        let Some(active) = self.touches.remove(contact) else {
            return Vec::new();
        };

        let mut cmds = Vec::new();
        if let Some(mod_kind) = active.modifier {
            let tr = self.modifiers.end_hold(
                mod_kind,
                active.chording_used,
                self.config.behavior.sticky_modifier_lock,
                now,
            );
            for c in tr.tap_codes {
                cmds.push(InputCommand::Tap(c));
            }
            for c in tr.release_codes {
                cmds.push(InputCommand::Release(c));
            }
        } else if let Some(kc) = active.keycode {
            if !self.touches.has_other_contact_holding_keycode(kc) {
                cmds.push(InputCommand::Release(kc));
            }
            if active.right_click_shift {
                cmds.push(InputCommand::Release(KeyCode::LEFTSHIFT));
            }
            let consumed = self.modifiers.consume_one_shot();
            for c in consumed.release_codes {
                cmds.push(InputCommand::Release(c));
            }
        }

        let (depressed, locked) = self.modifiers.xkb_masks();
        cmds.push(InputCommand::SyncModifiers { depressed, locked });
        cmds
    }

    /// Release every held contact and active modifier (called on hide/cancel/shutdown).
    pub fn release_all(&mut self) -> Vec<InputCommand> {
        self.touches.clear();
        let _ = self.modifiers.clear_all();
        vec![
            InputCommand::ReleaseAll,
            InputCommand::SyncModifiers {
                depressed: 0,
                locked: 0,
            },
        ]
    }

    /// Refresh cached user presets from disk.
    pub fn reload_presets(&mut self, store: &PresetStore) {
        self.presets = store.list().unwrap_or_default();
    }

    /// Refresh cached panels from disk.
    pub fn reload_panels(&mut self, store: &PanelStore) {
        self.panels = store.load_all(self.registry.get("en"));
        if !self
            .panels
            .iter()
            .any(|p| p.id == self.config.layout.active_panel)
        {
            self.config.layout.active_panel = PANEL_STANDARD.to_owned();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn right_click_character_wraps_with_temporary_shift() {
        let pstore = PresetStore::with_dir(std::env::temp_dir().join("wlboard_state_empty"));
        let panstore = PanelStore::with_dir(std::env::temp_dir().join("wlboard_panels_empty"));
        let mut state = KeyboardState::new(AppConfig::default(), &pstore, &panstore);
        let (_, keys) = state.active_resolved_keys();
        let key_a = keys
            .iter()
            .find(|k| k.grid.id == "KEY_A")
            .expect("KEY_A present")
            .clone();

        let press = state.press_grid_key(ContactId::Pointer, &key_a, true);
        assert!(press.contains(&InputCommand::Press(KeyCode::LEFTSHIFT)));
        assert!(press.contains(&InputCommand::Press(KeyCode::A)));

        let release = state.release_contact(ContactId::Pointer, Instant::now());
        assert!(release.contains(&InputCommand::Release(KeyCode::A)));
        assert!(release.contains(&InputCommand::Release(KeyCode::LEFTSHIFT)));
    }

    #[test]
    fn visible_languages_can_be_filtered_and_reordered() {
        let pstore = PresetStore::with_dir(std::env::temp_dir().join("wlboard_state_empty2"));
        let panstore = PanelStore::with_dir(std::env::temp_dir().join("wlboard_panels_empty2"));
        let mut state = KeyboardState::new(AppConfig::default(), &pstore, &panstore);

        assert!(state.toggle_visible_language("ar"));
        assert!(state.toggle_visible_language("ru"));
        let codes: Vec<_> = state
            .visible_layouts()
            .into_iter()
            .map(|l| l.code.as_str())
            .collect();
        assert_eq!(codes, vec!["en", "pl", "uk"]);

        assert!(state.move_visible_language("uk", -1));
        let reordered: Vec<_> = state
            .visible_layouts()
            .into_iter()
            .map(|l| l.code.as_str())
            .collect();
        assert_eq!(reordered, vec!["en", "uk", "pl"]);
    }

    #[test]
    fn input_method_mode_commits_unicode_text_directly() {
        let pstore = PresetStore::with_dir(std::env::temp_dir().join("wlboard_state_empty3"));
        let panstore = PanelStore::with_dir(std::env::temp_dir().join("wlboard_panels_empty3"));
        let mut state = KeyboardState::new(AppConfig::default(), &pstore, &panstore);
        state.input_method_mode = true;
        state.set_layout("pl");
        let now = Instant::now();
        state.modifiers.begin_hold(ModifierKind::AltGr);
        state
            .modifiers
            .end_hold(ModifierKind::AltGr, false, true, now);

        let (_, keys) = state.active_resolved_keys();
        let key_a = keys
            .iter()
            .find(|k| k.grid.id == "KEY_A")
            .expect("KEY_A")
            .clone();
        let cmds = state.press_grid_key(ContactId::Pointer, &key_a, false);
        assert!(cmds.contains(&InputCommand::TypeText("ą".to_owned())));
    }
}
