//! Identity & settings view — profile details, DHT records, security, actions.

use anyhow::Result;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use rekindle_types::display::StatusSnapshot;

use crate::v2::helpers;
use crate::v2::tui::action::{Action, CommandResult};
use crate::v2::tui::focus::{FocusId, FocusRing};
use crate::v2::tui::theme::ThemeManager;
use super::View;

pub struct IdentitySettingsView {
    focus: FocusRing,
    list_state: ListState,
    public_key: String,
    display_name: String,
    route_allocated: bool,
    route_age_secs: Option<u64>,
    attachment: String,
    active_watches: usize,
    community_count: usize,
    friend_count: usize,
    signing_key_loaded: bool,
    expanded: [bool; 5],
    loaded: bool,
    use_unicode: bool,
    // DHT record keys — populated from IdentityShow response (SessionIdentity)
    profile_dht_key: String,
    mailbox_dht_key: String,
    friend_list_dht_key: String,
    friend_inbox_key: String,
}

impl IdentitySettingsView {
    pub fn new(use_unicode: bool) -> Self {
        Self {
            focus: FocusRing::new(vec![FocusId::IdentitySettings]),
            list_state: ListState::default().with_selected(Some(0)),
            public_key: String::new(), display_name: String::new(),
            route_allocated: false, route_age_secs: None, attachment: "unknown".into(),
            active_watches: 0, community_count: 0, friend_count: 0,
            signing_key_loaded: false, expanded: [false; 5], loaded: false, use_unicode,
            profile_dht_key: String::new(), mailbox_dht_key: String::new(),
            friend_list_dht_key: String::new(), friend_inbox_key: String::new(),
        }
    }

    fn load_from_snapshot(&mut self, snapshot: &StatusSnapshot) {
        self.public_key = snapshot.identity_public_key.clone().unwrap_or_default();
        self.display_name = snapshot.identity_display_name.clone().unwrap_or_default();
        self.route_allocated = snapshot.route_allocated;
        self.route_age_secs = snapshot.route_age_secs;
        self.attachment.clone_from(&snapshot.attachment);
        self.active_watches = snapshot.active_watches;
        self.community_count = snapshot.community_count;
        self.friend_count = snapshot.friend_count;
        self.signing_key_loaded = snapshot.checks.iter().any(|c| {
            c.id == "crypto.signing_key" && c.status == rekindle_types::display::CheckStatus::Pass
        });
        self.loaded = true;
    }

    fn build_items(&self) -> Vec<ListItem<'static>> {
        let mut items = Vec::new();
        let arrow_right = if self.use_unicode { "▸" } else { ">" };
        let arrow_down = if self.use_unicode { "▾" } else { "v" };

        items.push(kv_item("  Public Key", &helpers::abbreviate_key(&self.public_key)));
        items.push(kv_item("  Display Name", &self.display_name));
        items.push(kv_item("  Attachment", &self.attachment));
        let route_str = if self.route_allocated { format!("allocated ({}s)", self.route_age_secs.unwrap_or(0)) }
        else { "not allocated".into() };
        items.push(kv_item("  Route", &route_str));
        items.push(kv_item("  Watches", &self.active_watches.to_string()));
        items.push(ListItem::new(Line::raw("")));

        let sections = ["DHT Records", "Security", "Network", "Theme", "Actions"];
        for (i, name) in sections.iter().enumerate() {
            let arrow = if self.expanded[i] { arrow_down } else { arrow_right };
            items.push(ListItem::new(Line::from(vec![
                Span::styled(format!("  {arrow} "), Style::new().bold()),
                Span::styled(*name, Style::new().bold()),
            ])));
            if self.expanded[i] {
                match i {
                    0 => { // DHT Records
                        let profile = if self.profile_dht_key.is_empty() { "not loaded".into() } else { helpers::abbreviate_key(&self.profile_dht_key) };
                        let mailbox = if self.mailbox_dht_key.is_empty() { "not loaded".into() } else { helpers::abbreviate_key(&self.mailbox_dht_key) };
                        let fl = if self.friend_list_dht_key.is_empty() { "not loaded".into() } else { helpers::abbreviate_key(&self.friend_list_dht_key) };
                        let fi = if self.friend_inbox_key.is_empty() { "not loaded".into() } else { helpers::abbreviate_key(&self.friend_inbox_key) };
                        items.push(kv_item("    Profile", &profile));
                        items.push(kv_item("    Mailbox", &mailbox));
                        items.push(kv_item("    Friend List", &fl));
                        items.push(kv_item("    Friend Inbox", &fi));
                    }
                    1 => { // Security
                        let key_status = if self.signing_key_loaded { "loaded (unlocked)" } else { "not loaded (locked)" };
                        items.push(kv_item("    Signing Key", key_status));
                        items.push(kv_item("    Keyring", "OS keyring + disk fallback"));
                    }
                    2 => { // Network
                        items.push(kv_item("    Communities", &self.community_count.to_string()));
                        items.push(kv_item("    Friends", &self.friend_count.to_string()));
                        items.push(kv_item("    Active Watches", &self.active_watches.to_string()));
                    }
                    3 => { // Theme
                        use crate::v2::tui::theme::ThemeManager;
                        items.push(kv_item("    Available", &ThemeManager::available_themes().join(", ")));
                    }
                    4 => { // Actions
                        items.push(ListItem::new(Line::from(vec![
                            Span::styled("    [e] ", Style::new().dim()), Span::raw("Export identity bundle"),
                        ])));
                        items.push(ListItem::new(Line::from(vec![
                            Span::styled("    [r] ", Style::new().dim()), Span::raw("Rotate identity keys"),
                            Span::styled("  (dangerous)", Style::new().dim()),
                        ])));
                        items.push(ListItem::new(Line::from(vec![
                            Span::styled("    [D] ", Style::new().dim()), Span::raw("Destroy identity"),
                            Span::styled("  (irreversible)", Style::new().dim()),
                        ])));
                    }
                    _ => {}
                }
            }
        }
        items
    }

    fn selected_section(&self) -> Option<usize> {
        let sel = self.list_state.selected()?;
        let items = self.build_items();
        let item = items.get(sel)?;
        let text = format!("{item:?}");
        for (i, name) in ["DHT Records", "Security", "Network", "Actions"].iter().enumerate() {
            if text.contains(name) { return Some(i); }
        }
        None
    }
}

