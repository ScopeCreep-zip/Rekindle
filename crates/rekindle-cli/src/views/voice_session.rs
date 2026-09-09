//! Voice session view — participant roster with controls.
//!
//! Displays voice channel participants with mute/deafen state,
//! and provides self-controls for mute, deafen, and leave.
//!
//! Layout:
//! ```text
//! ┌─ Voice: #gaming-voice ──────────────────────────────────┐
//! │                                                          │
//! │   ● alice (speaking)                                     │
//! │   ○ bob (muted)                                          │
//! │   ● carol                                                │
//! │                                                          │
//! ├──────────────────────────────────────────────────────────┤
//! │  [m] mute  [d] deafen  [l] leave  [?] help              │
//! └──────────────────────────────────────────────────────────┘
//! ```

use anyhow::Result;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, Paragraph};
use ratatui::Frame;

use rekindle_types::subscription_events::{SubscriptionEvent, VoiceEvent, VoiceScope};

use super::View;
use crate::helpers;
use crate::tui::action::{Action, CommandResult};
use crate::tui::focus::{FocusId, FocusRing};
use crate::tui::theme::ThemeManager;

/// One row in the TUI's voice participant list.
///
/// Named `...Row`, not `VoiceParticipant`, to stay distinct from
/// `rekindle_types::subscription_events::VoiceParticipant`, which is the
/// wire roster entry. This one carries view state (mute, deafen) that
/// the wire entry does not.
#[derive(Debug, Clone)]
pub struct VoiceParticipantRow {
    /// Pseudonym key.
    pub pseudonym_key: String,
    /// Display name.
    pub display_name: String,
    /// Whether the participant is muted.
    pub muted: bool,
    /// Whether the participant is deafened.
    pub deafened: bool,
}

/// Voice session view state.
pub struct VoiceSessionView {
    /// Community governance key.
    community: String,
    /// Voice channel ID.
    channel: String,
    /// Participants in the voice session.
    participants: Vec<VoiceParticipantRow>,
    /// Whether we are muted.
    self_muted: bool,
    /// Whether we are deafened.
    self_deafened: bool,
    /// Focus ring.
    focus: FocusRing,
    /// Unicode glyph support.
    use_unicode: bool,
}

impl VoiceSessionView {
    /// Whether a voice event belongs to the call this view is showing.
    ///
    /// A DM call never matches: this view is opened on a community
    /// channel, so a scope with no community is somebody else's call.
    fn is_this_call(&self, scope: &VoiceScope) -> bool {
        matches!(
            scope,
            VoiceScope::Community { community, channel }
                if *community == self.community && *channel == self.channel
        )
    }

    /// Create a new voice session view.
    pub fn new(community: String, channel: String, use_unicode: bool) -> Self {
        Self {
            community,
            channel,
            participants: Vec::new(),
            self_muted: false,
            self_deafened: false,
            focus: FocusRing::new(vec![FocusId::VoiceParticipants]),
            use_unicode,
        }
    }

    /// Build participant list items.
    fn build_items(&self) -> Vec<ListItem<'static>> {
        self.participants
            .iter()
            .map(|p| {
                let name = helpers::sanitize_for_display(&p.display_name);
                let glyph = if p.muted {
                    if self.use_unicode {
                        "○"
                    } else {
                        "."
                    }
                } else if self.use_unicode {
                    "●"
                } else {
                    "o"
                };

                let mut status_parts = Vec::new();
                if p.muted {
                    status_parts.push("[MUTED]");
                }
                if p.deafened {
                    status_parts.push("[DEAFENED]");
                }
                let status = if status_parts.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", status_parts.join(", "))
                };

                let line = Line::from(vec![
                    Span::raw(format!("   {glyph} ")),
                    Span::styled(name, Style::new().bold()),
                    Span::styled(status, Style::new().dim()),
                ]);
                ListItem::new(line)
            })
            .collect()
    }
}

