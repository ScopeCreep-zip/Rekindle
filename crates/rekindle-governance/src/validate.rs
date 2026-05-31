//! Reader-validates: check if a writer had permission for a governance entry.
//!
//! Every peer independently validates incoming governance entries against the
//! CRDT-merged permission state. Invalid entries are silently excluded from
//! the materialized view.
//!
//! See architecture doc §9.3 for the enforcement model.

use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::PseudonymKey;
use rekindle_types::permissions::*;

use crate::permissions::compute_permissions;
use crate::state::GovernanceState;

const MAX_STATIC_EMOJIS: usize = 50;
const MAX_ANIMATED_EMOJIS: usize = 50;
const MAX_STICKERS: usize = 30;
const MAX_SOUNDBOARD_SOUNDS: usize = 48;
const MAX_AUTOMOD_KEYWORDS: usize = 1000;
const MAX_AUTOMOD_REGEX_PATTERNS: usize = 10;
/// Architecture §20.4 + §26 W26 — adversarial admins can ship pathological
/// regex patterns whose compiled NFA fits in the default 10 MiB ceiling
/// but still takes seconds to assemble. Cap individual pattern source
/// length so a malicious `AutoModRule` can't DoS every other peer's
/// `compile_rules` pass on first message receive.
const MAX_AUTOMOD_REGEX_PATTERN_LEN: usize = 512;
const MAX_AUTOMOD_KEYWORD_LEN: usize = 256;
/// Tighten the regex compiler's NFA budget below the 10 MiB default.
/// Architecture §20.4 — moderation rules are advisory and short by
/// design; 256 KiB compiled NFA is plenty for the stated keyword/regex
/// patterns and rejects anything pathological well before it becomes a
/// receive-side DoS.
const AUTOMOD_REGEX_SIZE_LIMIT: usize = 256 * 1024;

// Architecture §19.1 (lines 2520-2531) — onboarding shape limits.
const MAX_ONBOARDING_QUESTIONS: usize = 5;
const MAX_ONBOARDING_OPTIONS_PER_QUESTION: usize = 10;
const MAX_ONBOARDING_GUIDE_STEPS: usize = 10;
const MAX_ONBOARDING_WELCOME_CHARS: usize = 500;
const MAX_ONBOARDING_QUESTION_TITLE_CHARS: usize = 100;
// Architecture §19.3 — welcome screen featured channels max 5.
const MAX_WELCOME_SCREEN_CHANNELS: usize = 5;

