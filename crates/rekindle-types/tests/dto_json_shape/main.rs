//! Golden JSON shape of every DTO a Host call answers (plan E1.1, recipe
//! step 2): the GUI, the CLI and the TUI decode these, and the TypeScript
//! side (`src/ipc/commands/dto.ts`) types them by hand. One sample per
//! shape (and per enum variant), every key camelCase, and the JSON equal to
//! `tests/golden/dto.json` (`UPDATE_GOLDEN=1` after a deliberate change).

#[path = "../common/json.rs"]
mod json;

use std::path::PathBuf;

use rekindle_types::dto::audit::{AuditEntryView, AuditVerified};
use rekindle_types::dto::community::{
    CommunityBanView, CommunityInviteCreated, CommunityInviteView, CommunityMemberView,
};
use rekindle_types::dto::dm::{DmInboxEntry, DmMessage};
use rekindle_types::dto::friends::{
    BlockedUser, Friend, FriendCodeView, InviteCreated, OutgoingInvite, PendingRequest,
    SafetyNumberView,
};
use rekindle_types::dto::group_dm::{GroupDmView, GroupDmViewMember};
use rekindle_types::dto::identity::{IdentityShown, IdentitySummary, IdentityView};
use rekindle_types::dto::onboarding::{
    GuideStepDto, OnboardingConfigDto, OnboardingModeDto, OnboardingOptionDto,
    OnboardingQuestionDto, WelcomeChannelEntryDto, WelcomeScreenDto,
};
use rekindle_types::dto::presence::{OwnGame, OwnPresenceView};
use rekindle_types::dto::{CommandError, Done};
use rekindle_types::friend::{FriendState, FriendshipState, GameInfoState};
use rekindle_types::intent::{
    IntentClass, IntentId, IntentKind, IntentReview, Notifies, ProofRequired, Reversibility,
    Subject, UndoOffer,
};
use rekindle_types::message::SendState;
use rekindle_types::preferences::Preferences;
use rekindle_types::presence::{IdleState, SessionStatus};
use serde_json::{json, Map, Value};

fn pk(digit: char) -> String {
    digit.to_string().repeat(64)
}

fn intent_id() -> IntentId {
    serde_json::from_value(json!("01900000-0000-7000-8000-000000000001")).unwrap()
}

fn game() -> OwnGame {
    OwnGame {
        game_id: 4181,
        game_name: "Counter-Strike 2".into(),
        elapsed_seconds: 90,
        server_address: Some("10.0.0.1:27015".into()),
    }
}

fn review() -> IntentReview {
    IntentReview {
        intent_id: intent_id(),
        kind: IntentKind::DeleteIdentity,
        class: IntentClass::Protected,
        subject: Subject::OwnIdentity { name: "Me".into() },
        facts: Vec::new(),
        escalations: Vec::new(),
        reversibility: Reversibility::Never,
        notifies: Notifies::Nobody,
        proof: ProofRequired::Passphrase,
        expires_at_ms: 1_700_000_300_000,
        cancel_sends: false,
    }
}

fn to(value: impl serde::Serialize) -> Value {
    serde_json::to_value(value).expect("serialize sample")
}

