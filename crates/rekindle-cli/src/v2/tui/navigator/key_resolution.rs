//! Key event resolution — the single decision point for what a keypress means.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::super::action::Action;
use super::super::focus::FocusId;
use super::super::keybinds::{KeymapContext, KeymapStore};
use super::Navigator;
use crate::v2::views::ViewKind;

/// Result of resolving a key event through the Navigator.
pub enum KeyResolution {
    /// Key resolved to an action — dispatch via App::process_action.
    Action(Action),
    /// Key should be forwarded to the view's content handler.
    ForwardToView(KeyEvent),
    /// Key consumed internally (overlay dismissed, mode changed).
    Consumed,
    /// Force quit — Ctrl+C.
    ForceQuit,
}

impl Navigator {
    /// Resolve a key event into a navigation outcome.
    pub fn handle_key(
        &mut self,
        key: KeyEvent,
        keymap: &KeymapStore,
        _use_unicode: bool,
    ) -> KeyResolution {
        // Priority 1: Ctrl+C always force-quits
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return KeyResolution::ForceQuit;
        }

        // Priority 2: Overlay intercepts all keys
        if let Some(ref overlay) = self.overlay {
            return self.handle_overlay_key(key, overlay.clone());
        }

        // Priority 3: Input mode
        if self.input_mode {
            return self.handle_input_key(key, keymap);
        }

        // Priority 4: Ctrl+1-9 direct tab selection
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            if let KeyCode::Char(c @ '1'..='9') = key.code {
                let index = (c as usize) - ('1' as usize);
                self.tab_bar.select(index);
                return KeyResolution::Action(Action::Render);
            }
        }

        // Priority 5: Dashboard grid navigation overrides
        if matches!(self.current_view_kind(), ViewKind::Dashboard) {
            if let Some(resolution) = self.handle_dashboard_key(key) {
                return resolution;
            }
        }

        // Priority 6: Global keymap
        let context = KeymapContext::Default;
        if let Some(action) = keymap.classify(key, context) {
            return KeyResolution::Action(action);
        }

        // Priority 7: Forward to view
        KeyResolution::ForwardToView(key)
    }

    fn handle_overlay_key(&mut self, key: KeyEvent, overlay: super::super::action::OverlayKind) -> KeyResolution {
        use super::super::action::OverlayKind;
        match overlay {
            OverlayKind::Help => {
                self.overlay = None;
                KeyResolution::Consumed
            }
            OverlayKind::ConfirmAction { action, .. } => {
                match key.code {
                    KeyCode::Char('y') | KeyCode::Enter => {
                        self.overlay = None;
                        KeyResolution::Action(*action)
                    }
                    KeyCode::Char('n') | KeyCode::Esc => {
                        self.overlay = None;
                        KeyResolution::Consumed
                    }
                    _ => KeyResolution::Consumed,
                }
            }
            OverlayKind::Search(_) => KeyResolution::Consumed,
        }
    }

    fn handle_input_key(&mut self, key: KeyEvent, keymap: &KeymapStore) -> KeyResolution {
        if key.code == KeyCode::Esc {
            self.input_mode = false;
            return KeyResolution::Consumed;
        }
        if let Some(action) = keymap.classify(key, KeymapContext::Input) {
            return KeyResolution::Action(action);
        }
        KeyResolution::ForwardToView(key)
    }

    fn handle_dashboard_key(&mut self, key: KeyEvent) -> Option<KeyResolution> {
        let focus = self.views.current_mut().focus_ring();
        let current = focus.current();

        match key.code {
            KeyCode::Char('h') => {
                match current {
                    FocusId::DashNode => focus.set(FocusId::DashIdentity),
                    FocusId::FriendList => focus.set(FocusId::ChannelTree),
                    _ => {}
                }
                Some(KeyResolution::Consumed)
            }
            KeyCode::Char('l') => {
                match current {
                    FocusId::DashIdentity => focus.set(FocusId::DashNode),
                    FocusId::ChannelTree => focus.set(FocusId::FriendList),
                    _ => {}
                }
                Some(KeyResolution::Consumed)
            }
            KeyCode::Char('j') | KeyCode::Down => {
                match current {
                    FocusId::DashIdentity => focus.set(FocusId::ChannelTree),
                    FocusId::DashNode => focus.set(FocusId::FriendList),
                    _ => {}
                }
                Some(KeyResolution::Consumed)
            }
            KeyCode::Char('k') | KeyCode::Up => {
                match current {
                    FocusId::ChannelTree => focus.set(FocusId::DashIdentity),
                    FocusId::FriendList => focus.set(FocusId::DashNode),
                    _ => {}
                }
                Some(KeyResolution::Consumed)
            }
            KeyCode::Enter => {
                Some(match current {
                    FocusId::FriendList => KeyResolution::Action(Action::ShowFriendList),
                    FocusId::DashNode | FocusId::ChannelTree => KeyResolution::Action(Action::ShowDoctor),
                    FocusId::DashIdentity => KeyResolution::Action(Action::ShowIdentitySettings),
                    _ => KeyResolution::Consumed,
                })
            }
            KeyCode::Char('d') => Some(KeyResolution::Action(Action::ShowDmInbox)),
            KeyCode::Char('f') => Some(KeyResolution::Action(Action::ShowFriendList)),
            _ => None,
        }
    }
}