/// Check if `writer` had permission to write `entry` given the current `state`.
///
/// Returns `true` if the entry should be included in the merged state.
/// Returns `false` if it should be silently excluded.
///
/// # Note on circular dependency
/// Role assignments determine who can make governance changes, but role
/// assignments ARE governance changes. The resolution: entries are processed
/// in Lamport order, and at each entry, the current accumulated permission
/// state is used. Genesis entries (first in order) bypass all checks.
pub fn validate_write(
    writer: &PseudonymKey,
    entry: &GovernanceEntry,
    state: &GovernanceState,
) -> bool {
    // Creator always passes validation
    if state.creator.as_ref() == Some(writer) {
        return true;
    }

    // Banned members can't write valid governance entries
    if state.bans.contains(writer) {
        return false;
    }

    let perms = compute_permissions(writer, None, state, 0);

    match entry {
        GovernanceEntry::ChannelCreated { .. }
        | GovernanceEntry::ChannelArchived { .. }
        | GovernanceEntry::ChannelUpdated { .. } => has(perms, MANAGE_CHANNELS),

        // Architecture §9.3 line 1946 — a writer can only define/edit a
        // role whose position is strictly less than their own max
        // position. Without this, a mid-rank admin could mint a higher-
        // ranked role and grant it to themselves to climb the hierarchy.
        GovernanceEntry::RoleDefinition { position, .. } => {
            has(perms, MANAGE_ROLES) && *position < state.member_max_position(writer)
        }

        GovernanceEntry::RoleAssignment {
            target, role_id, ..
        } => {
            // Self-assignable roles bypass MANAGE_ROLES (existing behavior).
            if can_self_assign_role(writer, target, role_id, state) {
                return true;
            }
            // Otherwise: MANAGE_ROLES required AND the role being granted
            // must rank strictly below the writer (architecture §9.3).
            if !has(perms, MANAGE_ROLES) {
                return false;
            }
            let role_position = state.roles.get(role_id).map(|r| r.position).unwrap_or(0);
            role_position < state.member_max_position(writer)
        }

        GovernanceEntry::RoleUnassignment {
            target, role_id, ..
        } => {
            // M10.2 — OWNER protection: the creator's roles can only be
            // unassigned by the creator themselves. Defense-in-depth on
            // top of the rank check; even if an attacker somehow climbs
            // above the creator's rank, this still blocks them.
            if state.creator.as_ref() == Some(target) && writer != target {
                return false;
            }
            // Self-unassign always allowed (you can step down).
            if writer == target {
                return true;
            }
            // Self-assignable role being yanked from the holder by the
            // holder is the self-unassign case above; another peer
            // pulling it falls through to the standard hierarchy check.
            if can_self_assign_role(writer, target, role_id, state) {
                return true;
            }
            if !has(perms, MANAGE_ROLES) {
                return false;
            }
            // Architecture §9.3 — target's max position must be strictly
            // less than writer's. Equal-rank or higher-rank target is
            // immune.
            state.member_max_position(target) < state.member_max_position(writer)
        }

        GovernanceEntry::BanEntry { target, .. } => {
            if !has(perms, BAN_MEMBERS) {
                return false;
            }
            // M10.2 — creator cannot be banned by anyone.
            if state.creator.as_ref() == Some(target) {
                return false;
            }
            // Architecture §9.3 — rank-strict gating.
            state.member_max_position(target) < state.member_max_position(writer)
        }

        // Unbans don't carry a hierarchy concern: banned members hold no
        // active roles inside the community (they're outside it). Anyone
        // with BAN_MEMBERS can lift a ban.
        GovernanceEntry::UnbanEntry { .. } => has(perms, BAN_MEMBERS),

        GovernanceEntry::TimeoutEntry { target, .. } => {
            if !has(perms, TIMEOUT_MEMBERS) {
                return false;
            }
            // M10.2 — creator cannot be timed out by anyone.
            if state.creator.as_ref() == Some(target) {
                return false;
            }
            // Architecture §9.3 — rank-strict gating.
            state.member_max_position(target) < state.member_max_position(writer)
        }

        GovernanceEntry::RemoveTimeoutEntry { .. } => has(perms, TIMEOUT_MEMBERS),

        GovernanceEntry::CommunityMeta { .. } => has(perms, MANAGE_COMMUNITY),

        // Notification default (architecture §17.1 tier 1) is admin-only;
        // any member's local override still trumps it via the resolver.
        GovernanceEntry::CommunityNotificationDefault { level, .. } => {
            matches!(level.as_str(), "all" | "mentions" | "nothing") && has(perms, MANAGE_COMMUNITY)
        }

        // MEK generation bumps use Max-Register (highest generation wins).
        // Rotator authority is verified by checking trigger_departed + cascade_skipped
        // against the deterministic rotator selection algorithm. However, since the
        // merge engine already enforces Max-Register (only highest generation survives),
        // a rogue bump to generation N is superseded by the legitimate bump to N+1.
        // Full rotator verification requires cross-referencing presence timestamps
        // (for cascade_skipped validation), which is done at the sync layer, not here.
        // At the governance CRDT layer, we enforce: writer is not banned (checked above).
        GovernanceEntry::MEKGenerationBump { .. } => true,

        GovernanceEntry::CategoryCreated { .. } | GovernanceEntry::CategoryArchived { .. } => {
            has(perms, MANAGE_CHANNELS)
        }

        GovernanceEntry::PermissionOverwrite { .. } => {
            has(perms, MANAGE_CHANNELS) || has(perms, MANAGE_ROLES)
        }

        GovernanceEntry::ThreadCreated {
            parent_channel_id,
            thread_type,
            invited,
            forum_tag,
            ..
        } => validate_thread_create(
            writer,
            *parent_channel_id,
            thread_type,
            invited,
            forum_tag.as_deref(),
            state,
        ),

        GovernanceEntry::ThreadArchived { .. } => has(perms, MANAGE_THREADS),

        GovernanceEntry::EventCreated { .. } => has(perms, CREATE_EVENTS),

        GovernanceEntry::EventArchived { .. } => has(perms, MANAGE_EVENTS),

        GovernanceEntry::ExpressionAdded {
            kind,
            animated,
            sound_meta,
            ..
        } => {
            (has(perms, MANAGE_EXPRESSIONS) || has(perms, CREATE_EXPRESSIONS))
                && expression_within_limits(kind, *animated, state)
                && soundboard_meta_valid(kind, sound_meta.as_ref())
        }

        GovernanceEntry::ExpressionRemoved { .. } => has(perms, MANAGE_EXPRESSIONS),

        GovernanceEntry::OnboardingConfig {
            mode,
            questions,
            welcome_message,
            guide_steps,
            ..
        } => {
            has(perms, MANAGE_COMMUNITY)
                && onboarding_within_limits(
                    mode,
                    questions,
                    welcome_message.as_deref(),
                    guide_steps,
                )
        }

        GovernanceEntry::WelcomeScreen { channels, .. } => {
            has(perms, MANAGE_COMMUNITY) && channels.len() <= MAX_WELCOME_SCREEN_CHANNELS
        }

        GovernanceEntry::AdminDelete { .. } => has(perms, MANAGE_MESSAGES),

        // Lost Cargo: pinning a file requires MANAGE_COMMUNITY (admin-only)
        // per architecture §28.9 line 3283.
        GovernanceEntry::AttachmentPinned { .. } => has(perms, MANAGE_COMMUNITY),

        // Community-wide policy (notification default + raid thresholds):
        // architecture §17.1 + §20.6 — admin-only.
        GovernanceEntry::CommunityPolicy { .. } => has(perms, MANAGE_COMMUNITY),

        // Segment expansion requires admin-level access
        GovernanceEntry::SegmentAdded { .. } => has(perms, MANAGE_COMMUNITY),

        // Plate Gate lazy channel records (architecture §15.4): any member
        // with SEND_MESSAGES can announce a new channel-segment record —
        // they're the one creating the SMPL record and writing the first
        // message into it. Reader-validates: peers reject ChannelSegmentLinked
        // entries from members without channel write access, and reject
        // entries that name a channel that doesn't exist in governance state.
        GovernanceEntry::ChannelSegmentLinked { channel_id, .. } => {
            has(perms, SEND_MESSAGES) && state.channels.contains_key(channel_id)
        }

        GovernanceEntry::AutoModRule {
            rule_id,
            enabled,
            trigger_json,
            action,
            ..
        } => {
            has(perms, MANAGE_COMMUNITY)
                && validate_automod_rule(rule_id, *enabled, trigger_json, action, state)
        }

        GovernanceEntry::RoleArchived { .. } => has(perms, MANAGE_ROLES),

        GovernanceEntry::CategoryUpdated { .. } => has(perms, MANAGE_CHANNELS),

        // M10.3 — invite minting is reader-validated against three caps:
        //   1. CREATE_INVITES permission (existing).
        //   2. `max_uses` ≤ MAX_USES_PER_INVITE — bound per-invite reuse.
        //   3. Active invites by this writer < policy.max_joins_per_interval —
        //      bound the number of simultaneous join-points opened.
        // A rogue admin who exceeds either cap has their entry silently
        // excluded from the merged state on every honest peer; the joiner
        // who tries to use that invite finds nothing in `state.invites` and
        // reports "invalid invite" via the existing flow.
        GovernanceEntry::InviteCreated { max_uses, .. } => {
            has(perms, CREATE_INVITES)
                && crate::invite_quota::check_max_uses_cap(*max_uses)
                && crate::invite_quota::check_active_invites_cap(state, writer)
        }

        GovernanceEntry::InviteRevoked { .. } => has(perms, MANAGE_COMMUNITY),
    }
}

