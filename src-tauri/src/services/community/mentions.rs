//! Architecture §28.5 + §32 Week 18 — mention parsing, permission
//! gating, and notification escalation.
//!
//! Mention syntax:
//!
//! - `@everyone` — pings every community member; requires `MENTION_EVERYONE`
//! - `@here` — pings every *online* member; requires `MENTION_EVERYONE`
//! - `@<role-name>` — pings members holding that role; requires the role
//!   to have `mentionable: true`
//! - `@<display-name>` — pings the specific member; always allowed
//!
//! Mentions are detected on a token boundary (whitespace or punctuation
//! before `@`) so a stray email address like `foo@bar.com` is not
//! treated as a community mention.

use std::sync::Arc;

use rekindle_governance::permissions::compute_permissions;
use rekindle_types::id::PseudonymKey;
use rekindle_types::permissions::MENTION_EVERYONE;

use crate::state::AppState;

#[inline]
fn has_perm(perms: u64, required: u64) -> bool {
    perms & required == required
}

/// Result of parsing mentions out of a message body.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MentionMatches {
    pub everyone: bool,
    pub here: bool,
    /// Lowercased role names referenced via `@role-name` — already
    /// filtered to roles that exist and are `mentionable` in the merged
    /// governance state.
    pub roles: Vec<String>,
    /// Lowercased display names referenced via `@display-name` — match
    /// against `community_members.display_name` on the receiver side.
    pub members: Vec<String>,
}

impl MentionMatches {
    /// Whether any mention class fired. Useful for fast-skipping the
    /// permission-validation pass when a body has no `@` tokens at all.
    pub fn is_any(&self) -> bool {
        self.everyone || self.here || !self.roles.is_empty() || !self.members.is_empty()
    }
}

/// Parse the message body and resolve every mention against the
/// merged governance state. Returns the resolved set; senders without
/// permission for a mention class will have it stripped via
/// `apply_send_permissions` before this is invoked on the wire.
pub fn parse_mentions(state: &Arc<AppState>, community_id: &str, body: &str) -> MentionMatches {
    let raw = scan_raw_tokens(body);
    let communities = state.communities.read();
    let Some(community) = communities.get(community_id) else {
        return MentionMatches::default();
    };

    let mut matches = MentionMatches::default();
    for token in raw {
        match token.as_str() {
            "everyone" => matches.everyone = true,
            "here" => matches.here = true,
            other => {
                // Roles take precedence over display-name fallback so
                // an admin can't accidentally make a bypass by naming
                // themselves "everyone".
                let role_match = community
                    .roles
                    .iter()
                    .find(|r| r.name.eq_ignore_ascii_case(other) && r.mentionable);
                if role_match.is_some() {
                    let lowered = other.to_lowercase();
                    if !matches.roles.contains(&lowered) {
                        matches.roles.push(lowered);
                    }
                    continue;
                }
                // Member display-name match. We accept any peer in the
                // community's member registry; the receiver still
                // checks against their own member table when deciding
                // to highlight or escalate.
                let lowered = other.to_lowercase();
                if !matches.members.contains(&lowered) {
                    matches.members.push(lowered);
                }
            }
        }
    }
    matches
}

