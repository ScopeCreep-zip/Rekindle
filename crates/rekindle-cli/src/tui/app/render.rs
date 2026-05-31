//! Rendering, overlays, breadcrumb, search items, and tab transitions.

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
use super::super::keybinds::KeymapContext;
use super::App;
use crate::views::ViewKind;

impl App {
    pub(crate) fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();

        if area.width < 40 || area.height < 10 {
            let msg = Paragraph::new("Terminal too small.\nMinimum: 40x10")
                .alignment(ratatui::layout::Alignment::Center)
                .style(Style::new().bold());
            frame.render_widget(msg, centered_rect(area, 30, 3));
            return;
        }

        let [header, content, footer_status, footer_hints] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(area);

        self.nav.tab_bar_row = header.y;
        tab_bar::render(frame, header, &mut self.nav.tab_bar, &self.theme);

        if self.loading_spinner.is_active() {
            self.loading_spinner.render(frame, content);
        } else if let Err(e) = self
            .nav
            .current_view_mut()
            .draw(frame, content, &self.theme)
        {
            tracing::error!(error = %e, "view draw failed");
        }

        let mode = if self.nav.input_mode() {
            status_bar::Mode::Insert
        } else if self.search.visible {
            status_bar::Mode::Search
        } else {
            status_bar::Mode::Normal
        };

        let hints_line = self.keymap.hint_line(self.current_context());
        status_bar::render(
            frame,
            footer_status,
            &status_bar::StatusBarState {
                mode,
                breadcrumb: self.breadcrumb(),
                node_attached: self.node_was_connected,
                peer_count: 0,
                hints: hints_line,
            },
            &self.theme,
        );

        let context = self.current_context();
        let help_hints = self.keymap.help_text(context);
        help_bar::render(frame, footer_hints, &help_hints, &self.theme);