/// Check if a permission bitmask includes the required permission.
/// ADMINISTRATOR always passes.
fn has(perms: u64, required: u64) -> bool {
    (perms & ADMINISTRATOR != 0) || (perms & required == required)
}

fn can_self_assign_role(
    writer: &PseudonymKey,
    target: &PseudonymKey,
    role_id: &rekindle_types::id::RoleId,
    state: &GovernanceState,
) -> bool {
    writer == target
        && state
            .roles
            .get(role_id)
            .map(|role| role.self_assignable)
            .unwrap_or(false)
}

fn expression_within_limits(kind: &str, animated: bool, state: &GovernanceState) -> bool {
    let static_emoji_count = state
        .expressions
        .values()
        .filter(|expr| expr.kind == "emoji" && !expr.animated)
        .count();
    let animated_emoji_count = state
        .expressions
        .values()
        .filter(|expr| expr.kind == "emoji" && expr.animated)
        .count();
    let sticker_count = state
        .expressions
        .values()
        .filter(|expr| expr.kind == "sticker")
        .count();
    let soundboard_count = state
        .expressions
        .values()
        .filter(|expr| expr.kind == "soundboard")
        .count();

    match kind {
        "emoji" if animated => animated_emoji_count < MAX_ANIMATED_EMOJIS,
        "emoji" => static_emoji_count < MAX_STATIC_EMOJIS,
        "sticker" => sticker_count < MAX_STICKERS,
        "soundboard" => soundboard_count < MAX_SOUNDBOARD_SOUNDS,
        _ => false,
    }
}

/// Architecture §19.1 line 2520-2531 — onboarding shape caps. We
/// enforce these at the validate layer (in addition to the Tauri
/// command pre-flight) so a tampered governance entry that exceeds
/// limits is dropped on the floor by every honest peer.
fn onboarding_within_limits(
    mode: &str,
    questions: &[rekindle_types::governance::OnboardingQuestion],
    welcome_message: Option<&str>,
    guide_steps: &[rekindle_types::governance::GuideStep],
) -> bool {
    if !matches!(mode, "default" | "guided" | "gated") {
        return false;
    }
    if questions.len() > MAX_ONBOARDING_QUESTIONS {
        return false;
    }
    if guide_steps.len() > MAX_ONBOARDING_GUIDE_STEPS {
        return false;
    }
    if let Some(text) = welcome_message {
        if text.chars().count() > MAX_ONBOARDING_WELCOME_CHARS {
            return false;
        }
    }
    for question in questions {
        if question.title.chars().count() > MAX_ONBOARDING_QUESTION_TITLE_CHARS {
            return false;
        }
        if question.options.len() > MAX_ONBOARDING_OPTIONS_PER_QUESTION {
            return false;
        }
    }
    true
}

/// Architecture §18.3 — soundboard entries must carry valid duration /
/// volume / emoji metadata. Emoji and sticker entries must NOT carry it
/// (any peer that smuggles `sound_meta` onto a non-soundboard entry has
/// produced an invalid wire object — drop it on the floor).
fn soundboard_meta_valid(
    kind: &str,
    sound_meta: Option<&rekindle_types::expression::SoundboardMeta>,
) -> bool {
    use rekindle_types::expression::SoundboardMeta;
    match (kind, sound_meta) {
        ("soundboard", Some(meta)) => {
            SoundboardMeta::validate_duration(meta.duration_seconds).is_ok()
                && SoundboardMeta::validate_volume(meta.volume).is_ok()
                && SoundboardMeta::validate_emoji(meta.emoji.as_deref()).is_ok()
        }
        // Legacy soundboard entries (pre-meta) are still acceptable —
        // peers display them with default volume 1.0 and skip duration
        // enforcement until the entry is rewritten by the uploader.
        ("soundboard", None) => true,
        // Emoji / sticker entries must not carry sound_meta.
        (_, None) => true,
        (_, Some(_)) => false,
    }
}

