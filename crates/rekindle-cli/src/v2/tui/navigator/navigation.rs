//! Navigation methods — view transitions, back, quit, overlays, input mode.

use super::super::action::OverlayKind;
use super::Navigator;
use crate::v2::views::ViewKind;

impl Navigator {
    /// Navigate to a view. Pushes to stack, transitions registry, syncs tab bar.
    pub fn navigate(&mut self, target: ViewKind, use_unicode: bool) {
        if matches!(target, ViewKind::Dashboard) {
            self.view_stack.clear();
            self.view_stack.push(ViewKind::Dashboard);
        } else if self.view_stack.last() != Some(&target) {
            self.view_stack.push(target.clone());
        }
        self.views.transition(target, use_unicode);
        self.tab_bar.sync_to_view(self.views.current_kind());
        self.input_mode = false;
    }

    /// Go back one view. No-op if on Dashboard.
    pub fn back(&mut self, use_unicode: bool) {
        if self.view_stack.len() > 1 {
            self.view_stack.pop();
            let target = self.view_stack.last().expect("stack not empty").clone();
            self.views.transition(target, use_unicode);
            self.tab_bar.sync_to_view(self.views.current_kind());
            self.input_mode = false;
        }
    }

    /// Quit: if on Dashboard, return true. Otherwise go to Dashboard.
    pub fn quit(&mut self, use_unicode: bool) -> bool {
        if matches!(self.current_view_kind(), ViewKind::Dashboard) {
            true
        } else {
            self.navigate(ViewKind::Dashboard, use_unicode);
            false
        }
    }

    // ── Overlay ─────────────────────────────────────────────

    pub fn open_overlay(&mut self, kind: OverlayKind) { self.overlay = Some(kind); }
    pub fn close_overlay(&mut self) { self.overlay = None; }

    pub fn toggle_help(&mut self) {
        if self.overlay.is_some() {
            self.overlay = None;
        } else {
            self.overlay = Some(OverlayKind::Help);
        }
    }

    // ── Input mode ──────────────────────────────────────────

    pub fn enter_input_mode(&mut self) {
        let has_input = matches!(
            self.current_view_kind(),
            ViewKind::ChannelWatch { .. } | ViewKind::DmInbox | ViewKind::DmThread { .. }
        );
        if has_input {
            self.input_mode = true;
        }
    }

    pub fn exit_input_mode(&mut self) { self.input_mode = false; }

    // ── Sidebar ─────────────────────────────────────────────

    pub fn toggle_sidebar(&mut self) { self.sidebar_visible = !self.sidebar_visible; }
}