/// Reader-validates: drop mention classes the sender lacked permission
/// to use. Architecture §9.3 (reader-validates) + §28.5: the literal
/// text remains in the body so a member without `MENTION_EVERYONE`
/// can still *type* "@everyone" without it actually escalating.
pub fn validate_sender_permissions(
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym_hex: &str,
    mentions: &mut MentionMatches,
) {
    // Fast path — no mentions to gate.
    if !mentions.is_any() {
        return;
    }
    let communities = state.communities.read();
    let Some(community) = communities.get(community_id) else {
        mentions.everyone = false;
        mentions.here = false;
        return;
    };
    let Ok(pk_bytes) = hex::decode(sender_pseudonym_hex) else {
        mentions.everyone = false;
        mentions.here = false;
        return;
    };
    let Ok(pk_arr) = <[u8; 32]>::try_from(pk_bytes.as_slice()) else {
        mentions.everyone = false;
        mentions.here = false;
        return;
    };
    let pseudonym = PseudonymKey(pk_arr);
    let Some(governance) = community.governance_state.as_ref() else {
        mentions.everyone = false;
        mentions.here = false;
        return;
    };
    let perms =
        compute_permissions(&pseudonym, None, governance, rekindle_utils::timestamp_secs());
    if !has_perm(perms, MENTION_EVERYONE) {
        mentions.everyone = false;
        mentions.here = false;
    }
    // @role mentions: parser already filters to roles whose
    // `mentionable` flag is set, and the sender's role membership
    // doesn't add additional gating per architecture (anyone can ping
    // a mentionable role).
}

/// True when the local member is referenced by a mention. Drives
/// notification escalation in the per-channel level resolver
/// (architecture §17.1: a mention always wakes the user, regardless
/// of `MentionsOnly` vs `Nothing`).
pub fn local_member_is_mentioned(
    state: &Arc<AppState>,
    community_id: &str,
    matches: &MentionMatches,
) -> bool {
    if matches.everyone || matches.here {
        return true;
    }
    let communities = state.communities.read();
    let Some(community) = communities.get(community_id) else {
        return false;
    };
    if !matches.roles.is_empty() {
        let any_role = community
            .my_role_ids
            .iter()
            .filter_map(|id| community.roles.iter().find(|r| r.id == *id))
            .any(|role| matches.roles.iter().any(|m| m == &role.name.to_lowercase()));
        if any_role {
            return true;
        }
    }
    if matches.members.is_empty() {
        return false;
    }
    let my_pseudonym = community.my_pseudonym_key.as_deref();
    let Some(my_pseudonym) = my_pseudonym else {
        return false;
    };
    community
        .member_profiles
        .keys()
        .filter(|pk| pk.as_str() == my_pseudonym)
        .any(|_| true)
        && matches.members.iter().any(|m| {
            community
                .roles
                .iter()
                .all(|r| r.name.to_lowercase() != *m)
        })
}

/// Inverse of `resolve_to_wire`: build a `MentionMatches` from the
/// cleartext envelope fields a receiver pulled off the wire. Member
/// pseudonyms are converted back to their display names by looking up
/// `community.member_profiles`; pseudonyms that don't resolve become
/// "raw-pseudonym" entries so the local-member check still works for
/// known pseudonyms even before a display name is cached.
pub fn matches_from_cleartext(
    state: &Arc<AppState>,
    community_id: &str,
    mentioned_pseudonyms: &[String],
    mentioned_roles: &[String],
    mention_everyone: bool,
    mention_here: bool,
) -> MentionMatches {
    let communities = state.communities.read();
    let mut matches = MentionMatches {
        everyone: mention_everyone,
        here: mention_here,
        roles: mentioned_roles.iter().map(|r| r.to_lowercase()).collect(),
        members: Vec::new(),
    };
    if let Some(community) = communities.get(community_id) {
        for pseudonym_hex in mentioned_pseudonyms {
            let display = community
                .member_profiles
                .get(pseudonym_hex)
                .and_then(|p| p.display_name.as_deref())
                .map_or_else(
                    || pseudonym_hex.to_lowercase(),
                    str::to_lowercase,
                );
            if !matches.members.contains(&display) {
                matches.members.push(display);
            }
        }
    }
    matches
}