fn validate_automod_rule(
    rule_id: &[u8; 16],
    enabled: bool,
    trigger_json: &str,
    action: &str,
    state: &GovernanceState,
) -> bool {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct TriggerConfig {
        #[serde(default)]
        keywords: Vec<String>,
        #[serde(default)]
        regex_patterns: Vec<String>,
    }

    if !matches!(
        action,
        "block_locally" | "blur_content" | "alert_moderators"
    ) {
        return false;
    }

    let Ok(trigger) = serde_json::from_str::<TriggerConfig>(trigger_json) else {
        return false;
    };
    // Length caps before compile so a 1 MB regex source can't even
    // reach the regex parser. Mirrors the spec's "small filter rules"
    // intent (§20.4) and prevents per-rule receive-side DoS.
    if trigger
        .keywords
        .iter()
        .any(|k| k.len() > MAX_AUTOMOD_KEYWORD_LEN)
    {
        return false;
    }
    if trigger
        .regex_patterns
        .iter()
        .any(|pattern| pattern.len() > MAX_AUTOMOD_REGEX_PATTERN_LEN)
    {
        return false;
    }
    if trigger.regex_patterns.iter().any(|pattern| {
        regex::RegexBuilder::new(pattern)
            .size_limit(AUTOMOD_REGEX_SIZE_LIMIT)
            .build()
            .is_err()
    }) {
        return false;
    }

    let current_totals = state
        .automod_rules
        .iter()
        .filter(|(existing_id, rule)| *existing_id != rule_id && rule.enabled)
        .filter_map(|(_, rule)| serde_json::from_str::<TriggerConfig>(&rule.trigger_json).ok())
        .fold((0usize, 0usize), |(keywords, regexes), trigger| {
            (
                keywords + trigger.keywords.len(),
                regexes + trigger.regex_patterns.len(),
            )
        });

    let next_keywords = current_totals.0 + if enabled { trigger.keywords.len() } else { 0 };
    let next_regexes = current_totals.1
        + if enabled {
            trigger.regex_patterns.len()
        } else {
            0
        };
    next_keywords <= MAX_AUTOMOD_KEYWORDS && next_regexes <= MAX_AUTOMOD_REGEX_PATTERNS
}

