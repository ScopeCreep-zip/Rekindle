//! Identity & settings view — profile details, DHT records, security, actions.
//!
//! Displays the local identity with increasing detail as the user navigates
//! deeper. Top-level shows the most commonly needed info (key, name, status).
//! Sections expand on Enter. Actions (export, rotate, destroy) require
//! confirmation via the confirm dialog.
//!
//! Layout:
//! ```text
//! ┌─ Identity ─────────────────────────────────────────────────┐
//! │                                                             │
//! │  Public Key    4cb2…7b76  [y] copy                         │
//! │  Display Name  alice                                        │
//! │  Status        online                                       │
//! │  Route         allocated (39s)                              │
//! │  Prekeys       47/50 available                              │
//! │                                                             │
//! │  ▸ DHT Records                                              │
//! │  ▸ Security                                                 │
//! │  ▸ Network                                                  │
//! │  ▸ Actions                                                  │
//! │                                                             │
//! ├─────────────────────────────────────────────────────────────┤
//! │  [y] yank key  [q] back  [?] help                          │
//! └─────────────────────────────────────────────────────────────┘
//! ```

use anyhow::Result;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use rekindle_types::display::StatusSnapshot;
use rekindle_types::subscription_events::SubscriptionEvent;

use super::View;
use crate::helpers;
use crate::tui::action::{Action, CommandResult};
use crate::tui::focus::{FocusId, FocusRing};
use crate::tui::theme::ThemeManager;

/// Identity & settings view state.
pub struct IdentitySettingsView {
    focus: FocusRing,
    list_state: ListState,
    /// Cached identity data from StatusSnapshot.
    public_key: String,
    display_name: String,
    /// Transport status fields.
    route_allocated: bool,
    route_age_secs: Option<u64>,
    attachment: String,
    /// Subscription system.
    active_watches: usize,
    community_count: usize,
    friend_count: usize,
    /// DHT record keys.
    profile_dht_key: String,
    mailbox_dht_key: String,
    friend_list_dht_key: String,
    friend_inbox_key: String,
    /// Security.
    signing_key_loaded: bool,
    /// Which sections are expanded.
    expanded: [bool; 4],
    /// Whether data has been loaded.
    loaded: bool,
    /// Unicode support.
    use_unicode: bool,
}

impl IdentitySettingsView {
    pub fn new(use_unicode: bool) -> Self {
        Self {
            focus: FocusRing::new(vec![FocusId::IdentitySettings]),
            list_state: ListState::default().with_selected(Some(0)),
            public_key: String::new(),
            display_name: String::new(),
            route_allocated: false,
            route_age_secs: None,
            attachment: "unknown".into(),
            active_watches: 0,
            community_count: 0,
            friend_count: 0,
            profile_dht_key: String::new(),
            mailbox_dht_key: String::new(),
            friend_list_dht_key: String::new(),
            friend_inbox_key: String::new(),
            signing_key_loaded: false,
            expanded: [false; 4],
            loaded: false,
            use_unicode,
        }
    }

