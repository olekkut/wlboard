//! Unified pure application state orchestrating config, layouts, modifiers, presets, and multi-touch input.

use std::time::Instant;

use crate::config::AppConfig;
use crate::keycode::{KeyCode, KeyRole};
use crate::layout::{
    CompactPage, KeyAction, KeySpec, KeyboardStyle, LayoutDefinition, LayoutRegistry,
};
use crate::modifiers::ModifierTracker;
use crate::presets::{PresetStore, UserPreset};
use crate::touch::{ActiveContact, ContactId, MultiTouchTracker};

/// Pure input commands emitted by `KeyboardState` for the platform input backend to execute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputCommand {
    Press(KeyCode),
    Release(KeyCode),
    Tap(KeyCode),
    SyncModifiers { depressed: u32, locked: u32 },
    ReleaseAll,
}

/// Complete pure state for the virtual keyboard overlay.
#[derive(Debug, Clone)]
pub struct KeyboardState {
    pub config: AppConfig,
    pub registry: LayoutRegistry,
    pub modifiers: ModifierTracker,
    pub touches: MultiTouchTracker,
    pub compact_page: CompactPage,
    pub hovered_key_id: Option<String>,
    pub visible: bool,
    pub settings_open: bool,
    pub presets: Vec<UserPreset>,
    pub input_backend_status: String,
    pub layout_sync_status: String,
}

impl KeyboardState {
    #[must_use]
    pub fn new(mut config: AppConfig, preset_store: &PresetStore) -> Self {
        let mut registry = LayoutRegistry::new();
        let _ = registry.load_overrides_from(&AppConfig::layouts_dir());

        if let Some(norm) = registry.normalize_code(&config.layout.active) {
            config.layout.active = norm;
        } else {
            config.layout.active = "en".to_owned();
        }
        config.clamp();

        let presets = preset_store.list().unwrap_or_default();
        let visible = !config.behavior.start_minimized;

        Self {
            config,
            registry,
            modifiers: ModifierTracker::new(),
            touches: MultiTouchTracker::new(),
            compact_page: CompactPage::Letters,
            hovered_key_id: None,
            visible,
            settings_open: false,
            presets,
            input_backend_status: "Initializing...".to_owned(),
            layout_sync_status: "Local layout switching".to_owned(),
        }
    }

    #[must_use]
    pub fn active_layout(&self) -> &LayoutDefinition {
        self.registry.get(&self.config.layout.active)
    }

    #[must_use]
    pub fn current_rows(&self) -> Vec<Vec<KeySpec>> {
        self.registry.build_rows(
            &self.config.layout.active,
            self.config.layout.style,
            self.compact_page,
        )
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
    }

    pub fn toggle_compact_page(&mut self) {
        self.compact_page = match self.compact_page {
            CompactPage::Letters => CompactPage::SymbolsNav,
            CompactPage::SymbolsNav => CompactPage::Letters,
        };
    }

    /// Handle key press from pointer or touch contact and emit pure `InputCommand`s.
    pub fn press_key(
        &mut self,
        contact: ContactId,
        spec: &KeySpec,
        right_click: bool,
    ) -> Vec<InputCommand> {
        let mut cmds = Vec::new();
        match spec.action {
            KeyAction::ToggleCompactPage => {
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
            KeyAction::Key(kc) => {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn right_click_character_wraps_with_temporary_shift() {
        let store = PresetStore::with_dir(std::env::temp_dir().join("wlboard_state_empty"));
        let mut state = KeyboardState::new(AppConfig::default(), &store);
        let rows = state.current_rows();
        let key_a = rows
            .iter()
            .flatten()
            .find(|k| k.id == "KEY_A")
            .expect("KEY_A present")
            .clone();

        let press = state.press_key(ContactId::Pointer, &key_a, true);
        assert!(press.contains(&InputCommand::Press(KeyCode::LEFTSHIFT)));
        assert!(press.contains(&InputCommand::Press(KeyCode::A)));

        let release = state.release_contact(ContactId::Pointer, Instant::now());
        assert!(release.contains(&InputCommand::Release(KeyCode::A)));
        assert!(release.contains(&InputCommand::Release(KeyCode::LEFTSHIFT)));
    }
}
