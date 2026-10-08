//! Phase 19.h — pure mention parser + match-result struct.
//!
//! Ported from src-tauri/services/community/mentions.rs. Chiral split:
//! pure `@token` scanner + `MentionMatches` aggregate live here;
//! src-tauri retains the AppState-tied resolvers (role-mentionable
//! filter, member-profile name lookups, sender permission gating,
//! cleartext-wire ↔ DTO conversion).
//!
//! Architecture §28.5 — mention syntax:
//! - `@everyone` — pings every community member; requires `MENTION_EVERYONE`
//! - `@here` — pings every online member; requires `MENTION_EVERYONE`
//! - `@<role-name>` — pings role members; role must be `mentionable: true`
//! - `@<display-name>` — pings the specific member; always allowed

/// Result of parsing mentions out of a message body.
///
/// The src-tauri orchestrator fills the typed fields by walking the
/// raw `scan_raw_tokens` output against the merged governance state
/// (roles list + member display names).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MentionMatches {
    pub everyone: bool,
    pub here: bool,
    /// Lowercased role names referenced via `@role-name`.
    pub roles: Vec<String>,
    /// Lowercased display names referenced via `@display-name`.
    pub members: Vec<String>,
}

impl MentionMatches {
    /// Whether any mention class fired. Fast-skip helper for the
    /// permission-validation pass.
    #[must_use]
    pub fn is_any(&self) -> bool {
        self.everyone || self.here || !self.roles.is_empty() || !self.members.is_empty()
    }
}

/// Walk the body once and collect each `@token` as it appears. Tokens
/// must be preceded by whitespace, start-of-string, or punctuation;
/// this keeps an embedded `foo@bar.com` from triggering a mention.
///
/// Token charset: alphanumeric + `_` + `-`. Hyphenated role names
/// (e.g. `@event-staff`) and display names with underscores work.
#[must_use]
pub fn scan_raw_tokens(body: &str) -> Vec<String> {
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

/// Permission-bit helper — `(perms & required) == required` semantics,
/// inlined so callers don't need a separate dependency on the bitflags
/// crate. Used by the src-tauri permission gating to enforce
/// `MENTION_EVERYONE` etc.
#[must_use]
#[inline]
pub fn has_perm(perms: u64, required: u64) -> bool {
    perms & required == required
}

// ---------- 19.d-REDO: full mention pipeline ----------

use crate::deps::ChannelMessagingDeps;
use rekindle_types::permissions::MENTION_EVERYONE;

/// Parse the message body, resolve each `@token` against the merged
/// governance state, and return a typed `MentionMatches`. Mirrors
/// src-tauri parse_mentions verbatim.
///
/// Role tokens take precedence over display-name tokens so an admin
/// can't accidentally bypass-via-naming themselves "everyone".
pub fn parse_mentions<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    body: &str,
) -> MentionMatches {
    let raw = scan_raw_tokens(body);
    let roles = deps.community_roles(community_id);

    let mut matches = MentionMatches::default();
    for token in raw {
        match token.as_str() {
            "everyone" => matches.everyone = true,
            "here" => matches.here = true,
            other => {
                let role_match = roles
                    .iter()
                    .find(|r| r.name.eq_ignore_ascii_case(other) && r.mentionable);
                if role_match.is_some() {
                    let lowered = other.to_lowercase();
                    if !matches.roles.contains(&lowered) {
                        matches.roles.push(lowered);
                    }
                    continue;
                }
                let lowered = other.to_lowercase();
                if !matches.members.contains(&lowered) {
                    matches.members.push(lowered);
                }
            }
        }
    }
    matches
}

/// Reader-validates: drop mention classes the sender lacks permission
/// to use. Architecture §9.3 — sender's permission is computed once
/// over the merged governance state; @everyone/@here are stripped if
/// they lack `MENTION_EVERYONE`. Role mentions don't need extra
/// gating (parser already filtered to mentionable roles).
pub fn validate_sender_permissions<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    _sender_pseudonym_hex: &str,
    matches: &mut MentionMatches,
) {
    if !matches.is_any() {
        return;
    }
    let perms = deps.compute_my_permissions(community_id);
    if !has_perm(perms, MENTION_EVERYONE) {
        matches.everyone = false;
        matches.here = false;
    }
}

/// `true` when the local member is referenced by the parsed mentions.
/// Drives notification escalation (architecture §17.1).
pub fn local_member_is_mentioned<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    matches: &MentionMatches,
) -> bool {
    if matches.everyone || matches.here {
        return true;
    }
    if !matches.roles.is_empty() {
        let my_roles = deps.my_role_ids(community_id);
        let roles = deps.community_roles(community_id);
        let any_role = my_roles
            .iter()
            .filter_map(|id| roles.iter().find(|r| r.id == *id))
            .any(|role| matches.roles.iter().any(|m| m == &role.name.to_lowercase()));
        if any_role {
            return true;
        }
    }
    if matches.members.is_empty() {
        return false;
    }
    let Some(my_pseudonym) = deps.my_pseudonym_hex(community_id) else {
        return false;
    };
    let my_profile = deps.member_profile(community_id, &my_pseudonym);
    let Some(my_display) = my_profile.display_name.map(|s| s.to_lowercase()) else {
        return false;
    };
    // A member-token match wins only if it isn't already a role token
    // — roles took precedence in the parser.
    let roles = deps.community_roles(community_id);
    matches
        .members
        .iter()
        .any(|name| roles.iter().all(|r| r.name.to_lowercase() != *name) && my_display == *name)
}

