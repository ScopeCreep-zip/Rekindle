//! Dashboard view — the default view on bare `rekindle` invocation.
//!
//! Four panels: Identity, Node Status, Communities, Friends summary.
//! Provides a quick overview of the user's rekindle state at a glance.
//!
//! Layout:
//! ```text
//! ┌─ Identity ──────────────────────────────────────────────────┐
//! │ ed25519:7f3a…  [UNLOCKED]    Display: alice                 │
//! ├─ Node ──────────────────────────────────────────────────────┤
//! │ ● [ONLINE]  attached  PublicInternet  12 peers  route OK    │
//! │ Uptime: 4h 23m                                              │
//! ├─ Communities (3 joined) ────────────────────────────────────┤
//! │   dev-team     12 members   3 channels                      │
//! │   gaming       48 members   8 channels                      │
//! ├─ Friends (8 online, 3 away, 12 offline) ────────────────────┤
//! │   ● alice   ● bob   ◐ carol   ○ dave  ...                  │
//! └─────────────────────────────────────────────────────────────┘
//! ```

use anyhow::Result;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

use crate::helpers;
use crate::tui::action::{Action, CommandResult};
use rekindle_types::display::{CommunityOverview, FriendDisplay};
use crate::tui::focus::{FocusId, FocusRing};
use crate::tui::theme::ThemeManager;
use super::View;

/// Dashboard view state.
pub struct DashboardView {
    focus: FocusRing,
    /// Panel rects from the last draw — used for click-to-focus hit testing.
    panel_rects: [Rect; 4],
    /// Identity display data — set by the App before first render.
    identity_public_key: String,
    identity_display_name: String,
    /// Cached node status data.
    node_attached: bool,
    node_public_internet: bool,
    node_uptime_secs: u64,
    node_peer_count: usize,
    node_route_allocated: bool,
    /// Cached community list.
    communities: Vec<CommunityOverview>,
    /// Cached friend list summary.
    friends: Vec<FriendDisplay>,
    /// Whether initial data has been loaded from the transport.
    /// When false, draw() shows a loading indicator instead of empty-state messages.
    loaded: bool,
    /// Unicode glyph support.
    use_unicode: bool,
}

impl DashboardView {
    /// Create a new dashboard view.
    pub fn new(use_unicode: bool) -> Self {
        Self {
            focus: FocusRing::new(vec![
                FocusId::CommunityInfoPanel,  // Identity
                FocusId::DoctorList,           // Node status
                FocusId::ChannelTree,          // Communities
                FocusId::FriendList,           // Friends
            ]),
            panel_rects: [Rect::default(); 4],
            identity_public_key: String::new(),
            identity_display_name: String::new(),
            node_attached: false,
            node_public_internet: false,
            node_uptime_secs: 0,
            node_peer_count: 0,
            node_route_allocated: false,
            communities: Vec::new(),
            friends: Vec::new(),
            loaded: false,
            use_unicode,
        }
    }

    /// Set identity data from the session. Called by App before first render.
    pub fn set_identity(&mut self, public_key: &str, display_name: &str) {
        self.identity_public_key = public_key.to_string();
        self.identity_display_name = display_name.to_string();
    }

    /// Render the identity panel.
    fn render_identity(&self, frame: &mut Frame, area: Rect, theme: &ThemeManager) {
        let key_short = helpers::abbreviate_key(&self.identity_public_key);
        let lines = vec![
            Line::from(vec![
                Span::styled("  Public key:    ", Style::new().dim()),
                Span::raw(key_short),
            ]),
            Line::from(vec![
                Span::styled("  Display name:  ", Style::new().dim()),
                Span::styled(self.identity_display_name.clone(), Style::new().bold()),
            ]),
        ];

        let border = if self.focus.is_focused(FocusId::CommunityInfoPanel) {
            theme.focused_border()
        } else {
            theme.unfocused_border()
        };
        let block = Block::bordered()
            .title(" Identity ")
            .border_style(border);
        frame.render_widget(Paragraph::new(lines).block(block), area);
    }

