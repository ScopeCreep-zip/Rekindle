//! The Infoview as text (plan E7.15, IV7): the Host's `InfoviewView`, its
//! sections in the Host's order, as headings and rows for the CLI and the
//! TUI. The Host composes every section, row and verdict (memory: backend
//! owns policy); this only names them. Every peer-supplied string passes
//! `sanitize_for_display`.

use std::fmt::Write as _;

use rekindle_types::activity::{ActivityKind, ActivityListItem};
use rekindle_types::dto::friends::Friend;
use rekindle_types::dto::infoview::{InfoviewAction, InfoviewSection, InfoviewView};
use rekindle_types::friend::NotJoinableReason;
use rekindle_utils::text::sanitize_for_display;

use crate::fmt::{abbreviate_key, format_timestamp};

/// One line of a rendered Infoview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InfoviewLine {
    /// A section's heading.
    Heading(String),
    /// One of its rows.
    Row(String),
}

/// `view`'s sections, in the Host's order, as headings and rows.
#[must_use]
pub fn lines(view: &InfoviewView) -> Vec<InfoviewLine> {
    let mut out = Vec::new();
    for section in &view.sections {
        section_lines(section, &mut out);
    }
    out
}

fn section_lines(section: &InfoviewSection, out: &mut Vec<InfoviewLine>) {
    let heading = |out: &mut Vec<InfoviewLine>, h: String| out.push(InfoviewLine::Heading(h));
    let row = |out: &mut Vec<InfoviewLine>, r: String| out.push(InfoviewLine::Row(r));
    match section {
        InfoviewSection::Activity { entries, more } => {
            heading(out, format!("Activity ({})", entries.len()));
            for e in entries {
                row(out, activity_row(e));
            }
            if more.is_some() {
                row(out, "[View more]".into());
            }
        }
        InfoviewSection::Communities { rows } => {
            heading(out, format!("Communities ({})", rows.len()));
            for c in rows {
                let mut r = format!(
                    "{}  unread {}  mentions {}",
                    sanitize_for_display(&c.name),
                    c.unread,
                    c.mentions
                );
                if let Some(e) = &c.next_event {
                    let _ = write!(r, "  next: {}", event(e));
                }
                row(out, r);
            }
        }
        InfoviewSection::NowPlaying { rows } => {
            heading(out, format!("Playing now ({})", rows.len()));
            for p in rows {
                let mut r = format!(
                    "{}  {}  {}m",
                    friend_label(&p.friend),
                    sanitize_for_display(&p.game_name),
                    p.elapsed_seconds / 60
                );
                if let Some(server) = &p.server_info {
                    let _ = write!(r, "  {}", sanitize_for_display(server));
                }
                r.push_str(&join(&p.join));
                row(out, r);
            }
        }
        InfoviewSection::FriendHeader {
            friend, blocked, ..
        } => {
            heading(out, "Friend".into());
            let mut r = format!("{}  {}", friend_label(friend), friend.status.as_wire_str());
            if *blocked {
                r.push_str("  [blocked]");
            }
            row(out, r);
            if let Some(message) = &friend.status_message {
                row(out, format!("\"{}\"", sanitize_for_display(message)));
            }
        }
        InfoviewSection::Bio { text } => {
            heading(out, "Bio".into());
            row(out, sanitize_for_display(text));
        }
        InfoviewSection::GroupMembers {
            label,
            online,
            offline,
        } => {
            heading(
                out,
                format!(
                    "{} ({})",
                    sanitize_for_display(label),
                    online.len() + offline.len()
                ),
            );
            for f in online {
                row(out, format!("online  {}", friend_label(f)));
            }
            for f in offline {
                row(out, format!("offline  {}", friend_label(f)));
            }
        }
        InfoviewSection::CommunityHeader {
            name,
            description,
            member_count,
            online_count,
            removed,
            ..
        } => {
            heading(out, sanitize_for_display(name));
            if !description.is_empty() {
                row(out, sanitize_for_display(description));
            }
            row(
                out,
                format!("{member_count} members, {online_count} online"),
            );
            if let Some(removed) = removed {
                let what = if removed.banned { "banned" } else { "removed" };
                let mut r = format!("You were {what}");
                if let Some(by) = removed.by_name.as_deref().or(removed.by.as_deref()) {
                    let _ = write!(r, " by {}", sanitize_for_display(by));
                }
                if let Some(reason) = &removed.reason {
                    let _ = write!(r, ": {}", sanitize_for_display(reason));
                }
                row(out, r);
            }
        }
        InfoviewSection::Pinned { pins } => {
            heading(out, format!("Pinned ({})", pins.len()));
            for p in pins {
                row(
                    out,
                    format!(
                        "{}  channel {}  message {}  by {}",
                        format_timestamp(p.pinned_at.saturating_mul(1000)),
                        abbreviate_key(&p.channel_id),
                        abbreviate_key(&p.message_id),
                        abbreviate_key(&p.pinned_by)
                    ),
                );
            }
        }
        InfoviewSection::Events { events } => {
            heading(out, format!("Events ({})", events.len()));
            for e in events {
                row(out, event(e));
            }
        }
        InfoviewSection::GameServers { servers } => {
            heading(out, format!("Servers ({})", servers.len()));
            for s in servers {
                let mut r = format!(
                    "{}  {}  game {}",
                    sanitize_for_display(&s.label),
                    sanitize_for_display(&s.address),
                    sanitize_for_display(&s.game_id)
                );
                r.push_str(&join(&s.join));
                row(out, r);
            }
        }
    }
}

