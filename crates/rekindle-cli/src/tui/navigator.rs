//! Unified navigation system for the TUI.
//!
//! [`Navigator`] is the single owner of all navigation state: the view stack,
//! tab bar, focus state, input mode, overlay, and sidebar visibility. Every
//! view transition, back-navigation, tab switch, and focus change goes through
//! Navigator methods. No other code mutates these states directly.
//!
//! The key insight: navigation keys (h/j/k/l/Tab/Enter/Esc/q) have DIFFERENT
//! meanings depending on context. On the dashboard, h/l move between grid
//! panels. In a channel view, h is "go back" and l is "select/expand". The
//! Navigator resolves this ambiguity in one place by consulting the current
//! view before dispatching to the keymap.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::action::{Action, OverlayKind};
use super::components::tab_bar::{Tab, TabBarState};
use super::focus::FocusId;
use super::keybinds::{KeymapContext, KeymapStore};
use crate::views::{View, ViewKind, ViewRegistry};

/// Result of resolving a key event through the Navigator.
pub enum KeyResolution {
    /// Key resolved to an action — dispatch via App::process_action.
    Action(Action),
    /// Key should be forwarded to the current view's content handler.
    /// The view's handle_focused_key or component handle_key should process it.
    ForwardToView(KeyEvent),
    /// Key was consumed internally (overlay dismissed, mode changed). No action needed.
    Consumed,
    /// Force quit — Ctrl+C.
    ForceQuit,
}

/// Single owner of all navigation state.
pub struct Navigator {
    /// View stack for back-navigation. Last element is current view.
    /// Dashboard is always at index 0 and can never be popped.
    view_stack: Vec<ViewKind>,

    /// Tab bar state — selection derived from current view via sync_to_view.
    pub tab_bar: TabBarState,

    /// View registry — owns view instances.
    views: ViewRegistry,

    /// Whether the user is in text input mode.
    input_mode: bool,

    /// Active modal overlay (help, search, confirm).
    overlay: Option<OverlayKind>,

    /// Whether the sidebar is visible (channel watch view).
    sidebar_visible: bool,

    /// Row of the tab bar header — stored during draw for click hit testing.
    pub tab_bar_row: u16,
}

impl Navigator {
    /// Create a new Navigator with Dashboard as the initial view.
    pub fn new(tabs: Vec<Tab>, use_unicode: bool) -> Self {
        Self {
            view_stack: vec![ViewKind::Dashboard],
            tab_bar: TabBarState::new(tabs),
            views: ViewRegistry::new(use_unicode),
            input_mode: false,
            overlay: None,
            sidebar_visible: true,
            tab_bar_row: 0,
        }
    }

    // ── State accessors ─────────────────────────────────────────────

    /// The current view kind.
    pub fn current_view(&self) -> &ViewKind {
        self.view_stack.last().expect("view stack never empty")
    }

    /// Whether we're in text input mode.
    pub fn input_mode(&self) -> bool {
        self.input_mode
    }

    /// The active overlay, if any.
    pub fn overlay(&self) -> Option<&OverlayKind> {
        self.overlay.as_ref()
    }

    /// Mutable access to the current view.
    pub fn current_view_mut(&mut self) -> &mut dyn View {
        self.views.current_mut()
    }

    /// Direct access to dashboard for identity setup.
    pub fn dashboard_mut(&mut self) -> &mut crate::views::dashboard::DashboardView {
        self.views.dashboard_mut()
    }

    /// Forward a subscription event to all instantiated views.
    pub fn forward_event_to_all_views(
        &mut self,
        event: &rekindle_types::subscription_events::SubscriptionEvent,
    ) -> anyhow::Result<()> {
        self.views.forward_event_to_all(event)
    }

    // ── Navigation ──────────────────────────────────────────────────

    /// Navigate to a view. Pushes to the view stack, transitions the registry,
    /// syncs the tab bar, and clears input mode. This is the ONLY way to
    /// change the current view.
    pub fn navigate(&mut self, target: ViewKind, use_unicode: bool) {
        // Navigating to Dashboard clears the stack
        if matches!(target, ViewKind::Dashboard) {
            self.view_stack.clear();
            self.view_stack.push(ViewKind::Dashboard);
        } else {
            // Don't push duplicates
            if self.view_stack.last() != Some(&target) {
                self.view_stack.push(target.clone());
            }
        }
        self.views.transition(target, use_unicode);
        self.tab_bar.sync_to_view(self.views.current_kind());
        self.input_mode = false;
    }

    /// Go back one view in the stack. If on Dashboard, no-op.
    pub fn back(&mut self, use_unicode: bool) {
        if self.view_stack.len() > 1 {
            self.view_stack.pop();
            let target = self.view_stack.last().expect("stack not empty").clone();
            self.views.transition(target, use_unicode);
            self.tab_bar.sync_to_view(self.views.current_kind());
            self.input_mode = false;
        }
    }

    /// Quit: if on Dashboard, return true (should_quit). Otherwise, go to Dashboard.
    pub fn quit(&mut self, use_unicode: bool) -> bool {
        if matches!(self.current_view(), ViewKind::Dashboard) {
            true
        } else {
            self.navigate(ViewKind::Dashboard, use_unicode);
            false
        }
    }

    // ── Overlay management ──────────────────────────────────────────

    pub fn open_overlay(&mut self, kind: OverlayKind) {
        self.overlay = Some(kind);
    }

    pub fn close_overlay(&mut self) {
        self.overlay = None;
    }

    pub fn toggle_help(&mut self) {
        if self.overlay.is_some() {
            self.overlay = None;
        } else {
            self.overlay = Some(OverlayKind::Help);
        }
    }

