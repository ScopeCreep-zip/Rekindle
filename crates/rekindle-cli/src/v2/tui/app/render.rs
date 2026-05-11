//! Rendering — draw(), overlays, breadcrumb, search items, tab transitions.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use ratatui::Frame;

use super::super::action::{Action, OverlayKind, SearchMode, ToastLevel};
use super::super::components::confirm_dialog;
use super::super::components::help_bar;
use super::super::components::search_overlay::SearchItem;
use super::super::components::status_bar;
use super::super::components::tab_bar;
use super::super::components::typing_indicator;
use super::super::keybinds::KeymapContext;
use super::App;
use crate::v2::helpers;
use crate::v2::views::ViewKind;

impl App {
    pub(crate) fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();

        if area.width < 40 || area.height < 10 {
            let msg = Paragraph::new("Terminal too small.\nMinimum: 40x10")
                .alignment(ratatui::layout::Alignment::Center).style(Style::new().bold());
            frame.render_widget(msg, centered_rect(area, 30, 3));
            return;
        }

        // Dynamic layout: community rail (top) and system rail (bottom) only
        // render when they have active signals, otherwise zero height.
        let is_community_view = matches!(
            self.nav.current_view_kind(),
            ViewKind::ChannelWatch { .. } | ViewKind::CommunityInfo { .. } | ViewKind::VoiceSession { .. }
        );
        let is_dashboard = matches!(self.nav.current_view_kind(), ViewKind::Dashboard);

        let community_rail_h = if is_community_view { self.rails.community_rail_height() } else { 0 };
        let system_rail_h = if is_dashboard { self.rails.system_rail_height() } else { 0 };

        let [header, community_rail_area, content, system_rail_area, footer_status, footer_hints] = Layout::vertical([
            Constraint::Length(1),                    // tab bar
            Constraint::Length(community_rail_h),     // community/channel signals (0 when empty)
            Constraint::Fill(1),                      // main content
            Constraint::Length(system_rail_h),         // system duress signals (0 when empty)
            Constraint::Length(1),                    // status bar
            Constraint::Length(1),                    // help hints
        ]).areas(area);

        self.nav.tab_bar_row = header.y;
        tab_bar::render::render(frame, header, &mut self.nav.tab_bar, &self.theme);

        // Community/channel notification rail (top, below tab bar)
        if community_rail_h > 0 {
            self.rails.render_community_rail(frame, community_rail_area, &self.theme);
        }

        if self.loading_spinner.is_active() {
            self.loading_spinner.render(frame, content);
        } else {
            let _ = self.nav.current_view_mut().draw(frame, content, &self.theme);
        }

        // System notification rail (bottom, above status bar, dashboard only)
        if system_rail_h > 0 {
            self.rails.render_system_rail(frame, system_rail_area, &self.theme);
        }

        // Read-only access for status bar — uses &dyn ViewQuery via current_view()
        let typing_names = self.nav.current_view().typing_names();
        let mode = if self.nav.input_mode() { status_bar::Mode::Insert }
        else if self.search_overlay.visible { status_bar::Mode::Search }
        else { status_bar::Mode::Normal };

        let panel_count = self.nav.current_view_mut().focus_ring().len();
        let breadcrumb = if panel_count > 1 && !self.nav.current_view_mut().focus_ring().is_empty() {
            format!("{} ({panel_count} panels)", self.breadcrumb())
        } else {
            self.breadcrumb()
        };
        status_bar::render(frame, footer_status, &status_bar::StatusBarState {
            mode, breadcrumb,
            typing_context: typing_indicator::format_typing_compact(&typing_names),
            node_attached: self.node_was_connected, peer_count: self.cached_peer_count,
            hints: self.keymap.hint_line(self.current_context()),
        }, &self.theme);

        let context = self.current_context();
        let help_hints = self.keymap.help_text(context);
        help_bar::render(frame, footer_hints, &help_hints, &self.theme);