    /// Render the node status panel.
    fn render_node(&self, frame: &mut Frame, area: Rect, theme: &ThemeManager) {
        let attach_glyph = if self.use_unicode {
            if self.node_attached { "●" } else { "○" }
        } else if self.node_attached { "o" } else { "." };

        let attach_label = if self.node_attached { "[ONLINE]" } else { "[OFFLINE]" };
        let internet = if self.node_public_internet { "[OK] ready" } else { "[--] not ready" };
        let route = if self.node_route_allocated { "[OK] allocated" } else { "[--] none" };
        let uptime = helpers::format_uptime(self.node_uptime_secs);

        let lines = vec![
            Line::from(vec![
                Span::styled("  Status:     ", Style::new().dim()),
                Span::raw(format!("{attach_glyph} {attach_label}")),
                Span::styled(format!("  {internet}"), Style::new().dim()),
            ]),
            Line::from(vec![
                Span::styled("  Peers:      ", Style::new().dim()),
                Span::raw(self.node_peer_count.to_string()),
                Span::styled(format!("  Route: {route}"), Style::new().dim()),
            ]),
            Line::from(vec![
                Span::styled("  Uptime:     ", Style::new().dim()),
                Span::raw(uptime),
            ]),
        ];

        let border = if self.focus.is_focused(FocusId::DoctorList) {
            theme.focused_border()
        } else {
            theme.unfocused_border()
        };
        let block = Block::bordered()
            .title(" Node ")
            .border_style(border);
        frame.render_widget(Paragraph::new(lines).block(block), area);
    }

    /// Render the communities panel.
    fn render_communities(&self, frame: &mut Frame, area: Rect, theme: &ThemeManager) {
        let title = format!(" Communities ({}) ", self.communities.len());
        let border = if self.focus.is_focused(FocusId::ChannelTree) {
            theme.focused_border()
        } else {
            theme.unfocused_border()
        };
        let block = Block::bordered()
            .title(title)
            .border_style(border);

        if self.communities.is_empty() {
            let para = Paragraph::new("  No communities joined.\n  Join one: rekindle community join --invite <code>")
                .style(Style::new().dim())
                .block(block);
            frame.render_widget(para, area);
            return;
        }

        let lines: Vec<Line<'_>> = self
            .communities
            .iter()
            .map(|c| {
                Line::from(vec![
                    Span::styled("  ", Style::new()),
                    Span::styled(&c.name, Style::new().bold()),
                    Span::styled(
                        format!("  {} members  {} channels", c.member_count, c.channel_count),
                        Style::new().dim(),
                    ),
                ])
            })
            .collect();

        frame.render_widget(Paragraph::new(lines).block(block), area);
    }

    /// Render the friends summary panel.
    fn render_friends(&self, frame: &mut Frame, area: Rect, theme: &ThemeManager) {
        let online = self.friends.iter().filter(|f| f.status == "online").count();
        let away = self.friends.iter().filter(|f| f.status == "away").count();
        let offline = self.friends.iter().filter(|f| f.status == "offline").count();
        let total = self.friends.len();

        let title = format!(" Friends ({total}) ");
        let border = if self.focus.is_focused(FocusId::FriendList) {
            theme.focused_border()
        } else {
            theme.unfocused_border()
        };
        let block = Block::bordered()
            .title(title)
            .border_style(border);

        if self.friends.is_empty() {
            let para = Paragraph::new("  No friends yet.\n  Add one: rekindle friend add --target <key>")
                .style(Style::new().dim())
                .block(block);
            frame.render_widget(para, area);
            return;
        }

        let summary = Line::from(vec![
            Span::styled("  ", Style::new()),
            Span::styled(format!("{online} online"), Style::new().bold()),
            Span::styled(format!("  {away} away  {offline} offline"), Style::new().dim()),
        ]);

        // Show first few friends inline
        let preview_count = 6.min(self.friends.len());
        let mut lines = vec![summary];
        let mut previews: Vec<Span<'_>> = vec![Span::raw("  ")];
        for f in self.friends.iter().take(preview_count) {
            let glyph = match f.status.as_str() {
                "online" => if self.use_unicode { "● " } else { "o " },
                "away" => if self.use_unicode { "◐ " } else { "~ " },
                _ => if self.use_unicode { "○ " } else { ". " },
            };
            previews.push(Span::raw(format!("{glyph}{}", helpers::sanitize_for_display(&f.display_name))));
            previews.push(Span::raw("  "));
        }
        if self.friends.len() > preview_count {
            previews.push(Span::styled(
                format!("...+{} more", self.friends.len() - preview_count),
                Style::new().dim(),
            ));
        }
        lines.push(Line::from(previews));

        frame.render_widget(Paragraph::new(lines).block(block), area);
    }
}

