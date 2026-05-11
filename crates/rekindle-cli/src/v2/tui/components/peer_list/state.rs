//! Peer list state — member data, presence sorting, name resolution.

use ratatui::widgets::ListState;

/// A member entry for display.
#[derive(Debug, Clone)]
pub struct PeerEntry {
    pub key: String,
    pub display_name: String,
    pub status: String,
    pub role: Option<String>,
}

/// Peer list component state.
pub struct PeerList {
    pub(super) members: Vec<PeerEntry>,
    pub(super) list_state: ListState,
    pub(super) is_focused: bool,
    pub(super) use_unicode: bool,
}

impl PeerList {
    pub fn new(use_unicode: bool) -> Self {
        Self { members: Vec::new(), list_state: ListState::default(), is_focused: false, use_unicode }
    }

    pub fn set_members(&mut self, mut members: Vec<PeerEntry>) {
        members.sort_by(|a, b| {
            presence_rank(&a.status).cmp(&presence_rank(&b.status))
                .then(a.display_name.cmp(&b.display_name))
        });
        self.members = members;
    }

    pub fn update_member_status(&mut self, pseudonym: &str, status: &str) {
        if let Some(m) = self.members.iter_mut().find(|m| m.key == pseudonym) {
            m.status = status.to_string();
        }
        self.members.sort_by(|a, b| {
            presence_rank(&a.status).cmp(&presence_rank(&b.status))
                .then(a.display_name.cmp(&b.display_name))
        });
    }

    pub fn resolve_name(&self, pseudonym: &str) -> Option<String> {
        self.members.iter().find(|m| m.key == pseudonym).map(|m| m.display_name.clone())
    }

    pub fn len(&self) -> usize { self.members.len() }
    pub fn is_empty(&self) -> bool { self.members.is_empty() }
}

pub fn presence_rank(status: &str) -> u8 {
    match status { "online" => 0, "away" => 1, "busy" => 2, "offline" => 3, _ => 4 }
}

pub fn capitalize_status(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => {
            let upper: String = first.to_uppercase().collect();
            format!("{upper}{}", chars.as_str())
        }
    }
}