/// One sample per DTO shape and enum variant.
fn samples() -> Map<String, Value> {
    let mut out = Map::new();
    let mut add = |id: &str, value: Value| {
        assert!(out.insert(id.to_owned(), value).is_none(), "duplicate {id}");
    };
    add(
        "identity.summary",
        to(IdentitySummary {
            public_key: pk('a'),
            display_name: "Ann".into(),
            created_at: 1_700_000_000_000,
        }),
    );
    add(
        "identity.view",
        to(IdentityView {
            public_key: pk('a'),
            display_name: "Ann".into(),
        }),
    );
    add(
        "identity.shown",
        to(Some(IdentityShown {
            public_key: pk('a'),
            display_name: "Ann".into(),
            profile_dht_key: format!("VLD0:{}", "p".repeat(43)),
            mailbox_dht_key: format!("VLD0:{}", "m".repeat(43)),
            friend_list_dht_key: format!("VLD0:{}", "f".repeat(43)),
            communities: 2,
        })),
    );
    add("identity.shown.locked", to(None::<IdentityShown>));
    add(
        "presence.view.playing",
        to(OwnPresenceView {
            status: SessionStatus::Busy,
            status_message: "fragging".into(),
            game: Some(game()),
            idle: IdleState::Active,
        }),
    );
    add(
        "presence.view.autoAway",
        to(OwnPresenceView {
            status: SessionStatus::Away,
            status_message: String::new(),
            game: None,
            idle: IdleState::AutoAway,
        }),
    );
    add(
        "presence.view.invisible",
        to(OwnPresenceView {
            status: SessionStatus::Invisible,
            status_message: String::new(),
            game: None,
            idle: IdleState::Unavailable,
        }),
    );
    add(
        "audit.verified.intact",
        to(AuditVerified::Intact { length: 12 }),
    );
    add(
        "audit.verified.broken",
        to(AuditVerified::Broken {
            length: 12,
            at: Some(7),
            detail: "entry 7: recomputed MAC does not match".into(),
        }),
    );
    add(
        "audit.entry",
        to(AuditEntryView {
            cursor: 7,
            prev_mac: "ab".repeat(32),
            mac: "cd".repeat(32),
            at_ms: 1_700_000_000_000,
            actor_pub: pk('a'),
            kind: "VaultUnlocked".into(),
            payload: json!({ "first_time": false }),
        }),
    );
    add(
        "command.done.undo",
        to(Done {
            value: game(),
            undo: Some(UndoOffer {
                id: intent_id(),
                inverse: IntentKind::ClearGamePresence,
                expires_at_ms: 1_700_000_010_000,
            }),
        }),
    );
    add(
        "command.done",
        to(Done {
            value: (),
            undo: None,
        }),
    );
    add(
        "command.error.failed",
        to(CommandError::Failed {
            code: 401,
            message: "Wrong passphrase".into(),
            remediation: None,
        }),
    );
    add(
        "command.error.intentRequired",
        to(CommandError::IntentRequired(review())),
    );
    add("command.error.cancelled", to(CommandError::Cancelled));
    // `SetReadReceiptsEnabled` answers the identity's preferences (plan E2.5).
    add(
        "preferences",
        to(Preferences {
            read_receipts_enabled: true,
            ..Preferences::default()
        }),
    );
    friend_samples(&mut add);
    onboarding_samples(&mut add);
    out
}

/// The onboarding flow, welcome screen and member view the Host answers
/// (plan E3.1 D3); `roleIds` is what an onboarding grant changes.
fn onboarding_samples(add: &mut impl FnMut(&str, Value)) {
    let channel = "0c".repeat(16);
    add(
        "onboarding.config.guided",
        to(OnboardingConfigDto {
            enabled: true,
            mode: OnboardingModeDto::Guided,
            default_channels: vec![channel.clone()],
            questions: vec![OnboardingQuestionDto {
                question_id: "q_01".into(),
                title: "What do you play?".into(),
                description: Some("Pick one".into()),
                required: true,
                single_select: true,
                options: vec![OnboardingOptionDto {
                    option_id: "opt_fps".into(),
                    title: "Shooters".into(),
                    description: None,
                    roles_to_assign: vec![5],
                    channels_to_show: vec![channel.clone()],
                }],
            }],
            welcome_message: Some("Welcome!".into()),
            guide_steps: vec![GuideStepDto {
                title: "Say hello".into(),
                description: "Introduce yourself".into(),
                channel_id: Some(channel.clone()),
                emoji: Some("👋".into()),
            }],
        }),
    );
    add(
        "onboarding.config.default",
        to(OnboardingConfigDto::default()),
    );
    add(
        "onboarding.welcomeScreen",
        to(WelcomeScreenDto {
            description: "LAN party HQ".into(),
            channels: vec![WelcomeChannelEntryDto {
                channel_id: channel,
                description: "Chat with everyone".into(),
                emoji: None,
            }],
        }),
    );
    add(
        "community.memberView",
        to(CommunityMemberView {
            pseudonym_key: pk('b'),
            display_name: "Bea".into(),
            role_ids: vec![0, 5],
            display_role: "Shooters".into(),
            status: "online".into(),
            timeout_until: None,
            bio: None,
            pronouns: None,
            theme_color: None,
            badges: Vec::new(),
            avatar_ref: None,
            banner_ref: None,
            location: None,
            last_active: 0,
        }),
    );
}