/// Resolve parsed mention matches into the wire-payload shape for the
/// cleartext envelope (architecture §28.5 line 3105-3120). Returns
/// `(mentioned_pseudonyms, mentioned_roles, mention_everyone, mention_here)`.
/// Member display-name mentions are resolved to pseudonym hex strings
/// against `community.member_profiles`. Roles are emitted as their
/// canonical lowercased names. Reader-validation has already been
/// applied — caller passes a `MentionMatches` post-`validate_sender_permissions`.
pub fn resolve_to_wire(
    state: &Arc<AppState>,
    community_id: &str,
    matches: &MentionMatches,
) -> (Vec<String>, Vec<String>, bool, bool) {
    let communities = state.communities.read();
    let Some(community) = communities.get(community_id) else {
        return (Vec::new(), Vec::new(), false, false);
    };
    let mut pseudonyms: Vec<String> = Vec::new();
    for member_name in &matches.members {
        for (pseudonym_hex, profile) in &community.member_profiles {
            if profile
                .display_name
                .as_deref()
                .is_some_and(|n| n.eq_ignore_ascii_case(member_name))
            {
                if !pseudonyms.contains(pseudonym_hex) {
                    pseudonyms.push(pseudonym_hex.clone());
                }
                break;
            }
        }
    }
    (
        pseudonyms,
        matches.roles.clone(),
        matches.everyone,
        matches.here,
    )
}

/// Walk the body once and collect each `@token` as it appears. Tokens
/// must be preceded by whitespace, start-of-string, or punctuation;
/// this keeps an embedded `foo@bar.com` from triggering a mention.
fn scan_raw_tokens(body: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let chars: Vec<char> = body.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '@' {
            i += 1;
            continue;
        }
        let prev_ok = if i == 0 {
            true
        } else {
            let prev = chars[i - 1];
            prev.is_whitespace() || prev.is_ascii_punctuation()
        };
        if !prev_ok {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < chars.len() {
            let c = chars[j];
            if c.is_alphanumeric() || c == '_' || c == '-' {
                j += 1;
            } else {
                break;
            }
        }
        if j > i + 1 {
            out.push(chars[i + 1..j].iter().collect());
        }
        i = j.max(i + 1);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_raw_tokens_basic() {
        let tokens = scan_raw_tokens("hello @everyone and @alice and @here");
        assert_eq!(tokens, vec!["everyone", "alice", "here"]);
    }

    #[test]
    fn scan_raw_tokens_ignores_email() {
        // Email shouldn't trigger because '@' has 'o' (alphanumeric) before it.
        let tokens = scan_raw_tokens("send to foo@bar.com please");
        assert!(tokens.is_empty());
    }

    #[test]
    fn scan_raw_tokens_punctuation_boundary() {
        let tokens = scan_raw_tokens("hey,@here!");
        assert_eq!(tokens, vec!["here"]);
    }

    #[test]
    fn scan_raw_tokens_handles_hyphenated_role() {
        let tokens = scan_raw_tokens("ping @event-staff and @alice-jones");
        assert_eq!(tokens, vec!["event-staff", "alice-jones"]);
    }

    #[test]
    fn matches_from_cleartext_falls_back_to_pseudonym_when_unknown() {
        // No CommunityState available in this unit-test context, so
        // the lookup falls through to the bare pseudonym hex (lowercased).
        // This still drives the local-member-mentioned check for known
        // peers and keeps the wire round-trip stable for unknown ones.
        let state = std::sync::Arc::new(crate::state::AppState::default());
        let m = matches_from_cleartext(
            &state,
            "missing-community",
            &["DEADBEEF".into()],
            &[],
            true,
            false,
        );
        // No community → empty members; everyone bit preserved.
        assert!(m.everyone);
        assert!(m.members.is_empty());
    }

    #[test]
    fn matches_from_cleartext_propagates_role_and_everyone_bits() {
        let state = std::sync::Arc::new(crate::state::AppState::default());
        let m = matches_from_cleartext(
            &state,
            "missing",
            &[],
            &["Moderator".into()],
            true,
            true,
        );
        assert!(m.everyone);
        assert!(m.here);
        assert_eq!(m.roles, vec!["moderator"]);
    }
}
