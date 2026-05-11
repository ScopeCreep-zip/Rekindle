//! Voice session view — participant roster with mute/deafen/leave controls.

use anyhow::Result;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, Paragraph};
use ratatui::Frame;

use rekindle_types::subscription_events::{SubscriptionEvent, VoiceEvent};

use crate::v2::helpers;
use crate::v2::tui::action::{Action, CommandResult};
use crate::v2::tui::focus::{FocusId, FocusRing};
use crate::v2::tui::theme::ThemeManager;
use super::View;

#[derive(Debug, Clone)]
pub struct VoiceParticipant {
    pub pseudonym_key: String,
    pub display_name: String,
    pub muted: bool,
    pub deafened: bool,
}

pub struct VoiceSessionView {
    community: String,
    channel: String,
    participants: Vec<VoiceParticipant>,
    self_muted: bool,
    self_deafened: bool,
    focus: FocusRing,
    use_unicode: bool,
}

impl VoiceSessionView {
    pub fn new(community: String, channel: String, use_unicode: bool) -> Self {
        Self {
            community, channel, participants: Vec::new(),
            self_muted: false, self_deafened: false,
            focus: FocusRing::new(vec![FocusId::VoiceParticipants]), use_unicode,
        }
    }
}

impl super::ViewQuery for VoiceSessionView {}

impl View for VoiceSessionView {
    fn draw(&mut self, frame: &mut Frame, area: Rect, theme: &ThemeManager) -> Result<()> {
        let [participant_area, controls_area] = Layout::vertical([
            Constraint::Fill(1), Constraint::Length(3),
        ]).areas(area);

        let community_short = helpers::abbreviate_key(&self.community);
        let title = format!(" Voice: {community_short} / #{} ({} participants) ", self.channel, self.participants.len());
        let block = Block::bordered().title(title).border_style(theme.focused_border());

        if self.participants.is_empty() {
            frame.render_widget(
                Paragraph::new("  No participants — waiting for others to join...")
                    .style(theme.style("dim")).block(block),
                participant_area,
            );
        } else {
            let items: Vec<ListItem<'_>> = self.participants.iter().map(|p| {
                let name = helpers::sanitize_for_display(&p.display_name);
                let glyph = if p.muted { if self.use_unicode { "○" } else { "." } }
                else if self.use_unicode { "●" } else { "o" };
                let mut status_parts = Vec::new();
                if p.muted { status_parts.push("[MUTED]"); }
                if p.deafened { status_parts.push("[DEAFENED]"); }
                let status = if status_parts.is_empty() { String::new() }
                else { format!(" ({})", status_parts.join(", ")) };

                ListItem::new(Line::from(vec![
                    Span::raw(format!("   {glyph} ")),
                    Span::styled(name, Style::new().bold()),
                    Span::styled(status, Style::new().dim()),
                ]))
            }).collect();
            frame.render_widget(List::new(items).block(block), participant_area);
        }

        let mute_label = if self.self_muted { "[m] unmute" } else { "[m] mute" };
        let deafen_label = if self.self_deafened { "[d] undeafen" } else { "[d] deafen" };
        let controls = Line::from(vec![
            Span::raw("  "), Span::styled(mute_label, Style::new().bold()),
            Span::raw("  "), Span::styled(deafen_label, Style::new().bold()),
            Span::raw("  "), Span::styled("[l] leave", Style::new().bold()),
            Span::raw("  "), Span::styled("[?] help", Style::new().dim()),
        ]);
        frame.render_widget(
            Paragraph::new(controls).block(Block::bordered().title(" Controls ").border_style(theme.unfocused_border())),
            controls_area,
        );
        Ok(())
    }

    fn update(&mut self, action: Action) -> Result<Option<Action>> {
        match action {
            Action::ToggleMute => { self.self_muted = !self.self_muted; }
            Action::ToggleDeafen => { self.self_deafened = !self.self_deafened; }
            Action::LeaveVoice => { return Ok(Some(Action::LeaveVoice)); }
            _ => {}
        }
        Ok(None)
    }

    fn on_subscription_event(&mut self, event: &SubscriptionEvent) -> Result<()> {
        match event {
            SubscriptionEvent::Voice(VoiceEvent::Joined { community, channel, pseudonym })
                if *community == self.community && *channel == self.channel =>
            {
                if !self.participants.iter().any(|p| p.pseudonym_key == *pseudonym) {
                    self.participants.push(VoiceParticipant {
                        pseudonym_key: pseudonym.clone(),
                        display_name: helpers::abbreviate_key(pseudonym),
                        muted: false, deafened: false,
                    });
                }
            }
            SubscriptionEvent::Voice(VoiceEvent::Left { community, channel, pseudonym })
                if *community == self.community && *channel == self.channel =>
            {
                self.participants.retain(|p| p.pseudonym_key != *pseudonym);
            }
            SubscriptionEvent::Voice(VoiceEvent::MuteChanged { community, channel, target_pseudonym, muted })
                if *community == self.community && *channel == self.channel =>
            {
                if let Some(p) = self.participants.iter_mut().find(|p| p.pseudonym_key == *target_pseudonym) {
                    p.muted = *muted;
                }
            }
            SubscriptionEvent::Voice(VoiceEvent::DeafenChanged { community, channel, target_pseudonym, deafened })
                if *community == self.community && *channel == self.channel =>
            {
                if let Some(p) = self.participants.iter_mut().find(|p| p.pseudonym_key == *target_pseudonym) {
                    p.deafened = *deafened;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn on_command_result(&mut self, _result: CommandResult) -> Result<()> { Ok(()) }
    fn focus_ring(&mut self) -> &mut FocusRing { &mut self.focus }
}
