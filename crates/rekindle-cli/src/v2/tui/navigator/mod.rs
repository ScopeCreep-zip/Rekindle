//! Unified navigation system — single owner of all navigation state.

mod key_resolution;
mod navigation;
mod split_pane;

pub use key_resolution::KeyResolution;
pub use split_pane::SplitPaneState;

use super::action::OverlayKind;
use super::components::tab_bar::state::TabBarState;
use crate::v2::views::{View, ViewKind, ViewQuery, ViewRegistry};

/// Single owner of all navigation state.
pub struct Navigator {
    /// View stack — Dashboard is always at index 0.
    view_stack: Vec<ViewKind>,
    /// Tab bar state.
    pub tab_bar: TabBarState,
    /// View registry — owns view instances.
    views: ViewRegistry,
    /// Text input mode active.
    input_mode: bool,
    /// Active modal overlay.
    overlay: Option<OverlayKind>,
    /// Sidebar visibility.
    sidebar_visible: bool,
    /// Tab bar header row for click hit testing.
    pub tab_bar_row: u16,
}

impl Navigator {
    /// Create with Dashboard as initial view.
    pub fn new(tabs: Vec<super::components::tab_bar::state::Tab>, use_unicode: bool) -> Self {
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

    /// The current view kind enum (for matching in breadcrumb, tab sync, etc.).
    pub fn current_view_kind(&self) -> &ViewKind {
        self.view_stack.last().expect("view stack never empty")
    }

    /// Read-only access to the current view's query methods (&self).
    pub fn current_view(&self) -> &dyn ViewQuery {
        self.views.current_ref()
    }

    /// Mutable access to the current view (&mut self).
    pub fn current_view_mut(&mut self) -> &mut dyn View {
        self.views.current_mut()
    }

    pub fn input_mode(&self) -> bool { self.input_mode }
    pub fn overlay(&self) -> Option<&OverlayKind> { self.overlay.as_ref() }
    pub fn dashboard_mut(&mut self) -> &mut crate::v2::views::dashboard::DashboardView { self.views.dashboard_mut() }

    pub fn forward_event_to_all_views(
        &mut self,
        event: &rekindle_types::subscription_events::SubscriptionEvent,
    ) {
        let _ = self.views.forward_event_to_all(event);
    }
}