/// Inverse of `resolve_to_wire`: rebuild a `MentionMatches` from the
/// cleartext envelope fields a receiver pulled off the wire. Member
/// pseudonyms are converted back to lowercase display names via
/// `list_member_profiles`; unknown pseudonyms become "raw-pseudonym"
/// entries so the local-member check still works for known peers.
pub fn matches_from_cleartext<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    mentioned_pseudonyms: &[String],
    mentioned_roles: &[String],
    mention_everyone: bool,
    mention_here: bool,
) -> MentionMatches {
    let profiles = deps.list_member_profiles(community_id);
    let mut matches = MentionMatches {
        everyone: mention_everyone,
        here: mention_here,
        roles: mentioned_roles.iter().map(|r| r.to_lowercase()).collect(),
        members: Vec::new(),
    };
    for pseudonym_hex in mentioned_pseudonyms {
        let display = profiles
            .get(pseudonym_hex)
            .and_then(|p| p.display_name.as_deref())
            .map_or_else(|| pseudonym_hex.to_lowercase(), str::to_lowercase);
        if !matches.members.contains(&display) {
            matches.members.push(display);
        }
    }
    matches
}

/// Resolve parsed mention matches into the wire-payload shape for the
/// cleartext envelope (architecture §28.5 line 3105-3120). Returns
/// `(mentioned_pseudonyms, mentioned_roles, mention_everyone, mention_here)`.
pub fn resolve_to_wire<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    matches: &MentionMatches,
) -> (Vec<String>, Vec<String>, bool, bool) {
    let profiles = deps.list_member_profiles(community_id);
    let mut pseudonyms: Vec<String> = Vec::new();
    for member_name in &matches.members {
        for (pseudonym_hex, profile) in &profiles {
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

/// One-shot resolver used by senders: parse + permission-gate +
/// resolve to wire, then compute the mention-flag bits. Returns
/// `(mentioned_pseudonyms, mentioned_roles, mention_flag_bits)`.
///
/// Centralizes the send-path mention resolution that both
/// channel_messages::send_message and threads::send_thread_message
/// need so the rules stay consistent across both paths.
pub fn resolve_outbound_mentions<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    sender_pseudonym_hex: &str,
    body: &str,
) -> (Vec<String>, Vec<String>, u32) {
    use rekindle_types::channel::flags::{MENTION_EVERYONE as F_EVERYONE, MENTION_HERE as F_HERE};
    let mut matches = parse_mentions(deps, community_id, body);
    validate_sender_permissions(deps, community_id, sender_pseudonym_hex, &mut matches);
    let (pseudonyms, roles, everyone, here) = resolve_to_wire(deps, community_id, &matches);
    let mut flag_bits = 0u32;
    if everyone {
        flag_bits |= F_EVERYONE;
    }
    if here {
        flag_bits |= F_HERE;
    }
    (pseudonyms, roles, flag_bits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_default_is_empty_and_not_any() {
        let m = MentionMatches::default();
        assert!(!m.is_any());
        assert!(!m.everyone);
        assert!(!m.here);
        assert!(m.roles.is_empty());
        assert!(m.members.is_empty());
    }

    #[test]
    fn matches_is_any_each_branch() {
        for m in [
            MentionMatches {
                everyone: true,
                ..Default::default()
            },
            MentionMatches {
                here: true,
                ..Default::default()
            },
            MentionMatches {
                roles: vec!["mod".into()],
                ..Default::default()
            },
            MentionMatches {
                members: vec!["alice".into()],
                ..Default::default()
            },
        ] {
            assert!(m.is_any());
        }
    }

    #[test]
    fn scan_basic_tokens() {
        let tokens = scan_raw_tokens("hello @everyone and @alice and @here");
        assert_eq!(tokens, vec!["everyone", "alice", "here"]);
    }

    #[test]
    fn scan_ignores_email_address() {
        let tokens = scan_raw_tokens("send to foo@bar.com please");
        assert!(tokens.is_empty());
    }

    #[test]
    fn scan_punctuation_boundary() {
        let tokens = scan_raw_tokens("hey,@here!");
        assert_eq!(tokens, vec!["here"]);
    }

    #[test]
    fn scan_hyphenated_token() {
        let tokens = scan_raw_tokens("ping @event-staff and @alice-jones");
        assert_eq!(tokens, vec!["event-staff", "alice-jones"]);
    }

    #[test]
    fn scan_underscore_token() {
        let tokens = scan_raw_tokens("@my_role hi");
        assert_eq!(tokens, vec!["my_role"]);
    }

    #[test]
    fn scan_start_of_string_token() {
        let tokens = scan_raw_tokens("@start mention");
        assert_eq!(tokens, vec!["start"]);
    }

    #[test]
    fn scan_double_at_skips() {
        // @@ has an alphanumeric requirement after the inner @ — the
        // first @ has no token; the second @ is preceded by punctuation
        // ('@' is ascii_punctuation), so it does trigger.
        let tokens = scan_raw_tokens("@@hello");
        assert_eq!(tokens, vec!["hello"]);
    }

    #[test]
    fn scan_empty_token_skipped() {
        let tokens = scan_raw_tokens("@ @  @");
        assert!(tokens.is_empty());
    }

    #[test]
    fn has_perm_matches_bitflag_semantics() {
        const A: u64 = 0b0001;
        const B: u64 = 0b0010;
        const C: u64 = 0b0100;
        assert!(has_perm(A | B, A));
        assert!(has_perm(A | B, B));
        assert!(has_perm(A | B | C, A | B));
        assert!(!has_perm(A, B));
        assert!(!has_perm(A | C, A | B));
        assert!(has_perm(u64::MAX, A | B | C));
    }
}