impl View for DashboardView {
    fn draw(&mut self, frame: &mut Frame, area: Rect, theme: &ThemeManager) -> Result<()> {
        if !self.loaded {
            let block = Block::bordered()
                .title(" Dashboard ")
                .border_style(theme.focused_border());
            let loading = Paragraph::new("  Loading dashboard data...")
                .style(Style::new().dim())
                .block(block);
            frame.render_widget(loading, area);
            return Ok(());
        }

        // 2x2 grid layout:
        // ┌─ Identity ──────┬─ Node ────────────┐
        // ├─ Communities ───┼─ Friends ──────────┤
        // └─────────────────┴────────────────────┘
        let [top_row, bottom_row] = Layout::vertical([
            Constraint::Length(5),
            Constraint::Fill(1),
        ])
        .areas(area);

        let [identity_area, node_area] = Layout::horizontal([
            Constraint::Percentage(50),
            Constraint::Percentage(50),
        ])
        .areas(top_row);

        let [communities_area, friends_area] = Layout::horizontal([
            Constraint::Percentage(50),
            Constraint::Percentage(50),
        ])
        .areas(bottom_row);

        // Store rects for click-to-focus hit testing
        self.panel_rects = [identity_area, node_area, communities_area, friends_area];

        self.render_identity(frame, identity_area, theme);
        self.render_node(frame, node_area, theme);
        self.render_communities(frame, communities_area, theme);
        self.render_friends(frame, friends_area, theme);

        Ok(())
    }

    fn update(&mut self, action: Action) -> Result<Option<Action>> {
        match action {
            // 2x2 grid navigation:
            // [CommunityInfoPanel, DoctorList ]  = [Identity, Node      ]
            // [ChannelTree,        FriendList ]  = [Communities, Friends]
            //
            // j/Down = move down in the grid
            Action::ScrollDown(_) => {
                match self.focus.current() {
                    FocusId::CommunityInfoPanel => self.focus.set(FocusId::ChannelTree),
                    FocusId::DoctorList => self.focus.set(FocusId::FriendList),
                    _ => {} // already on bottom row
                }
            }
            // k/Up = move up in the grid
            Action::ScrollUp(_) => {
                match self.focus.current() {
                    FocusId::ChannelTree => self.focus.set(FocusId::CommunityInfoPanel),
                    FocusId::FriendList => self.focus.set(FocusId::DoctorList),
                    _ => {} // already on top row
                }
            }
            // Tab / FocusNext = cycle through all 4 panels linearly
            Action::FocusNext => self.focus.next(),
            Action::FocusPrev => self.focus.prev(),
            // h/Back = move left in the grid (on dashboard, Back doesn't navigate away)
            Action::Back => {
                match self.focus.current() {
                    FocusId::DoctorList => self.focus.set(FocusId::CommunityInfoPanel),
                    FocusId::FriendList => self.focus.set(FocusId::ChannelTree),
                    _ => {} // already on left column
                }
            }
            Action::ScrollToTop => self.focus.set(FocusId::CommunityInfoPanel),
            Action::ScrollToBottom => self.focus.set(FocusId::FriendList),
            Action::Select => {
                // Enter/l on a focused panel navigates into that view.
                return Ok(match self.focus.current() {
                    FocusId::CommunityInfoPanel => {
                        // Identity → show identity details (status command)
                        Some(Action::ShowDoctor) // identity detail view is M3 scope
                    }
                    FocusId::DoctorList => Some(Action::ShowDoctor),
                    FocusId::ChannelTree => {
                        self.communities.first().map(|c| {
                            Action::ShowCommunityInfo { community: c.governance_key.clone() }
                        })
                    }
                    FocusId::FriendList => Some(Action::ShowFriendList),
                    _ => None,
                });
            }
            _ => {}
        }
        Ok(None)
    }