        if let Some(overlay) = self.nav.overlay().cloned() {
            self.render_overlay(frame, content, &overlay);
        }
        confirm_dialog::render::render(frame, content, &self.confirm, &self.theme);
        self.search_overlay.render(frame, content, &self.theme);
        self.file_content_search.render(frame, content, &self.theme);
        if !self.notifications.is_empty() { self.notifications.render(frame, area, &self.theme); }
    }

    fn render_overlay(&self, frame: &mut Frame, area: Rect, overlay: &OverlayKind) {
        match overlay {
            OverlayKind::Help => {
                let context = if self.nav.input_mode() { KeymapContext::Input } else { KeymapContext::Default };
                let bindings = self.keymap.help_text(context);
                let mut lines = vec![
                    Line::from(Span::styled("  Keybindings:", Style::new().bold())), Line::from(""),
                ];
                for (combo, desc) in &bindings {
                    lines.push(Line::from(vec![
                        Span::styled(format!("  {combo:<16}"), self.theme.style("keyword")),
                        self.theme.span("dim", desc),
                    ]));
                }
                let theme_info = if self.theme.is_light() { "light" } else { "dark" };
                let tier = self.theme.tier();
                let color_info = format!("{tier:?}{}",
                    if tier.has_256() { " (256+)" } else { "" }
                );
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    format!("  Theme: {} ({theme_info}, {color_info})  Themes: {}",
                        self.theme.name(),
                        crate::v2::tui::theme::ThemeManager::available_themes().join(", "),
                    ),
                    self.theme.style("dim"),
                )));
                lines.push(Line::from(Span::styled(
                    "  Press any key to close.",
                    self.theme.style("dim"),
                )));
                #[allow(clippy::cast_possible_truncation)]
                let height = (lines.len() as u16 + 2).min(area.height);
                let popup = centered_rect(area, 50, height);
                frame.render_widget(Clear, popup);
                frame.render_widget(
                    Paragraph::new(lines).block(Block::bordered().title(" Help ").border_style(self.theme.focused_border())),
                    popup,
                );
            }
            OverlayKind::ConfirmAction { ref prompt, ref consequence, .. } => {
                let lines = vec![
                    Line::from(""), Line::from(format!("  {prompt}")), Line::from(""),
                    Line::from(Span::styled(format!("  {consequence}"), Style::new().dim())),
                    Line::from(""), Line::from("  [y] Confirm    [n/Esc] Cancel"),
                ];
                let popup = centered_rect(area, 50, 8);
                frame.render_widget(Clear, popup);
                frame.render_widget(
                    Paragraph::new(lines).block(
                        Block::bordered().title(" Confirm ").border_style(Style::default().fg(self.theme.color("warning")))
                    ),
                    popup,
                );
            }
            OverlayKind::Search(_) => {} // Handled by self.search_overlay.render()
        }
    }

    pub(crate) fn transition_to_selected_tab(&mut self) {
        let Some(tab_id) = self.nav.tab_bar.selected_id().map(str::to_string) else { return };
        self.nav.tab_bar.clear_unread(&tab_id);
        match tab_id.as_str() {
            "dashboard" => { self.nav.navigate(ViewKind::Dashboard, self.theme.use_unicode()); self.load_dashboard_data(); }
            "communities" => {
                if let Some(first) = self.cached_communities.first() {
                    let gov = first.governance_key.clone();
                    self.nav.navigate(ViewKind::CommunityInfo { community: gov.clone() }, self.theme.use_unicode());
                    self.load_community_info(&gov);
                } else {
                    self.notifications.push("No communities joined yet.".into(), ToastLevel::Info);
                }
            }
            "dms" => { self.nav.navigate(ViewKind::DmInbox, self.theme.use_unicode()); self.load_dm_inbox(); }
            "friends" => { self.nav.navigate(ViewKind::FriendList, self.theme.use_unicode()); self.load_friend_list(); }
            _ => {}
        }
    }

    pub(crate) fn breadcrumb(&self) -> String {
        match self.nav.current_view_kind() {
            ViewKind::Dashboard => "Dashboard".into(),
            ViewKind::IdentitySettings => "Dashboard > Identity".into(),
            ViewKind::ChannelWatch { community, channel } => format!("{} / #{channel}", self.community_name(community)),
            ViewKind::DmInbox => "DMs".into(),
            ViewKind::DmThread { peer_key } => format!("DM / {}", helpers::abbreviate_key(peer_key)),
            ViewKind::VoiceSession { community, channel } => format!("{} / #{channel} (voice)", self.community_name(community)),
            ViewKind::FriendList => "Friends".into(),
            ViewKind::Doctor => "Doctor".into(),
            ViewKind::CommunityInfo { community } => format!("{} / Info", self.community_name(community)),
            ViewKind::FilePreview { ref path, .. } => format!("File / {}", helpers::abbreviate_key(path)),
        }
    }

    /// Build search items for the overlay. Uses `&self` — read-only access
    /// to views via `ViewQuery` trait, no `&mut self` escalation.
    pub(crate) fn build_search_items(&self, mode: SearchMode) -> Vec<SearchItem> {
        match mode {
            SearchMode::QuickSwitch => {
                let mut items: Vec<SearchItem> = self.cached_communities.iter().map(|c| SearchItem {
                    label: c.name.clone(), detail: "community".into(),
                    action: Action::ShowCommunityInfo { community: c.governance_key.clone() },
                }).collect();

                // fff project-wide file search results mixed into quick switcher
                // when the user types — the search overlay's filter will narrow
                // these down. Static navigation targets are always present.
                items.extend([
                    SearchItem { label: "Dashboard".into(), detail: "view".into(), action: Action::ShowDashboard },
                    SearchItem { label: "Identity".into(), detail: "settings".into(), action: Action::ShowIdentitySettings },
                    SearchItem { label: "Friends".into(), detail: "view".into(), action: Action::ShowFriendList },
                    SearchItem { label: "DMs".into(), detail: "view".into(), action: Action::ShowDmInbox },
                    SearchItem { label: "Doctor".into(), detail: "diagnostics".into(), action: Action::ShowDoctor },
                ]);

                // If fff search is initialized, seed with recent queries and
                // frecency-ranked files so the quick switcher is immediately useful
                if let Some(ref search) = self.search {
                    // Recent search queries — lets the user re-run past searches
                    for query in search.recent_queries(5) {
                        items.push(SearchItem {
                            label: format!("recent: {query}"), detail: "history".into(),
                            action: Action::OpenSearch(SearchMode::MessageSearch),
                        });
                    }
                    for (path, score) in search.search_files("", None, 30) {
                        let detail = if score.frecency_boost > 0 {
                            "recent file".into()
                        } else {
                            "file".into()
                        };
                        items.push(SearchItem {
                            label: path.clone(),
                            detail,
                            action: Action::FileSelected { path },
                        });
                    }
                }

                items
            }
            SearchMode::MessageSearch => {
                // Read-only access via ViewQuery — no &mut self needed
                self.nav.current_view().message_search_index()
                    .into_iter()
                    .map(|(message_id, author, body)| {
                        let preview = if body.chars().count() > 80 {
                            let end = body.char_indices().nth(77).map_or(body.len(), |(i, _)| i);
                            format!("{}...", &body[..end])
                        } else {
                            body
                        };
                        SearchItem {
                            label: preview,
                            detail: author,
                            action: Action::ScrollToMessage { message_id },
                        }
                    })
                    .collect()
            }
            SearchMode::CommandPalette => {
                let mut items = vec![
                    // Navigation
                    SearchItem { label: "Dashboard".into(), detail: "navigate".into(), action: Action::ShowDashboard },
                    SearchItem { label: "Friends".into(), detail: "navigate".into(), action: Action::ShowFriendList },
                    SearchItem { label: "DMs".into(), detail: "navigate".into(), action: Action::ShowDmInbox },
                    SearchItem { label: "Doctor".into(), detail: "diagnostics".into(), action: Action::ShowDoctor },
                    SearchItem { label: "Identity Settings".into(), detail: "navigate".into(), action: Action::ShowIdentitySettings },
                    // UI controls
                    SearchItem { label: "Toggle Help".into(), detail: "ui".into(), action: Action::ToggleHelp },
                    SearchItem { label: "Toggle Sidebar".into(), detail: "ui".into(), action: Action::ToggleSidebar },
                    SearchItem { label: "Refresh".into(), detail: "ui".into(), action: Action::Refresh },
                    SearchItem { label: "Quit".into(), detail: "app".into(), action: Action::Quit },
                    // Presence
                    SearchItem { label: "Set Online".into(), detail: "presence".into(), action: Action::SetPresence { status: "online".into(), message: None } },
                    SearchItem { label: "Set Away".into(), detail: "presence".into(), action: Action::SetPresence { status: "away".into(), message: None } },
                    SearchItem { label: "Set Busy".into(), detail: "presence".into(), action: Action::SetPresence { status: "busy".into(), message: None } },
                    SearchItem { label: "Set Invisible".into(), detail: "presence".into(), action: Action::SetPresence { status: "invisible".into(), message: None } },
                ];
                // Community-specific commands
                for c in &self.cached_communities {
                    items.push(SearchItem {
                        label: format!("Open {}", c.name), detail: "community".into(),
                        action: Action::ShowCommunityInfo { community: c.governance_key.clone() },
                    });
                    items.push(SearchItem {
                        label: format!("Leave {}", c.name), detail: "community".into(),
                        action: Action::LeaveCommunity { community: c.governance_key.clone() },
                    });
                }
                // Mixed file+directory search for the command palette
                if let Some(ref search) = self.search {
                    for (path, _score) in search.search_mixed("", None, 20) {
                        items.push(SearchItem {
                            label: path.clone(), detail: "file/dir".into(),
                            action: Action::FileSelected { path },
                        });
                    }
                }
                items
            }
        }
    }
}

pub(crate) fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect { x, y, width, height }
}
