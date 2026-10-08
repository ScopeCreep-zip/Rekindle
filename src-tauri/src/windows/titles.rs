//! OS window titles, resolved from backend state.
//!
//! The webview never supplies a window title: a title is shown by the OS
//! (task switcher, taskbar, Mission Control), outside the app's own chrome,
//! so it is built here from the friend list, the community member table
//! and the DM store, and stripped of control and bidi characters.

use std::sync::Arc;

use rekindle_types::key_format::{PublicKeyHex, RecordKeyStr};

use crate::db_helpers::db_call_or_default;
use crate::state::AppState;
use crate::state_helpers;
use rekindle_db::Db;

/// Longest name shown in a window title, in chars.
const MAX_TITLE_NAME: usize = 64;
/// Hex digits of a key shown when no name is known.
const SHORT_KEY_LEN: usize = 8;

/// Drop C0/C1 controls and Unicode bidi controls (CWE-451 spoofing), trim,
/// and cap at [`MAX_TITLE_NAME`] chars.
fn title_text(s: &str) -> String {
    let is_bidi = |c: char| matches!(c, '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}');
    s.chars()
        .filter(|&c| !c.is_control() && !is_bidi(c))
        .collect::<String>()
        .trim()
        .chars()
        .take(MAX_TITLE_NAME)
        .collect()
}

fn short_key(key: &str) -> String {
    format!("{}…", rekindle_utils::text::prefix(key, SHORT_KEY_LEN))
}

/// A peer's name: friend nickname or display name, then a community member
/// display name for a pseudonym key, then the short key.
async fn peer_name(state: &Arc<AppState>, pool: &Db, key: &str) -> String {
    let friend = state.friends.read().get(key).map(|f| {
        f.nickname
            .clone()
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| f.display_name.clone())
    });
    if let Some(name) = friend.map(|n| title_text(&n)).filter(|n| !n.is_empty()) {
        return name;
    }
    let owner = state_helpers::owner_key_or_default(state);
    let pseudonym = key.to_owned();
    let member: Option<String> = db_call_or_default(pool, move |conn| {
        rekindle_db::repo::members::any_display_name(conn, &owner, &pseudonym)
    })
    .await;
    member
        .map(|n| title_text(&n))
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| short_key(key))
}

pub async fn chat(state: &Arc<AppState>, pool: &Db, peer: &PublicKeyHex) -> String {
    format!("Chat - {}", peer_name(state, pool, peer.as_str()).await)
}

pub async fn profile(state: &Arc<AppState>, pool: &Db, peer: &PublicKeyHex) -> String {
    format!("Profile - {}", peer_name(state, pool, peer.as_str()).await)
}

/// `DM - <peer>` for a 1:1 DM, `Group DM` for a group.
pub async fn dm(state: &Arc<AppState>, pool: &Db, record: &RecordKeyStr) -> String {
    let me = state_helpers::owner_key_or_default(state);
    let conversations = crate::services::dm::list_dm_conversations(state, pool).await;
    let Some(conv) = conversations
        .into_iter()
        .find(|c| c.record_key == record.as_str())
    else {
        return "Direct Message".to_owned();
    };
    if conv.is_group {
        return "Group DM".to_owned();
    }
    match conv.participants.iter().find(|p| p.public_key != me) {
        Some(peer) => format!("DM - {}", peer_name(state, pool, &peer.public_key).await),
        None => "Direct Message".to_owned(),
    }
}

/// `Community - <name>`, or `Communities` for the community browser.
pub fn community(state: &Arc<AppState>, community: Option<&RecordKeyStr>) -> String {
    let Some(id) = community else {
        return "Communities".to_owned();
    };
    let name = state
        .communities
        .read()
        .get(id.as_str())
        .map(|c| title_text(&c.name))
        .filter(|n| !n.is_empty());
    match name {
        Some(name) => format!("Community - {name}"),
        None => "Community".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_text_strips_controls_and_bidi() {
        assert_eq!(title_text("  Alice\u{7}  "), "Alice");
        assert_eq!(title_text("evil\u{202E}gpj.exe"), "evilgpj.exe");
        assert_eq!(title_text("a\u{2066}b\u{200F}c\nd"), "abcd");
        assert_eq!(title_text(&"é".repeat(100)).chars().count(), MAX_TITLE_NAME);
        assert_eq!(title_text("\u{202E}\u{0}"), "");
    }

    #[test]
    fn short_key_is_prefix() {
        assert_eq!(short_key("3f1a9c0b7e2d4f6a"), "3f1a9c0b…");
    }
}