    fn on_command_result(&mut self, result: CommandResult) -> Result<()> {
        match result {
            CommandResult::StatusLoaded { snapshot } => {
                self.node_attached = snapshot.is_attached;
                self.node_public_internet = snapshot.public_internet_ready;
                self.node_uptime_secs = snapshot.uptime_secs;
                self.node_peer_count = snapshot.peer_count;
                self.node_route_allocated = snapshot.route_allocated;
                self.loaded = true;
            }
            CommandResult::CommunityListLoaded { communities } => {
                self.communities = communities;
            }
            CommandResult::FriendListLoaded { friends } => {
                self.friends = friends;
            }
            CommandResult::IdentityLoaded { public_key, display_name } => {
                self.identity_public_key = public_key;
                self.identity_display_name = display_name;
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_focused_key(&mut self, key: crossterm::event::KeyEvent) -> Option<Action> {
        use crossterm::event::KeyCode;
        match key.code {
            // Dashboard-only quick navigation shortcuts
            KeyCode::Char('d') => Some(Action::ShowDmInbox),
            KeyCode::Char('f') => Some(Action::ShowFriendList),
            KeyCode::Char('c') => {
                self.communities.first().map(|c| {
                    Action::ShowCommunityInfo { community: c.governance_key.clone() }
                })
            }
            // h/l for horizontal grid navigation — intercepted here as raw keys
            // because the global keymap maps h→Back and l→Select which get
            // consumed by App::process_action before reaching the view's update().
            KeyCode::Char('h') => {
                match self.focus.current() {
                    FocusId::DoctorList => self.focus.set(FocusId::CommunityInfoPanel),
                    FocusId::FriendList => self.focus.set(FocusId::ChannelTree),
                    _ => {} // already on left column
                }
                None // consumed — don't propagate as Back
            }
            KeyCode::Char('l') => {
                match self.focus.current() {
                    FocusId::CommunityInfoPanel => self.focus.set(FocusId::DoctorList),
                    FocusId::ChannelTree => self.focus.set(FocusId::FriendList),
                    _ => {} // already on right column
                }
                None // consumed — don't propagate as Select
            }
            // Enter explicitly enters the focused panel (distinct from l which moves right)
            KeyCode::Enter => {
                match self.focus.current() {
                    FocusId::CommunityInfoPanel => Some(Action::ShowIdentitySettings),
                    FocusId::DoctorList => Some(Action::ShowDoctor),
                    FocusId::ChannelTree => {
                        if let Some(c) = self.communities.first() {
                            Some(Action::ShowCommunityInfo { community: c.governance_key.clone() })
                        } else {
                            // No communities yet — still navigate to show the empty state
                            Some(Action::ShowCommunityInfo { community: String::new() })
                        }
                    }
                    FocusId::FriendList => Some(Action::ShowFriendList),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn handle_click(&mut self, column: u16, row: u16) -> Option<Action> {
        let focus_ids = [
            FocusId::CommunityInfoPanel,
            FocusId::DoctorList,
            FocusId::ChannelTree,
            FocusId::FriendList,
        ];
        for (rect, &id) in self.panel_rects.iter().zip(&focus_ids) {
            if column >= rect.x
                && column < rect.x + rect.width
                && row >= rect.y
                && row < rect.y + rect.height
            {
                self.focus.set(id);
                // Navigate into the clicked pane
                return match id {
                    FocusId::CommunityInfoPanel => Some(Action::ShowIdentitySettings),
                    FocusId::DoctorList => Some(Action::ShowDoctor),
                    FocusId::ChannelTree => {
                        self.communities.first().map(|c| {
                            Action::ShowCommunityInfo { community: c.governance_key.clone() }
                        })
                    }
                    FocusId::FriendList => Some(Action::ShowFriendList),
                    _ => None,
                };
            }
        }
        None
    }

    fn focus_ring(&mut self) -> &mut FocusRing {
        &mut self.focus
    }
}