fn kv_item(key: &str, value: &str) -> ListItem<'static> {
    ListItem::new(Line::from(vec![
        Span::styled(format!("{key:<18}"), Style::new().dim()),
        Span::raw(value.to_string()),
    ]))
}

impl super::ViewQuery for IdentitySettingsView {}

impl View for IdentitySettingsView {
    fn draw(&mut self, frame: &mut Frame, area: Rect, theme: &ThemeManager) -> Result<()> {
        let [list_area, help_area] = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(area);

        let title = format!(" Identity — {} ", if self.display_name.is_empty() { "loading..." } else { &self.display_name });
        let block = Block::bordered().title(title).border_style(theme.focused_border());

        if self.loaded {
            let items = self.build_items();
            frame.render_stateful_widget(
                List::new(items).block(block).highlight_style(Style::new().reversed()),
                list_area, &mut self.list_state,
            );
        } else {
            frame.render_widget(Paragraph::new("  Loading identity...").style(theme.style("dim")).block(block), list_area);
        }

        frame.render_widget(Paragraph::new(Line::from(
            Span::styled("  [y] yank key  [Enter] expand  [q] back  [?] help", Style::new().dim()),
        )), help_area);
        Ok(())
    }

    fn update(&mut self, action: Action) -> Result<Option<Action>> {
        match action {
            Action::ScrollDown(_) => {
                let max = self.build_items().len().saturating_sub(1);
                let i = self.list_state.selected().unwrap_or(0);
                self.list_state.select(Some((i + 1).min(max)));
            }
            Action::ScrollUp(_) => {
                let i = self.list_state.selected().unwrap_or(0);
                self.list_state.select(Some(i.saturating_sub(1)));
            }
            Action::Select => {
                if let Some(section_idx) = self.selected_section() {
                    self.expanded[section_idx] = !self.expanded[section_idx];
                }
            }
            _ => {}
        }
        Ok(None)
    }

    fn handle_focused_key(&mut self, key: crossterm::event::KeyEvent) -> Option<Action> {
        use crossterm::event::KeyCode;
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                let max = self.build_items().len().saturating_sub(1);
                let i = self.list_state.selected().unwrap_or(0);
                self.list_state.select(Some((i + 1).min(max)));
                None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                let i = self.list_state.selected().unwrap_or(0);
                self.list_state.select(Some(i.saturating_sub(1)));
                None
            }
            KeyCode::Enter => {
                if let Some(section_idx) = self.selected_section() {
                    self.expanded[section_idx] = !self.expanded[section_idx];
                }
                None
            }
            KeyCode::Char('y') => {
                if self.public_key.is_empty() { None }
                else { Some(Action::YankToClipboard { text: self.public_key.clone() }) }
            }
            _ => None,
        }
    }

    fn on_command_result(&mut self, result: CommandResult) -> Result<()> {
        match result {
            CommandResult::StatusLoaded { ref snapshot } => { self.load_from_snapshot(snapshot); }
            CommandResult::IdentityLoaded {
                ref public_key, ref display_name,
                ref profile_dht_key, ref mailbox_dht_key,
                ref friend_list_dht_key, ref friend_inbox_key,
            } => {
                if !public_key.is_empty() {
                    self.public_key.clone_from(public_key);
                    self.display_name.clone_from(display_name);
                    self.profile_dht_key.clone_from(profile_dht_key);
                    self.mailbox_dht_key.clone_from(mailbox_dht_key);
                    self.friend_list_dht_key.clone_from(friend_list_dht_key);
                    self.friend_inbox_key.clone_from(friend_inbox_key);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn on_subscription_event(&mut self, event: &rekindle_types::subscription_events::SubscriptionEvent) -> Result<()> {
        if let rekindle_types::subscription_events::SubscriptionEvent::Network(
            rekindle_types::subscription_events::NetworkEvent::AttachmentChanged { is_attached, .. }
        ) = event {
            self.attachment = if *is_attached { "attached".into() } else { "detached".into() };
        }
        Ok(())
    }

    fn focus_ring(&mut self) -> &mut FocusRing { &mut self.focus }
}