fn validate_thread_create(
    writer: &PseudonymKey,
    parent_channel_id: rekindle_types::id::ChannelId,
    thread_type: &str,
    invited: &[PseudonymKey],
    forum_tag: Option<&str>,
    state: &GovernanceState,
) -> bool {
    let channel_perms = compute_permissions(
        writer,
        Some(&parent_channel_id),
        state,
        rekindle_utils::time::timestamp_secs(),
    );

    match thread_type {
        "public" => has(channel_perms, SEND_MESSAGES),
        "private" => {
            has(channel_perms, CREATE_PRIVATE_THREADS)
                && invited.iter().all(|invitee| invitee != writer)
        }
        "announcement" => has(channel_perms, MANAGE_THREADS),
        "forum_post" => {
            state
                .channels
                .get(&parent_channel_id)
                .is_some_and(|channel| {
                    channel.channel_type == "forum"
                        && forum_tag.is_none_or(|tag| {
                            channel
                                .forum_tags
                                .as_ref()
                                .is_some_and(|tags| tags.iter().any(|candidate| candidate == tag))
                        })
                })
                && has(channel_perms, SEND_MESSAGES)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{GovernanceState, RoleState};
    use std::collections::{HashMap, HashSet};

    fn pseudo(b: u8) -> PseudonymKey {
        PseudonymKey([b; 32])
    }

    fn rid(b: u8) -> rekindle_types::id::RoleId {
        rekindle_types::id::RoleId([b; 16])
    }

    fn state_with_creator_and_roles() -> GovernanceState {
        let mut roles = HashMap::new();
        roles.insert(
            rid(0),
            RoleState {
                name: "everyone".into(),
                permissions: VIEW_CHANNELS | SEND_MESSAGES | READ_HISTORY,
                position: 0,
                color: 0,
                hoist: false,
                mentionable: false,
                self_assignable: false,
                exclusion_group: None,
                lamport: 1,
            },
        );
        roles.insert(
            rid(1),
            RoleState {
                name: "admin".into(),
                permissions: ADMINISTRATOR,
                position: 1,
                color: 0,
                hoist: false,
                mentionable: false,
                self_assignable: false,
                exclusion_group: None,
                lamport: 2,
            },
        );

        let mut assignments = HashMap::new();
        let mut admin_roles = HashSet::new();
        admin_roles.insert(rid(1));
        assignments.insert(pseudo(5), admin_roles); // pseudo(5) is admin

        GovernanceState {
            creator: Some(pseudo(1)),
            roles,
            role_assignments: assignments,
            ..Default::default()
        }
    }

    #[test]
    fn creator_always_validates() {
        let state = state_with_creator_and_roles();
        let entry = GovernanceEntry::ChannelCreated {
            channel_id: rekindle_types::id::ChannelId([0; 16]),
            name: "test".into(),
            channel_type: "text".into(),
            record_key: "k".into(),
            category_id: None,
            position: 0,
            parent_voice_channel_id: None,
            lamport: 10,
        };
        assert!(validate_write(&pseudo(1), &entry, &state));
    }

    #[test]
    fn regular_member_cannot_manage_channels() {
        let state = state_with_creator_and_roles();
        let entry = GovernanceEntry::ChannelCreated {
            channel_id: rekindle_types::id::ChannelId([0; 16]),
            name: "test".into(),
            channel_type: "text".into(),
            record_key: "k".into(),
            category_id: None,
            position: 0,
            parent_voice_channel_id: None,
            lamport: 10,
        };
        // pseudo(99) has only @everyone perms — no MANAGE_CHANNELS
        assert!(!validate_write(&pseudo(99), &entry, &state));
    }

    #[test]
    fn admin_can_manage_channels() {
        let state = state_with_creator_and_roles();
        let entry = GovernanceEntry::ChannelCreated {
            channel_id: rekindle_types::id::ChannelId([0; 16]),
            name: "test".into(),
            channel_type: "text".into(),
            record_key: "k".into(),
            category_id: None,
            position: 0,
            parent_voice_channel_id: None,
            lamport: 10,
        };
        // pseudo(5) is admin
        assert!(validate_write(&pseudo(5), &entry, &state));
    }

    #[test]
    fn mek_bump_accepted_at_governance_layer() {
        let state = state_with_creator_and_roles();
        let entry = GovernanceEntry::MEKGenerationBump {
            generation: 2,
            trigger_departed: pseudo(10),
            cascade_skipped: vec![],
            lamport: 10,
        };
        // At governance CRDT layer, MEK bumps are accepted (Max-Register).
        // Full rotator authority is verified at the sync layer.
        assert!(validate_write(&pseudo(99), &entry, &state));
    }

    #[test]
    fn banned_member_cannot_write() {
        let mut state = state_with_creator_and_roles();
        state.bans.insert(pseudo(50));

        let entry = GovernanceEntry::MEKGenerationBump {
            generation: 2,
            trigger_departed: pseudo(10),
            cascade_skipped: vec![],
            lamport: 10,
        };
        assert!(!validate_write(&pseudo(50), &entry, &state));
    }

    #[test]
    fn regular_member_can_create_thread() {
        let state = state_with_creator_and_roles();
        let entry = GovernanceEntry::ThreadCreated {
            thread_id: rekindle_types::id::ThreadId([0; 16]),
            parent_channel_id: rekindle_types::id::ChannelId([0; 16]),
            name: "discussion".into(),
            thread_type: "public".into(),
            record_key: None,
            invited: Vec::new(),
            forum_tag: None,
            auto_archive_seconds: 86_400,
            lamport: 10,
        };
        // @everyone has SEND_MESSAGES
        assert!(validate_write(&pseudo(99), &entry, &state));
    }

    #[test]
    fn self_assignable_role_allows_self_write() {
        let mut state = state_with_creator_and_roles();
        let self_role_id = rid(9);
        state.roles.insert(
            self_role_id,
            RoleState {
                name: "self".into(),
                permissions: 0,
                position: 0,
                color: 0,
                hoist: false,
                mentionable: false,
                self_assignable: true,
                exclusion_group: None,
                lamport: 3,
            },
        );

        let entry = GovernanceEntry::RoleAssignment {
            target: pseudo(99),
            role_id: self_role_id,
            lamport: 10,
        };
        assert!(validate_write(&pseudo(99), &entry, &state));
    }

    #[test]
    fn expression_limit_rejects_extra_static_emoji() {
        let mut state = state_with_creator_and_roles();
        for index in 0..50_u8 {
            state.expressions.insert(
                [index; 16],
                crate::state::ExpressionState {
                    name: format!("emoji-{index}"),
                    kind: "emoji".into(),
                    content_hash: format!("hash-{index}"),
                    attachment: None,
                    animated: false,
                    tags: vec![],
                    sound_meta: None,
                    creator_pseudonym: None,
                    created_at: None,
                    available_to_peers: true,
                    lamport: u64::from(index),
                },
            );
        }

        let entry = GovernanceEntry::ExpressionAdded {
            expression_id: [99_u8; 16],
            name: "overflow".into(),
            kind: "emoji".into(),
            content_hash: "hash-overflow".into(),
            attachment: None,
            animated: false,
            tags: vec![],
            sound_meta: None,
            creator_pseudonym: None,
            created_at: None,
            available_to_peers: Some(true),
            lamport: 50,
        };
        assert!(!validate_write(&pseudo(5), &entry, &state));
    }

    #[test]
    fn expression_limit_rejects_extra_animated_emoji() {
        // Architecture §18.1 — separate 50-cap for animated; the
        // static count + animated count are independent, so a server
        // at the static limit can still accept animated uploads until
        // the animated bucket also fills.
        let mut state = state_with_creator_and_roles();
        for index in 0..50_u8 {
            state.expressions.insert(
                [index; 16],
                crate::state::ExpressionState {
                    name: format!("anim-{index}"),
                    kind: "emoji".into(),
                    content_hash: format!("anim-hash-{index}"),
                    attachment: None,
                    animated: true,
                    tags: vec![],
                    sound_meta: None,
                    creator_pseudonym: None,
                    created_at: None,
                    available_to_peers: true,
                    lamport: u64::from(index),
                },
            );
        }

        let entry = GovernanceEntry::ExpressionAdded {
            expression_id: [200_u8; 16],
            name: "anim-overflow".into(),
            kind: "emoji".into(),
            content_hash: "anim-hash-overflow".into(),
            attachment: None,
            animated: true,
            tags: vec![],
            sound_meta: None,
            creator_pseudonym: None,
            created_at: None,
            available_to_peers: Some(true),
            lamport: 100,
        };
        assert!(
            !validate_write(&pseudo(5), &entry, &state),
            "51st animated emoji must be rejected"
        );

        // Sanity: a static-emoji upload should still succeed at the
        // animated cap because the buckets are independent.
        let static_entry = GovernanceEntry::ExpressionAdded {
            expression_id: [201_u8; 16],
            name: "still-fine".into(),
            kind: "emoji".into(),
            content_hash: "still-fine-hash".into(),
            attachment: None,
            animated: false,
            tags: vec![],
            sound_meta: None,
            creator_pseudonym: None,
            created_at: None,
            available_to_peers: Some(true),
            lamport: 101,
        };
        assert!(
            validate_write(&pseudo(5), &static_entry, &state),
            "static slot still has room when animated bucket is full"
        );
    }

    #[test]
    fn soundboard_entry_with_valid_meta_accepted() {
        use rekindle_types::expression::SoundboardMeta;
        let state = state_with_creator_and_roles();
        let entry = GovernanceEntry::ExpressionAdded {
            expression_id: [10_u8; 16],
            name: "horn".into(),
            kind: "soundboard".into(),
            content_hash: "h".into(),
            attachment: None,
            animated: false,
            tags: vec![],
            sound_meta: Some(SoundboardMeta {
                duration_seconds: 2.5,
                volume: 0.8,
                emoji: Some("📯".into()),
            }),
            creator_pseudonym: None,
            created_at: None,
            available_to_peers: Some(true),
            lamport: 5,
        };
        assert!(validate_write(&pseudo(5), &entry, &state));
    }

    #[test]
    fn soundboard_entry_rejects_overlong_duration() {
        use rekindle_types::expression::SoundboardMeta;
        let state = state_with_creator_and_roles();
        let entry = GovernanceEntry::ExpressionAdded {
            expression_id: [11_u8; 16],
            name: "ten_sec".into(),
            kind: "soundboard".into(),
            content_hash: "h".into(),
            attachment: None,
            animated: false,
            tags: vec![],
            sound_meta: Some(SoundboardMeta {
                duration_seconds: 10.0,
                volume: 1.0,
                emoji: None,
            }),
            creator_pseudonym: None,
            created_at: None,
            available_to_peers: Some(true),
            lamport: 5,
        };
        assert!(!validate_write(&pseudo(5), &entry, &state));
    }

    #[test]
    fn soundboard_entry_rejects_volume_outside_range() {
        use rekindle_types::expression::SoundboardMeta;
        let state = state_with_creator_and_roles();
        let entry = GovernanceEntry::ExpressionAdded {
            expression_id: [12_u8; 16],
            name: "loud".into(),
            kind: "soundboard".into(),
            content_hash: "h".into(),
            attachment: None,
            animated: false,
            tags: vec![],
            sound_meta: Some(SoundboardMeta {
                duration_seconds: 1.0,
                volume: 1.5,
                emoji: None,
            }),
            creator_pseudonym: None,
            created_at: None,
            available_to_peers: Some(true),
            lamport: 5,
        };
        assert!(!validate_write(&pseudo(5), &entry, &state));
    }

    #[test]
    fn emoji_entry_rejects_smuggled_sound_meta() {
        use rekindle_types::expression::SoundboardMeta;
        let state = state_with_creator_and_roles();
        let entry = GovernanceEntry::ExpressionAdded {
            expression_id: [13_u8; 16],
            name: "weird".into(),
            kind: "emoji".into(),
            content_hash: "h".into(),
            attachment: None,
            animated: false,
            tags: vec![],
            sound_meta: Some(SoundboardMeta {
                duration_seconds: 1.0,
                volume: 1.0,
                emoji: None,
            }),
            creator_pseudonym: None,
            created_at: None,
            available_to_peers: Some(true),
            lamport: 5,
        };
        assert!(!validate_write(&pseudo(5), &entry, &state));
    }

    #[test]
    fn onboarding_rejects_too_many_questions() {
        use rekindle_types::governance::{OnboardingOption, OnboardingQuestion};
        let state = state_with_creator_and_roles();
        let mut questions = Vec::new();
        for i in 0..6_u8 {
            questions.push(OnboardingQuestion {
                question_id: format!("q{i}"),
                title: "x".into(),
                description: None,
                required: false,
                single_select: true,
                options: vec![OnboardingOption {
                    option_id: "o".into(),
                    title: "o".into(),
                    description: None,
                    emoji: None,
                    roles_to_assign: vec![],
                    channels_to_show: vec![],
                }],
            });
        }
        let entry = GovernanceEntry::OnboardingConfig {
            enabled: true,
            mode: "guided".into(),
            default_channels: vec![],
            questions,
            welcome_message: None,
            guide_steps: vec![],
            lamport: 10,
        };
        assert!(!validate_write(&pseudo(5), &entry, &state));
    }

    #[test]
    fn onboarding_rejects_overlong_welcome_message() {
        let state = state_with_creator_and_roles();
        let entry = GovernanceEntry::OnboardingConfig {
            enabled: true,
            mode: "default".into(),
            default_channels: vec![],
            questions: vec![],
            welcome_message: Some("a".repeat(501)),
            guide_steps: vec![],
            lamport: 10,
        };
        assert!(!validate_write(&pseudo(5), &entry, &state));
    }

    #[test]
    fn onboarding_rejects_unknown_mode() {
        let state = state_with_creator_and_roles();
        let entry = GovernanceEntry::OnboardingConfig {
            enabled: true,
            mode: "bogus".into(),
            default_channels: vec![],
            questions: vec![],
            welcome_message: None,
            guide_steps: vec![],
            lamport: 10,
        };
        assert!(!validate_write(&pseudo(5), &entry, &state));
    }

    // ──────────────────────────────────────────────────────────────────
    // M10.1 / M10.2 — role hierarchy + OWNER protection (arch §9.3 line 1946)
    // ──────────────────────────────────────────────────────────────────

    /// Build a state with a creator, three roles (everyone=0, mod=2, admin=5),
    /// and three test members:
    /// - pseudo(1): creator (no role assignments needed; creator-bypass kicks in)
    /// - pseudo(5): admin (position 5)
    /// - pseudo(6): mod (position 2)
    /// - pseudo(7): another admin (position 5)
    /// - pseudo(99): regular member (no roles → position 0)
    fn state_with_three_tier_hierarchy() -> GovernanceState {
        let mut roles = HashMap::new();
        roles.insert(
            rid(0),
            RoleState {
                name: "everyone".into(),
                permissions: VIEW_CHANNELS | SEND_MESSAGES | READ_HISTORY,
                position: 0,
                color: 0,
                hoist: false,
                mentionable: false,
                self_assignable: false,
                exclusion_group: None,
                lamport: 1,
            },
        );
        roles.insert(
            rid(2),
            RoleState {
                name: "mod".into(),
                permissions: BAN_MEMBERS | TIMEOUT_MEMBERS | MANAGE_ROLES,
                position: 2,
                color: 0,
                hoist: false,
                mentionable: false,
                self_assignable: false,
                exclusion_group: None,
                lamport: 2,
            },
        );
        roles.insert(
            rid(5),
            RoleState {
                name: "admin".into(),
                permissions: ADMINISTRATOR,
                position: 5,
                color: 0,
                hoist: false,
                mentionable: false,
                self_assignable: false,
                exclusion_group: None,
                lamport: 3,
            },
        );

        let mut assignments = HashMap::new();
        let mut admin_5 = HashSet::new();
        admin_5.insert(rid(5));
        assignments.insert(pseudo(5), admin_5);

        let mut admin_7 = HashSet::new();
        admin_7.insert(rid(5));
        assignments.insert(pseudo(7), admin_7);

        let mut mod_role = HashSet::new();
        mod_role.insert(rid(2));
        assignments.insert(pseudo(6), mod_role);

        GovernanceState {
            creator: Some(pseudo(1)),
            roles,
            role_assignments: assignments,
            ..Default::default()
        }
    }

    #[test]
    fn ban_admin_blocked_by_equal_rank_admin() {
        // M10.1 — admin (pseudo 5, pos 5) tries to ban another admin (pseudo 7, pos 5).
        // Equal rank → reject. This is the exact rogue-admin scenario.
        let state = state_with_three_tier_hierarchy();
        let entry = GovernanceEntry::BanEntry {
            target: pseudo(7),
            reason: None,
            lamport: 100,
        };
        assert!(
            !validate_write(&pseudo(5), &entry, &state),
            "equal-rank admin must not be able to ban another admin"
        );
    }

    #[test]
    fn ban_mod_succeeds_from_admin() {
        // M10.1 — admin (pos 5) banning a mod (pos 2): strictly higher rank → accept.
        let state = state_with_three_tier_hierarchy();
        let entry = GovernanceEntry::BanEntry {
            target: pseudo(6),
            reason: None,
            lamport: 100,
        };
        assert!(validate_write(&pseudo(5), &entry, &state));
    }

    #[test]
    fn ban_admin_blocked_from_mod() {
        // M10.1 — mod (pos 2) attempting to ban admin (pos 5): strictly lower rank → reject.
        // (Even though mod has BAN_MEMBERS perm via the role.)
        let state = state_with_three_tier_hierarchy();
        let entry = GovernanceEntry::BanEntry {
            target: pseudo(5),
            reason: None,
            lamport: 100,
        };
        assert!(!validate_write(&pseudo(6), &entry, &state));
    }

    #[test]
    fn ban_creator_blocked_from_admin() {
        // M10.2 — admin trying to ban the creator must be rejected even
        // though admin has ADMINISTRATOR perm. Creator is OWNER-tier.
        let state = state_with_three_tier_hierarchy();
        let entry = GovernanceEntry::BanEntry {
            target: pseudo(1),
            reason: None,
            lamport: 100,
        };
        assert!(!validate_write(&pseudo(5), &entry, &state));
    }

    #[test]
    fn timeout_admin_blocked_by_equal_rank_admin() {
        // M10.1 — same hierarchy enforcement applies to timeouts.
        let state = state_with_three_tier_hierarchy();
        let entry = GovernanceEntry::TimeoutEntry {
            target: pseudo(7),
            duration_seconds: 3600,
            reason: None,
            started_at: 0,
            lamport: 100,
        };
        assert!(!validate_write(&pseudo(5), &entry, &state));
    }

    #[test]
    fn timeout_creator_blocked() {
        // M10.2 — creator is immune to timeouts.
        let state = state_with_three_tier_hierarchy();
        let entry = GovernanceEntry::TimeoutEntry {
            target: pseudo(1),
            duration_seconds: 3600,
            reason: None,
            started_at: 0,
            lamport: 100,
        };
        assert!(!validate_write(&pseudo(5), &entry, &state));
    }

    #[test]
    fn unassign_admin_role_blocked_by_equal_rank_admin() {
        // M10.1 — admin (pos 5) attempting to revoke another admin's
        // (pos 5) admin role: target_max == writer_max → reject.
        let state = state_with_three_tier_hierarchy();
        let entry = GovernanceEntry::RoleUnassignment {
            target: pseudo(7),
            role_id: rid(5),
            lamport: 100,
        };
        assert!(!validate_write(&pseudo(5), &entry, &state));
    }

    #[test]
    fn unassign_creator_role_blocked_from_anyone_else() {
        // M10.2 — even if creator had a role assignment, no other peer
        // can unassign roles from them. (Synthesized: give creator the
        // admin role and then try to revoke from a peer that climbed
        // somehow to the same rank.)
        let mut state = state_with_three_tier_hierarchy();
        let mut creator_roles = HashSet::new();
        creator_roles.insert(rid(5));
        state.role_assignments.insert(pseudo(1), creator_roles);

        let entry = GovernanceEntry::RoleUnassignment {
            target: pseudo(1),
            role_id: rid(5),
            lamport: 100,
        };
        // Even though pseudo(7) is also rank 5, they cannot touch creator.
        assert!(!validate_write(&pseudo(7), &entry, &state));
    }

    #[test]
    fn unassign_self_always_allowed() {
        // M10.1 — a member can step down from their own role regardless
        // of rank. (Admin gives up admin → accepted.)
        let state = state_with_three_tier_hierarchy();
        let entry = GovernanceEntry::RoleUnassignment {
            target: pseudo(5),
            role_id: rid(5),
            lamport: 100,
        };
        assert!(validate_write(&pseudo(5), &entry, &state));
    }

    #[test]
    fn unassign_self_creator_allowed() {
        // M10.2 — creator unassigning their own role is allowed (the
        // OWNER-block only blocks OTHER peers).
        let mut state = state_with_three_tier_hierarchy();
        let mut creator_roles = HashSet::new();
        creator_roles.insert(rid(5));
        state.role_assignments.insert(pseudo(1), creator_roles);

        let entry = GovernanceEntry::RoleUnassignment {
            target: pseudo(1),
            role_id: rid(5),
            lamport: 100,
        };
        assert!(validate_write(&pseudo(1), &entry, &state));
    }

    #[test]
    fn role_assignment_blocked_at_or_above_writer_rank() {
        // M10.1 — mod (pos 2) trying to grant the admin role (pos 5) to
        // pseudo(99): role_position >= writer_max → reject.
        let state = state_with_three_tier_hierarchy();
        let entry = GovernanceEntry::RoleAssignment {
            target: pseudo(99),
            role_id: rid(5),
            lamport: 100,
        };
        assert!(!validate_write(&pseudo(6), &entry, &state));
    }

    #[test]
    fn role_assignment_below_rank_succeeds() {
        // M10.1 — admin (pos 5) granting mod role (pos 2) to a member:
        // role_position < writer_max → accept.
        let state = state_with_three_tier_hierarchy();
        let entry = GovernanceEntry::RoleAssignment {
            target: pseudo(99),
            role_id: rid(2),
            lamport: 100,
        };
        assert!(validate_write(&pseudo(5), &entry, &state));
    }

    #[test]
    fn role_definition_blocked_at_or_above_writer_rank() {
        // M10.1 — admin (pos 5) trying to mint a new role at position 5
        // (which they could then grant themselves to climb): reject.
        let state = state_with_three_tier_hierarchy();
        let entry = GovernanceEntry::RoleDefinition {
            role_id: rid(99),
            name: "shadow-admin".into(),
            permissions: ADMINISTRATOR,
            position: 5,
            color: 0,
            hoist: false,
            mentionable: false,
            self_assignable: false,
            exclusion_group: None,
            lamport: 100,
        };
        assert!(!validate_write(&pseudo(5), &entry, &state));
    }

    #[test]
    fn role_definition_below_rank_succeeds() {
        // M10.1 — admin (pos 5) mints a role at position 3 (below them):
        // accept.
        let state = state_with_three_tier_hierarchy();
        let entry = GovernanceEntry::RoleDefinition {
            role_id: rid(3),
            name: "trial-mod".into(),
            permissions: TIMEOUT_MEMBERS,
            position: 3,
            color: 0,
            hoist: false,
            mentionable: false,
            self_assignable: false,
            exclusion_group: None,
            lamport: 100,
        };
        assert!(validate_write(&pseudo(5), &entry, &state));
    }

    #[test]
    fn creator_can_act_at_any_rank() {
        // M10.2 — creator-bypass (validate.rs:61) means creator can ban
        // any peer, even another admin at equal rank.
        let state = state_with_three_tier_hierarchy();
        let entry = GovernanceEntry::BanEntry {
            target: pseudo(5),
            reason: None,
            lamport: 100,
        };
        assert!(validate_write(&pseudo(1), &entry, &state));
    }

    #[test]
    fn welcome_screen_rejects_too_many_channels() {
        use rekindle_types::governance::WelcomeChannel;
        let state = state_with_creator_and_roles();
        let mut channels = Vec::new();
        for i in 0..6_u8 {
            channels.push(WelcomeChannel {
                channel_id: rekindle_types::id::ChannelId([i; 16]),
                description: "d".into(),
                emoji: None,
            });
        }
        let entry = GovernanceEntry::WelcomeScreen {
            description: "d".into(),
            channels,
            lamport: 11,
        };
        assert!(!validate_write(&pseudo(5), &entry, &state));
    }
}