/// The friend and DM DTOs (plan E2.1).
fn friend_samples(add: &mut impl FnMut(&str, Value)) {
    let mut accepted = FriendState::new(pk('b'), "Bob".into(), FriendshipState::Accepted);
    accepted.nickname = Some("Bobby".into());
    accepted.status = SessionStatus::Busy;
    accepted.status_message = Some("raiding".into());
    accepted.group = Some("Raid team".into());
    accepted.unread_count = 2;
    accepted.identity_pin_mismatch = true;
    accepted.last_seen_at = Some(1_700_000_000_000);
    accepted.game_info = Some(GameInfoState {
        game_id: 4181,
        game_name: "Counter-Strike 2".into(),
        server_info: None,
        elapsed_seconds: 90,
        server_address: Some("10.0.0.1:27015".into()),
    });
    add("friends.friend.accepted", to(Friend::of(&accepted)));
    let mut pending = FriendState::new(pk('c'), "Cy".into(), FriendshipState::PendingOut);
    pending.status = SessionStatus::Online;
    pending.handshake_error = Some("their accept carried no session data".into());
    add("friends.friend.pendingOut", to(Friend::of(&pending)));
    add(
        "friends.pendingRequest",
        to(PendingRequest {
            public_key: pk('d'),
            display_name: "Dee".into(),
            message: "remember me from LAN night?".into(),
            received_at: 1_700_000_000_000,
        }),
    );
    add(
        "friends.blockedUser",
        to(BlockedUser {
            public_key: pk('e'),
            display_name: "Eve".into(),
            blocked_at: 1_700_000_000_000,
        }),
    );
    add(
        "friends.outgoingInvite",
        to(OutgoingInvite {
            invite_id: "3f2a7c1e-0000-4000-8000-000000000001".into(),
            url: "rekindle://abc".into(),
            created_at: 1_700_000_000_000,
            expires_at: 1_700_172_800_000,
            status: "responded".into(),
            accepted_by: Some(pk('f')),
        }),
    );
    add(
        "friends.inviteCreated",
        to(InviteCreated {
            url: "rekindle://abc".into(),
            invite_id: "3f2a7c1e-0000-4000-8000-000000000001".into(),
            created_at: 1_700_000_000_000,
            expires_at: 1_700_604_800_000,
        }),
    );
    add(
        "friends.friendCode",
        to(FriendCodeView {
            url: format!(
                "rekindle://add/{}/VLD0:abc/abcdefghijklmnopqrstuvwx2q",
                pk('a')
            ),
            public_key: pk('a'),
            profile_key: "VLD0:abc".into(),
            token: "abcdefghijklmnopqrstuvwx2q".into(),
            qr_rows: vec!["101".into(), "010".into(), "101".into()],
        }),
    );
    add(
        "friends.safetyNumber",
        to(SafetyNumberView {
            peer_key: pk('b'),
            digits: "123456789012345678901234567890123456789012345678901234567890".into(),
            scannable: vec![0x08, 0x02],
            qr_rows: vec!["101".into(), "010".into(), "101".into()],
            verified: false,
            verified_at: None,
            changed: true,
        }),
    );
    add(
        "dm.message",
        to(DmMessage {
            id: 9,
            message_id: Some("0192d5a8-7f00-7c3a-9b1e-4d2f6a8c0e11".into()),
            sender_id: pk('a'),
            body: "gg".into(),
            timestamp: 1_700_000_000_000,
            is_own: true,
            send_state: SendState::Delivered,
            history_gap_through: None,
        }),
    );
    add(
        "dm.historyGap",
        to(DmMessage {
            id: 10,
            message_id: None,
            sender_id: pk('b'),
            body: String::new(),
            timestamp: 1_700_000_000_000,
            is_own: false,
            send_state: SendState::Delivered,
            history_gap_through: Some(4),
        }),
    );
    add(
        "dm.inboxEntry",
        to(DmInboxEntry {
            peer_key: pk('b'),
            sender_key: pk('b'),
            body: "gg".into(),
            timestamp: 1_700_000_000_000,
            is_read: false,
        }),
    );
    let member = |digit, label: &str| GroupDmViewMember {
        key: pk(digit),
        label: label.into(),
    };
    add(
        "groupDm.view.pending",
        to(GroupDmView {
            group_id: "0f".repeat(16),
            name: "raid night".into(),
            members: vec![
                member('b', "bob"),
                member('a', "aaaa aaaa"),
                member('c', "carol"),
            ],
            epoch: 0,
            pending: true,
        }),
    );
    add(
        "community.banView",
        to(CommunityBanView {
            pseudonym_key: pk('b'),
            display_name: "Bea".into(),
        }),
    );
    add(
        "community.inviteCreated",
        to(CommunityInviteCreated {
            invite_id: "0a".repeat(16),
            code_hash: "ab".repeat(32),
            url: "rekindle://invite/VLD0:g/VLD0:s/0b0b".into(),
            expires_at: Some(1_700_604_800),
        }),
    );
    add(
        "community.inviteView.ours",
        to(CommunityInviteView {
            invite_id: "0a".repeat(16),
            code_hash: "ab".repeat(32),
            created_by: pk('a'),
            max_uses: Some(5),
            uses: 2,
            expires_at: None,
            created_at: Some(1_700_000_000_000),
            url: Some("rekindle://invite/VLD0:g/VLD0:s/0b0b".into()),
        }),
    );
}

fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/dto.json")
}

/// Keys inside `payload` are the audit record's own, written before the
/// DTO existed; everything else is the DTOs' shape.
#[test]
fn every_key_is_camel_case() {
    let mut bad = Vec::new();
    for (id, mut value) in samples() {
        if let Some(object) = value.as_object_mut() {
            object.remove("payload");
        }
        json::collect_bad_keys(&value, &format!("[{id}] $"), &mut bad);
    }
    assert!(
        bad.is_empty(),
        "keys that are not camelCase:\n{}",
        bad.join("\n")
    );
}

#[test]
fn json_matches_the_golden_file() {
    json::check_golden(&golden_path(), &samples(), "DTO");
}

/// Every DTO round-trips: what a frontend decodes is what the host sent.
#[test]
fn dtos_round_trip() {
    let samples = samples();
    let back: Done<OwnGame> = serde_json::from_value(samples["command.done.undo"].clone()).unwrap();
    assert_eq!(back.value, game());
    let error: CommandError =
        serde_json::from_value(samples["command.error.intentRequired"].clone()).unwrap();
    assert_eq!(error, CommandError::IntentRequired(review()));
    let view: OwnPresenceView =
        serde_json::from_value(samples["presence.view.playing"].clone()).unwrap();
    assert_eq!(view.game, Some(game()));
    let verified: AuditVerified =
        serde_json::from_value(samples["audit.verified.broken"].clone()).unwrap();
    assert!(matches!(
        verified,
        AuditVerified::Broken { at: Some(7), .. }
    ));
}