impl View for VoiceSessionView {
    fn draw(&mut self, frame: &mut Frame, area: Rect, theme: &ThemeManager) -> Result<()> {
        let [participant_area, controls_area] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(3)]).areas(area);

        // Participant list
        let community_short = helpers::abbreviate_key(&self.community);
        let title = format!(
            " Voice: {community_short} / #{} ({} participants) ",
            self.channel,
            self.participants.len()
        );
        let block = Block::bordered()
            .title(title)
            .border_style(theme.focused_border());

        if self.participants.is_empty() {
            let para = Paragraph::new("  No participants — waiting for others to join...")
                .style(Style::new().dim())
                .block(block);
            frame.render_widget(para, participant_area);
        } else {
            let items = self.build_items();
            let list = List::new(items).block(block);
            frame.render_widget(list, participant_area);
        }

        // Controls bar
        let mute_label = if self.self_muted {
            "[m] unmute"
        } else {
            "[m] mute"
        };
        let deafen_label = if self.self_deafened {
            "[d] undeafen"
        } else {
            "[d] deafen"
        };

        let controls = Line::from(vec![
            Span::raw("  "),
            Span::styled(mute_label, Style::new().bold()),
            Span::raw("  "),
            Span::styled(deafen_label, Style::new().bold()),
            Span::raw("  "),
            Span::styled("[l] leave", Style::new().bold()),
            Span::raw("  "),
            Span::styled("[?] help", Style::new().dim()),
        ]);

        let controls_block = Block::bordered()
            .title(" Controls ")
            .border_style(theme.unfocused_border());
        frame.render_widget(
            Paragraph::new(controls).block(controls_block),
            controls_area,
        );

        Ok(())
    }

    fn update(&mut self, action: Action) -> Result<Option<Action>> {
        match action {
            Action::ToggleMute => {
                self.self_muted = !self.self_muted;
                // Don't return ToggleMute — that would re-enter this handler
                // and create an infinite toggle loop. The local state is updated;
                // M3 wires the transport side in process_action directly.
            }
            Action::ToggleDeafen => {
                self.self_deafened = !self.self_deafened;
            }
            Action::LeaveVoice => {
                return Ok(Some(Action::LeaveVoice));
            }
            _ => {}
        }
        Ok(None)
    }

    fn on_subscription_event(&mut self, event: &SubscriptionEvent) -> Result<()> {
        match event {
            SubscriptionEvent::Voice(VoiceEvent::Joined {
                scope, pseudonym, ..
            }) if self.is_this_call(scope) => {
                if !self
                    .participants
                    .iter()
                    .any(|p| p.pseudonym_key == *pseudonym)
                {
                    self.participants.push(VoiceParticipantRow {
                        pseudonym_key: pseudonym.clone(),
                        display_name: helpers::abbreviate_key(pseudonym),
                        muted: false,
                        deafened: false,
                    });
                }
            }
            SubscriptionEvent::Voice(VoiceEvent::Left { scope, pseudonym })
                if self.is_this_call(scope) =>
            {
                self.participants.retain(|p| p.pseudonym_key != *pseudonym);
            }
            SubscriptionEvent::Voice(VoiceEvent::MuteChanged {
                scope,
                target_pseudonym,
                muted,
            }) if self.is_this_call(scope) => {
                if let Some(p) = self
                    .participants
                    .iter_mut()
                    .find(|p| p.pseudonym_key == *target_pseudonym)
                {
                    p.muted = *muted;
                }
            }
            SubscriptionEvent::Voice(VoiceEvent::DeafenChanged {
                scope,
                target_pseudonym,
                deafened,
            }) if self.is_this_call(scope) => {
                if let Some(p) = self
                    .participants
                    .iter_mut()
                    .find(|p| p.pseudonym_key == *target_pseudonym)
                {
                    p.deafened = *deafened;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn on_command_result(&mut self, _result: CommandResult) -> Result<()> {
        Ok(())
    }

    fn handle_focused_key(&mut self, key: crossterm::event::KeyEvent) -> Option<Action> {
        use crossterm::event::KeyCode;
        // Voice controls advertised in the controls bar: [m] mute, [d] deafen, [l] leave.
        match key.code {
            KeyCode::Char('m') => Some(Action::ToggleMute),
            KeyCode::Char('d') => Some(Action::ToggleDeafen),
            KeyCode::Char('l') => Some(Action::LeaveVoice),
            _ => None,
        }
    }

    fn focus_ring(&mut self) -> &mut FocusRing {
        &mut self.focus
    }
}
