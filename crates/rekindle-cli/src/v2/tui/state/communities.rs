//! Community state — list, detail cache, member lists, peer snapshots,
//! invites, events, onboarding.

use std::collections::HashMap;
use std::time::Instant;

use rekindle_types::display::{CommunityDetail, CommunityOverview, MemberWithPresence, PeerSnapshot};
use rekindle_types::dht_types::{OnboardingConfig, WelcomeScreen};

#[derive(Clone, Debug)]
pub struct CommunitySnapshot {
    pub governance_key: String,
    pub name: String,
    pub pseudonym: String,
    pub is_operator: bool,
}

impl From<&CommunityOverview> for CommunitySnapshot {
    fn from(c: &CommunityOverview) -> Self {
        Self {
            governance_key: c.governance_key.clone(),
            name: c.name.clone(),
            pseudonym: c.pseudonym.clone(),
            is_operator: c.is_operator,
        }
    }
}

#[derive(Debug)]
pub struct CommunityState {
    pub list: Vec<CommunitySnapshot>,
    /// Keyed by governance_key.
    pub details: HashMap<String, CommunityDetail>,
    /// Keyed by governance_key.
    pub members: HashMap<String, Vec<MemberWithPresence>>,
    pub peer_list: Option<Vec<PeerSnapshot>>,
    /// Keyed by governance_key.
    pub invites: HashMap<String, Vec<serde_json::Value>>,
    /// Keyed by governance_key.
    pub events: HashMap<String, Vec<serde_json::Value>>,
    /// Keyed by governance_key.
    pub onboarding: HashMap<String, (Option<OnboardingConfig>, Option<WelcomeScreen>)>,
    /// Keyed by governance_key.
    pub bans: HashMap<String, Vec<serde_json::Value>>,
    /// Keyed by governance_key.
    pub pending_members: HashMap<String, Vec<serde_json::Value>>,
    /// Per-community selected channel index for community_info view navigation.
    pub selected_channel: HashMap<String, usize>,
    pub list_loaded: bool,
    pub list_loaded_at: Option<Instant>,
}

impl CommunityState {
    pub fn new() -> Self {
        Self {
            list: Vec::new(),
            details: HashMap::new(),
            members: HashMap::new(),
            peer_list: None,
            invites: HashMap::new(),
            events: HashMap::new(),
            onboarding: HashMap::new(),
            bans: HashMap::new(),
            pending_members: HashMap::new(),
            selected_channel: HashMap::new(),
            list_loaded: false,
            list_loaded_at: None,
        }
    }

    /// Look up community name by governance key. Falls back to the key itself.
    pub fn name_for<'a>(&'a self, governance_key: &'a str) -> &'a str {
        self.list
            .iter()
            .find(|c| c.governance_key == governance_key)
            .map_or(governance_key, |c| c.name.as_str())
    }
}

impl Default for CommunityState {
    fn default() -> Self {
        Self::new()
    }
}