/// An event: its start and title.
fn event(e: &rekindle_types::event::EventInfo) -> String {
    format!(
        "{}  {}",
        format_timestamp(e.start_time.saturating_mul(1000)),
        sanitize_for_display(&e.title)
    )
}

/// A row's Join with the Host's verdict: plain when joinable, the reason
/// when not.
fn join(action: &InfoviewAction) -> String {
    match action {
        InfoviewAction::JoinServer { joinable: true, .. } => "  [Join]".into(),
        InfoviewAction::JoinServer {
            joinable_reason, ..
        } => match joinable_reason {
            Some(NotJoinableReason::NoServer) => "  [Join: not on a server]".into(),
            Some(NotJoinableReason::CannotLaunch) => {
                "  [Join: Rekindle cannot start this game on a server]".into()
            }
            None => "  [Join: not joinable]".into(),
        },
        _ => String::new(),
    }
}

/// A friend by the names the Host gave and their key.
fn friend_label(f: &Friend) -> String {
    let mut label = String::new();
    if let Some(nickname) = &f.nickname {
        label.push_str(&sanitize_for_display(nickname));
        label.push(' ');
    }
    label.push_str(&sanitize_for_display(&f.display_name));
    let _ = write!(label, " ({})", abbreviate_key(&f.public_key));
    if f.identity_pin_mismatch {
        label.push_str(" [identity pin mismatch]");
    }
    label
}

/// A feed entry's row: its time, its kind's name, then its fields as the
/// DTO carries them (a friend by the names the Host gave and their key).
#[must_use]
pub fn activity_row(e: &ActivityListItem) -> String {
    let at = u64::try_from(e.at).map_or_else(|_| e.at.to_string(), format_timestamp);
    let who = e.kind.friend().map(|key| entry_friend(e, key));
    let fields = fields(&e.kind);
    let mut row = format!("{at}  {}", e.kind.name());
    for part in [who, fields].into_iter().flatten() {
        row.push_str("  ");
        row.push_str(&part);
    }
    row
}

fn entry_friend(e: &ActivityListItem, key: &str) -> String {
    let mut label = String::new();
    for name in [e.nickname.as_deref(), e.display_name.as_deref()]
        .into_iter()
        .flatten()
    {
        label.push_str(&sanitize_for_display(name));
        label.push(' ');
    }
    label.push('(');
    label.push_str(&abbreviate_key(key));
    label.push(')');
    if e.identity_pin_mismatch {
        label.push_str(" [identity pin mismatch]");
    }
    label
}

fn fields(kind: &ActivityKind) -> Option<String> {
    let text = |s: &str| sanitize_for_display(s);
    match kind {
        ActivityKind::StartedGame {
            game_id, server, ..
        } => Some(match server {
            Some(server) => format!("game {game_id}  {}", text(server)),
            None => format!("game {game_id}"),
        }),
        ActivityKind::StoppedGame {
            game_id, seconds, ..
        } => Some(format!("game {game_id}  {seconds}s")),
        ActivityKind::FirstPlayed { game_id, .. } => Some(format!("game {game_id}")),
        ActivityKind::RenamedSelf { from, to, .. } => {
            Some(format!("\"{}\" -> \"{}\"", text(from), text(to)))
        }
        ActivityKind::StatusMessage { text: message, .. } => Some(format!("\"{}\"", text(message))),
        ActivityKind::BioChanged { .. } | ActivityKind::NowFriends { .. } => None,
        ActivityKind::FavouriteAdded {
            game_id,
            address,
            label,
        } => Some(format!(
            "game {game_id}  {}  \"{}\"",
            text(address),
            text(label)
        )),
        ActivityKind::JoinedCommunity { community } => Some(abbreviate_key(community)),
    }
}

#[cfg(test)]
#[path = "infoview_tests.rs"]
mod tests;