    // ── Input mode ──────────────────────────────────────────────────

    /// Enter input mode. Only allowed on views that have an InputBox.
    pub fn enter_input_mode(&mut self) {
        let has_input = matches!(
            self.current_view(),
            ViewKind::ChannelWatch { .. } | ViewKind::DmInbox | ViewKind::DmThread { .. }
        );
        if has_input {
            self.input_mode = true;
        }
    }

    pub fn exit_input_mode(&mut self) {
        self.input_mode = false;
    }

    // ── Sidebar ─────────────────────────────────────────────────────

    pub fn toggle_sidebar(&mut self) {
        self.sidebar_visible = !self.sidebar_visible;
    }

    // ── Key resolution ──────────────────────────────────────────────

    /// Resolve a key event into a navigation outcome.
    ///
    /// This is the single decision point for what a keypress means.
    /// It replaces the distributed logic across event_to_action,
    /// process_action, handle_focused_key, and view update methods.
    pub fn handle_key(
        &mut self,
        key: KeyEvent,
        keymap: &KeymapStore,
        _use_unicode: bool,
    ) -> KeyResolution {
        // Priority 1: Ctrl+C always force-quits
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && key.code == KeyCode::Char('c')
        {
            return KeyResolution::ForceQuit;
        }

        // Priority 2: Overlay intercepts all keys
        if let Some(ref overlay) = self.overlay {
            return self.handle_overlay_key(key, overlay.clone());
        }

        // Priority 3: Input mode — only Esc and keymap-classified keys
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

        // Priority 5: Dashboard-specific key overrides
        // On the dashboard, h/j/k/l navigate the 2x2 grid.
        // These keys have DIFFERENT global meanings (Back/ScrollDown/ScrollUp/Select)
        // that must be overridden on the dashboard.
        if matches!(self.current_view(), ViewKind::Dashboard) {
            if let Some(resolution) = self.handle_dashboard_key(key) {
                return resolution;
            }
        }

        // Priority 6: Global keymap classification
        let context = if self.input_mode {
            KeymapContext::Input
        } else {
            KeymapContext::Default
        };

        if let Some(action) = keymap.classify(key, context) {
            return KeyResolution::Action(action);
        }

        // Priority 7: Forward unclassified keys to the view's content handler
        KeyResolution::ForwardToView(key)
    }

    /// Handle keys when an overlay is active.
    fn handle_overlay_key(&mut self, key: KeyEvent, overlay: OverlayKind) -> KeyResolution {
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
            OverlayKind::Search(_) => {
                // Search overlay handles its own keys in the App layer
                KeyResolution::Consumed
            }
        }
    }

    /// Handle keys in input mode.
    fn handle_input_key(&mut self, key: KeyEvent, keymap: &KeymapStore) -> KeyResolution {
        if key.code == KeyCode::Esc {
            self.input_mode = false;
            return KeyResolution::Consumed;
        }

        // Try keymap for input-context bindings (Enter → InputSubmit)
        if let Some(action) = keymap.classify(key, KeymapContext::Input) {
            return KeyResolution::Action(action);
        }

        // Forward unclassified keys to the view (characters → InputBox textarea)
        KeyResolution::ForwardToView(key)
    }

    /// Dashboard-specific key handling for 2x2 grid navigation.
    ///
    /// Returns Some if the key was handled, None to fall through to global keymap.
    fn handle_dashboard_key(&mut self, key: KeyEvent) -> Option<KeyResolution> {
        let focus = self.views.current_mut().focus_ring();
        let current = focus.current();

        match key.code {
            // h = move left in grid
            KeyCode::Char('h') => {
                match current {
                    FocusId::DoctorList => focus.set(FocusId::CommunityInfoPanel),
                    FocusId::FriendList => focus.set(FocusId::ChannelTree),
                    _ => {} // already left column
                }
                Some(KeyResolution::Consumed)
            }
            // l = move right in grid
            KeyCode::Char('l') => {
                match current {
                    FocusId::CommunityInfoPanel => focus.set(FocusId::DoctorList),
                    FocusId::ChannelTree => focus.set(FocusId::FriendList),
                    _ => {} // already right column
                }
                Some(KeyResolution::Consumed)
            }
            // j = move down in grid
            KeyCode::Char('j') | KeyCode::Down => {
                match current {
                    FocusId::CommunityInfoPanel => focus.set(FocusId::ChannelTree),
                    FocusId::DoctorList => focus.set(FocusId::FriendList),
                    _ => {} // already bottom row
                }
                Some(KeyResolution::Consumed)
            }
            // k = move up in grid
            KeyCode::Char('k') | KeyCode::Up => {
                match current {
                    FocusId::ChannelTree => focus.set(FocusId::CommunityInfoPanel),
                    FocusId::FriendList => focus.set(FocusId::DoctorList),
                    _ => {} // already top row
                }
                Some(KeyResolution::Consumed)
            }
            // Enter = navigate into the focused panel
            KeyCode::Enter => {
                Some(match current {
                    FocusId::FriendList => KeyResolution::Action(Action::ShowFriendList),
                    FocusId::DoctorList | FocusId::ChannelTree =>
                        KeyResolution::Action(Action::ShowDoctor),
                    _ => KeyResolution::Consumed,
                })
            }
            // Dashboard quick shortcuts: d=DMs, f=Friends
            KeyCode::Char('d') => Some(KeyResolution::Action(Action::ShowDmInbox)),
            KeyCode::Char('f') => Some(KeyResolution::Action(Action::ShowFriendList)),
            // All other keys fall through to global keymap
            _ => None,
        }
    }
}