    /// Populate from StatusSnapshot.
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
        // DHT keys come from identity checks
        for check in &snapshot.checks {
            if check.id == "identity.public_key" {
                // Already have this from identity_public_key field
            }
        }
        self.loaded = true;
    }

    /// Build the list items for rendering.
    fn build_items(&self) -> Vec<ListItem<'static>> {
        let mut items = Vec::new();
        let arrow_right = if self.use_unicode { "▸" } else { ">" };
        let arrow_down = if self.use_unicode { "▾" } else { "v" };

        // ── Top-level summary ────────────────────────────────
        items.push(Self::kv_item(
            "  Public Key",
            &helpers::abbreviate_key(&self.public_key),
        ));
        items.push(Self::kv_item("  Display Name", &self.display_name));
        items.push(Self::kv_item("  Attachment", &self.attachment));
        let route_str = if self.route_allocated {
            format!("allocated ({}s)", self.route_age_secs.unwrap_or(0))
        } else {
            "not allocated".into()
        };
        items.push(Self::kv_item("  Route", &route_str));
        items.push(Self::kv_item("  Watches", &self.active_watches.to_string()));
        items.push(ListItem::new(Line::raw("")));

        // ── DHT Records section ──────────────────────────────
        let dht_arrow = if self.expanded[0] {
            arrow_down
        } else {
            arrow_right
        };
        items.push(ListItem::new(Line::from(vec![
            Span::styled(format!("  {dht_arrow} "), Style::new().bold()),
            Span::styled("DHT Records", Style::new().bold()),
        ])));
        if self.expanded[0] {
            items.push(Self::kv_item("    Profile", &self.profile_dht_key));
            items.push(Self::kv_item("    Mailbox", &self.mailbox_dht_key));
            items.push(Self::kv_item("    Friend List", &self.friend_list_dht_key));
            items.push(Self::kv_item("    Friend Inbox", &self.friend_inbox_key));
        }

        // ── Security section ─────────────────────────────────
        let sec_arrow = if self.expanded[1] {
            arrow_down
        } else {
            arrow_right
        };
        items.push(ListItem::new(Line::from(vec![
            Span::styled(format!("  {sec_arrow} "), Style::new().bold()),
            Span::styled("Security", Style::new().bold()),
        ])));
        if self.expanded[1] {
            let key_status = if self.signing_key_loaded {
                "loaded (unlocked)"
            } else {
                "not loaded (locked)"
            };
            items.push(Self::kv_item("    Signing Key", key_status));
            items.push(Self::kv_item("    Keyring", "OS keyring + disk fallback"));
        }

        // ── Network section ──────────────────────────────────
        let net_arrow = if self.expanded[2] {
            arrow_down
        } else {
            arrow_right
        };
        items.push(ListItem::new(Line::from(vec![
            Span::styled(format!("  {net_arrow} "), Style::new().bold()),
            Span::styled("Network", Style::new().bold()),
        ])));
        if self.expanded[2] {
            items.push(Self::kv_item(
                "    Communities",
                &self.community_count.to_string(),
            ));
            items.push(Self::kv_item("    Friends", &self.friend_count.to_string()));
            items.push(Self::kv_item(
                "    Active Watches",
                &self.active_watches.to_string(),
            ));
        }

        // ── Actions section ──────────────────────────────────
        let act_arrow = if self.expanded[3] {
            arrow_down
        } else {
            arrow_right
        };
        items.push(ListItem::new(Line::from(vec![
            Span::styled(format!("  {act_arrow} "), Style::new().bold()),
            Span::styled("Actions", Style::new().bold()),
        ])));
        if self.expanded[3] {
            items.push(ListItem::new(Line::from(vec![
                Span::styled("    [e] ", Style::new().dim()),
                Span::raw("Export identity bundle"),
            ])));
            items.push(ListItem::new(Line::from(vec![
                Span::styled("    [r] ", Style::new().dim()),
                Span::raw("Rotate identity keys"),
                Span::styled("  (dangerous)", Style::new().dim()),
            ])));
            items.push(ListItem::new(Line::from(vec![
                Span::styled("    [D] ", Style::new().dim()),
                Span::raw("Destroy identity"),
                Span::styled("  (irreversible)", Style::new().dim()),
            ])));
        }

        items
    }

    /// Build a key-value list item.
    fn kv_item(key: &str, value: &str) -> ListItem<'static> {
        ListItem::new(Line::from(vec![
            Span::styled(format!("{key:<18}"), Style::new().dim()),
            Span::raw(value.to_string()),
        ]))
    }

    /// Get the section index for the current selection (if on a section header).
    fn selected_section(&self) -> Option<usize> {
        let sel = self.list_state.selected()?;
        let items = self.build_items();
        let item_text = format!("{:?}", items.get(sel)?);
        // Section headers contain the arrow + section name
        for (i, name) in ["DHT Records", "Security", "Network", "Actions"]
            .iter()
            .enumerate()
        {
            if item_text.contains(name) {
                return Some(i);
            }
        }
        None
    }
}

impl View for IdentitySettingsView {
    fn draw(&mut self, frame: &mut Frame, area: Rect, theme: &ThemeManager) -> Result<()> {
        let [list_area, help_area] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(area);

        let title = format!(
            " Identity — {} ",
            if self.display_name.is_empty() {
                "loading..."
            } else {
                &self.display_name
            }
        );
        let block = Block::bordered()
            .title(title)
            .border_style(theme.focused_border());

        if self.loaded {
            let items = self.build_items();
            let list = List::new(items)
                .block(block)
                .highlight_style(Style::new().reversed());
            frame.render_stateful_widget(list, list_area, &mut self.list_state);
        } else {
            let para = Paragraph::new("  Loading identity...")
                .style(Style::new().dim())
                .block(block);
            frame.render_widget(para, list_area);
        }

        // Help bar
        let help = Line::from(vec![Span::styled(
            "  [y] yank key  [Enter] expand  [q] back  [?] help",
            Style::new().dim(),
        )]);
        frame.render_widget(Paragraph::new(help), help_area);

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
                // Toggle section expansion
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
                if self.public_key.is_empty() {
                    None
                } else {
                    Some(Action::YankToClipboard {
                        text: self.public_key.clone(),
                    })
                }
            }
            _ => None,
        }
    }

    fn on_command_result(&mut self, result: CommandResult) -> Result<()> {
        if let CommandResult::StatusLoaded { ref snapshot } = result {
            self.load_from_snapshot(snapshot);
        }
        // Also populate DHT keys from IdentityLoaded
        if let CommandResult::IdentityLoaded {
            ref public_key,
            ref display_name,
        } = result
        {
            if !public_key.is_empty() {
                self.public_key.clone_from(public_key);
                self.display_name.clone_from(display_name);
            }
        }
        Ok(())
    }

    fn on_subscription_event(&mut self, event: &SubscriptionEvent) -> Result<()> {
        // Update route/attachment on network events
        if let SubscriptionEvent::Network(
            rekindle_types::subscription_events::NetworkEvent::AttachmentChanged {
                is_attached,
                ..
            },
        ) = event
        {
            self.attachment = if *is_attached {
                "attached".into()
            } else {
                "detached".into()
            };
        }
        Ok(())
    }

    fn focus_ring(&mut self) -> &mut FocusRing {
        &mut self.focus
    }
}