        if let Some(overlay) = self.nav.overlay().cloned() {
            self.render_overlay(frame, content, &overlay);
        }
        confirm_dialog::render(frame, content, &self.confirm, &self.theme);
        self.search.render(frame, content, &self.theme);
        if !self.notifications.is_empty() {
            self.notifications.render(frame, area, &self.theme);
        }
    }

    fn render_overlay(&self, frame: &mut Frame, area: Rect, overlay: &OverlayKind) {
        match overlay {
            OverlayKind::Help => {
                let context = if self.nav.input_mode() {
                    KeymapContext::Input
                } else {
                    KeymapContext::Default
                };
                let bindings = self.keymap.help_text(context);
                let keyword_style = self.theme.style("keyword");
                let dimmed_style = self.theme.style("dimmed");
                let theme_info = if self.theme.is_light() {
                    "light"
                } else {
                    "dark"
                };
                let mut lines = vec![
                    Line::from(Span::styled("  Keybindings:", Style::new().bold())),
                    Line::from(""),
                ];
                for (combo, description) in &bindings {
                    lines.push(Line::from(vec![
                        Span::styled(format!("  {combo:<16}"), keyword_style),
                        self.theme.span("muted", description),
                    ]));
                }
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    format!(
                        "  Theme: {} ({theme_info})  Press any key to close.",
                        self.theme.name()
                    ),
                    dimmed_style,
                )));
                #[allow(clippy::cast_possible_truncation)]
                let height = (lines.len() as u16 + 2).min(area.height);
                let popup = centered_rect(area, 50, height);
                frame.render_widget(Clear, popup);
                let block = Block::bordered()
                    .title(" Help ")
                    .border_style(self.theme.focused_border());
                frame.render_widget(Paragraph::new(lines).block(block), popup);
            }
            OverlayKind::ConfirmAction {
                ref prompt,
                ref consequence,
                ..
            } => {
                let lines = vec![
                    Line::from(""),
                    Line::from(format!("  {prompt}")),
                    Line::from(""),
                    Line::from(Span::styled(format!("  {consequence}"), Style::new().dim())),
                    Line::from(""),
                    Line::from("  [y] Confirm    [n/Esc] Cancel"),
                ];
                let popup = centered_rect(area, 50, 8);
                frame.render_widget(Clear, popup);
                let block = Block::bordered()
                    .title(" Confirm ")
                    .border_style(Style::default().fg(self.theme.color("warning")));
                frame.render_widget(Paragraph::new(lines).block(block), popup);
            }
            OverlayKind::Search(_) => {} // Handled by self.search.render()
        }
    }

    pub(crate) fn transition_to_selected_tab(&mut self) {
        let Some(tab_id) = self.nav.tab_bar.selected_id().map(str::to_string) else {
            return;
        };
        self.nav.tab_bar.clear_unread(&tab_id);
        match tab_id.as_str() {
            "dashboard" => {
                self.nav
                    .navigate(ViewKind::Dashboard, self.theme.use_unicode());
                self.load_dashboard_data();
            }
            "communities" => {
                if let Some(first) = self.cached_communities.first() {
                    let gov = first.governance_key.clone();
                    self.nav.navigate(
                        ViewKind::CommunityInfo {
                            community: gov.clone(),
                        },
                        self.theme.use_unicode(),
                    );
                    self.load_community_info(&gov);
                } else {
                    self.notifications.push(
                        "No communities joined yet.\n  rekindle community join --invite <code>"
                            .into(),
                        ToastLevel::Info,
                    );
                }
            }
            "dms" => {
                self.nav
                    .navigate(ViewKind::DmInbox, self.theme.use_unicode());
                self.load_dm_inbox();
            }
            "friends" => {
                self.nav
                    .navigate(ViewKind::FriendList, self.theme.use_unicode());
                self.load_friend_list();
            }
            _ => {}
        }
    }

    pub(crate) fn breadcrumb(&self) -> String {
        match self.nav.current_view() {
            ViewKind::Dashboard => "Dashboard".into(),
            ViewKind::IdentitySettings => "Dashboard > Identity".into(),
            ViewKind::ChannelWatch { community, channel } => {
                format!("{} / #{channel}", self.community_name(community))
            }
            ViewKind::DmInbox => "DMs".into(),
            ViewKind::DmThread { peer_key } => {
                format!("DM / {}", crate::helpers::abbreviate_key(peer_key))
            }
            ViewKind::VoiceSession { community, channel } => {
                format!("{} / #{channel} (voice)", self.community_name(community))
            }
            ViewKind::FriendList => "Friends".into(),
            ViewKind::Doctor => "Doctor".into(),
            ViewKind::CommunityInfo { community } => {
                format!("{} / Info", self.community_name(community))
            }
        }
    }

    pub(crate) fn build_search_items(&self, mode: SearchMode) -> Vec<SearchItem> {
        match mode {
            SearchMode::QuickSwitch => {
                let mut items: Vec<SearchItem> = self
                    .cached_communities
                    .iter()
                    .map(|c| SearchItem {
                        label: c.name.clone(),
                        detail: "community".into(),
                        action: Action::ShowCommunityInfo {
                            community: c.governance_key.clone(),
                        },
                    })
                    .collect();
                items.push(SearchItem {
                    label: "Dashboard".into(),
                    detail: "view".into(),
                    action: Action::ShowDashboard,
                });
                items.push(SearchItem {
                    label: "Identity".into(),
                    detail: "settings".into(),
                    action: Action::ShowIdentitySettings,
                });
                items.push(SearchItem {
                    label: "Friends".into(),
                    detail: "view".into(),
                    action: Action::ShowFriendList,
                });
                items.push(SearchItem {
                    label: "DMs".into(),
                    detail: "view".into(),
                    action: Action::ShowDmInbox,
                });
                items.push(SearchItem {
                    label: "Doctor".into(),
                    detail: "diagnostics".into(),
                    action: Action::ShowDoctor,
                });
                items
            }
            SearchMode::MessageSearch => Vec::new(),
            SearchMode::CommandPalette => vec![
                SearchItem {
                    label: "Quit".into(),
                    detail: String::new(),
                    action: Action::Quit,
                },
                SearchItem {
                    label: "Toggle Help".into(),
                    detail: String::new(),
                    action: Action::ToggleHelp,
                },
                SearchItem {
                    label: "Refresh".into(),
                    detail: String::new(),
                    action: Action::Refresh,
                },
                SearchItem {
                    label: "Toggle Sidebar".into(),
                    detail: String::new(),
                    action: Action::ToggleSidebar,
                },
                SearchItem {
                    label: "Dashboard".into(),
                    detail: String::new(),
                    action: Action::ShowDashboard,
                },
                SearchItem {
                    label: "Friends".into(),
                    detail: String::new(),
                    action: Action::ShowFriendList,
                },
                SearchItem {
                    label: "DMs".into(),
                    detail: String::new(),
                    action: Action::ShowDmInbox,
                },
                SearchItem {
                    label: "Doctor".into(),
                    detail: String::new(),
                    action: Action::ShowDoctor,
                },
            ],
        }
    }
}

/// Center a rect within a larger area.
pub(crate) fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect {
        x,
        y,
        width,
        height,
    }
}
