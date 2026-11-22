//! IPC protocol definitions: Request, Response, and Event enums.
//!
//! These are the typed payloads carried inside `Message<T>`. The bus server
//! routes based on the `Message` envelope; the payload is opaque to routing.
//!
//! Every `IpcRequest` variant maps 1:1 to a `rekindle_transport::operations::*`
//! function or a `QueryEngine` method. There are no catch-all variants — the
//! match in `daemon::dispatch` is exhaustive without a wildcard arm.
//!
//! [RC-16] `Unlock`, `IdentityCreate`, `IdentityExport`, `IdentityImport` and
//! `IdentityRotate` contain secrets — their
//! `Debug` impls redact sensitive fields.

use serde::{Deserialize, Serialize};

use super::message::AgentType;
use rekindle_types::passphrase::Passphrase;

// ── Subscription filter (re-exported from rekindle-types) ──────────────

pub use rekindle_types::subscription_events::{
    EventCategory, SubscriptionFilter, MAX_FILTERS_PER_CONNECTION,
};

// ── IPC Request ─────────────────────────────────────────────────────────

/// Frontend → Daemon request.
///
/// Every variant is explicitly matched in `daemon::dispatch::dispatch()`.
/// No wildcard arm — adding a variant here forces a handler implementation.
///
/// Variant naming: `{Domain}{Verb}` — e.g., `ChannelCreate`, `FriendRemove`.
/// This convention makes the exhaustive match self-documenting.
///
/// [RC-16] Variants containing secrets (`Unlock`, `IdentityCreate`) have
/// custom `Debug` impls that redact sensitive fields.
#[derive(Clone, Serialize, Deserialize)]
pub enum IpcRequest {
    // ── Lifecycle ──────────────────────────────────────────────────
    /// Unlock `identity` (a public key from `IdentityList`) with its
    /// passphrase: open its vault and database, resume its session,
    /// transition to Operational.
    Unlock {
        identity: String,
        passphrase: Passphrase,
    },
    /// Lock the daemon (zeroize signing key, transition to Locked).
    Lock,
    /// Log out: announce offline, then lock and deselect the identity.
    Logout,
    /// Query daemon status (always available, any state).
    /// Returns `StatusSnapshot` with compact status, subscription health,
    /// and full diagnostic checks. Renderers decide display depth.
    Status,
    /// Graceful shutdown: drain connections, stop transport, exit.
    /// Returns Ok before the process exits so the client gets confirmation.
    Shutdown,

    // ── Identity ──────────────────────────────────────────────────
    /// Create a new identity and unlock it (plan E1.1): its vault and
    /// database under `passphrase`, then the unlock, whose own-records stage
    /// publishes its DHT records once the network has a route. Without a
    /// name it is `User_` and its key's first eight characters. Answers the
    /// opened identity (`dto::identity::IdentityView`).
    IdentityCreate {
        display_name: Option<String>,
        passphrase: Passphrase,
    },
    /// The identities on this device (the registry; no unlock needed).
    IdentityList,
    /// Show the unlocked identity (pubkey, display name, DHT keys).
    IdentityShow,
    /// Export the unlocked identity as an archive (base64): its registry
    /// entry, vault and header, sealed by the vault's passphrase.
    IdentityExport { passphrase: Passphrase },
    /// Install an identity from an exported archive (base64); it is
    /// opened with `passphrase` to verify it, and stays locked.
    IdentityImport {
        archive: String,
        passphrase: Passphrase,
    },
    /// Rotate the unlocked identity's Ed25519 keypair: friends are told,
    /// then the identity is locked and moved to its new key.
    IdentityRotate { passphrase: Passphrase },
    /// Delete `identity`. Protected: the host's intent gate asks for its
    /// passphrase (plan D3.3).
    IdentityDelete { identity: String },
    /// Factory reset: delete every identity and Veilid storage. Protected.
    IdentityWipe,
    /// Arm the quick wipe (plan E7.1, Protected `ArmQuickWipe`): the
    /// duress passphrase goes into the vault header, the optional panic
    /// shortcut into `app_settings.panic_shortcut`.
    QuickWipeArm {
        duress_passphrase: Passphrase,
        shortcut: Option<String>,
    },
    /// Disarm the quick wipe (Protected `DisarmQuickWipe`).
    QuickWipeDisarm,
    /// The panic shortcut's wipe (Routine `TriggerQuickWipe`, no review);
    /// 409 while no shortcut is armed. Window-only in shell mode.
    QuickWipeTrigger,
    /// Our safety number with `peer` and whether we verified their key
    /// (`dto::friends::SafetyNumberView`, plan E2.3).
    IdentitySafetyNumber { peer: String },
    /// Mark `peer`'s identity key verified, after comparing the safety
    /// number out of band; acknowledges an unacknowledged key change.
    IdentityMarkVerified { peer: String },
    /// Withdraw `peer`'s verification.
    IdentityClearVerification { peer: String },

    // ── Friends (plan E2.1) ───────────────────────────────────────
    /// Send a friend request to `public_key` (they become a pending friend).
    /// Their name is the Host's (plan audit U-7), never the caller's.
    /// Send a friend request to the identity a friend code names
    /// (`rekindle://add/<key>/<profile>/<token>`, plan E2.5 D17).
    FriendAddFromCode { code: String, message: String },
    /// Accept the pending request from `public_key` (named by the request).
    FriendAccept { public_key: String },
    /// Reject the pending request from `public_key`.
    FriendReject { public_key: String },
    /// Remove a friend.
    FriendRemove { public_key: String },
    /// The buddy list (`dto::friends::Friend`).
    FriendList,
    /// The requests waiting for our answer.
    FriendRequests,
    /// Cancel the request we sent `public_key`.
    FriendCancelRequest { public_key: String },
    /// Retry the first contact with `public_key` that failed (plan E2.4,
    /// B7): our accept, or our reset accept when that was the one pending,
    /// from a fresh slot of their prekey record.
    FriendRetryHandshake { public_key: String },
    /// Add the friend an invite link names (reviewed with their key's
    /// fingerprint).
    FriendAddFromInvite { invite: String },
    /// Make a tracked friend invite link.
    FriendInviteCreate,
    /// Our open invites.
    FriendInviteList,
    /// Cancel an open invite.
    FriendInviteCancel { invite_id: String },
    /// Our friend code, its parts and its QR (`dto::friends::FriendCodeView`,
    /// plan E2.5 D17).
    FriendCode,
    /// Mint a new friend-request token: codes carrying the old one stop
    /// working and friends are untouched (`dto::friends::FriendCodeView`).
    FriendTokenRotate,
    /// Block `public_key` (our profile moves to a new key).
    FriendBlock { public_key: String },
    /// Unblock `public_key`.
    FriendUnblock { public_key: String },
    /// Everyone we blocked.
    FriendBlockedList,
    /// Set (or clear) our nickname for a friend.
    FriendSetNickname {
        public_key: String,
        nickname: Option<String>,
    },
    /// Create a friend group.
    FriendGroupCreate { name: String },
    /// Rename a friend group.
    FriendGroupRename { group_id: i64, name: String },
    /// Move a friend into a group, or out of any with `None`.
    FriendGroupMove {
        public_key: String,
        group_id: Option<i64>,
    },
    /// Delete a friend group; its friends fall back to no group.
    FriendGroupDelete { group_id: i64 },
    /// Reset our secure session with a friend (safety-number checked).
    SessionResetStart { public_key: String },
    /// Re-initiate our secure session with a friend whose retry requests
    /// are exhausted (plan E2.5 last stage, ruling Q1): the backend holds
    /// it for the user's review, and commit runs the same reset.
    SessionReinitiate { public_key: String },
    /// Accept the session reset a friend asked for.
    SessionResetAccept { public_key: String },
    /// Decline the session reset a friend asked for.
    SessionResetDecline { public_key: String },
    /// Accept the identity rotation a friend (by their current key) sent:
    /// they move to the new key (owner ruling R1). `new_key` is the key the
    /// review named; a rotation replaced since is refused, never accepted
    /// unreviewed (finding 79 review, M2).
    IdentityRotationAccept { public_key: String, new_key: String },
    /// Decline a friend's identity rotation to `new_key`: it is dropped,
    /// nothing moves; a rotation replaced since is refused as above.
    IdentityRotationDecline { public_key: String, new_key: String },

    // ── Communities (plan E3.1) ──────────────────────────────────
    /// A community-domain request: communities, channels, categories,
    /// roles, moderation, invites, game servers and channel keys.
    Community(CommunityRequest),

    // ── DMs ───────────────────────────────────────────────────────
    /// Send a direct message.
    DmSend {
        peer_key: String,
        body: String,
        /// The parent message's client id (a UUID) when this is a reply.
        reply_to: Option<String>,
    },
    /// The draft to a peer changed: `typing` is whether it holds text. The
    /// Host decides what goes out (the start, its refresh, the idle stop;
    /// plan E2.3).
    DmTyping { peer_key: String, typing: bool },
    /// The latest message of each direct conversation.
    DmInbox { limit: u32 },
    /// The newest `limit` messages with `peer_key` before `before` (ms).
    DmHistory {
        peer_key: String,
        limit: u32,
        before: Option<i64>,
    },
    /// Mark everything from `peer_key` read; `rendered_through` is the
    /// client id of the newest of their messages the focused window
    /// rendered, for our read marker (plan E2.5).
    DmMarkRead {
        peer_key: String,
        rendered_through: Option<String>,
    },
    /// Retry an outbox row by its client id (plan E2.5): queued again
    /// under the same id, and no new row is stored.
    OutboxRetry { client_id: String },
    /// Discard a failed outbox row: it settles failed and leaves its
    /// stream.
    OutboxDiscard { client_id: String },

    // ── Group DMs (plan E2.6) ──────────────────────────────────────
    /// Create a group named `name` of us and `members` (2-7 friends'
    /// keys); the answer is its id.
    GroupDmCreate { name: String, members: Vec<String> },
    /// Accept the pending invite to `group_id` (32 hex).
    GroupDmAccept { group_id: String },
    /// Decline the pending invite to `group_id`.
    GroupDmDecline { group_id: String },
    /// Leave `group_id`; every other member rotates its key.
    GroupDmLeave { group_id: String },
    /// Set the disappearing-message timer of `conversation` (a friend's
    /// key: our DM; a group's 32-hex id: the group DM) to `seconds`
    /// (0 = off, at most four weeks; plan E7.1). Every other member is
    /// told and every window shows the notice.
    DisappearingSet { conversation: String, seconds: u32 },
    /// The disappearing-message timer `conversation` (as
    /// `DisappearingSet` names it) carries now (0 = off; plan E7.1), with
    /// the presets the chat window's timer control offers.
    DisappearingGet { conversation: String },
    /// Send `body` to `group_id`; the answer is the message's client id
    /// once its row and local echo are committed (`queued`, then
    /// `GroupDmEvent::SendStatus`).
    GroupDmSend { group_id: String, body: String },
    /// The draft to `group_id` changed: `typing` is whether it holds text.
    /// The Host decides what goes out, as for `DmTyping`.
    GroupDmTyping { group_id: String, typing: bool },
    /// Every group we are in and every invite we have not answered.
    GroupDmList,
    /// The newest `limit` messages of `group_id` before `before` (ms).
    GroupDmMessages {
        group_id: String,
        limit: u32,
        before: Option<i64>,
    },
    /// The group's window displayed its messages (plan E7.1): the
    /// received messages whose disappearing timer had not started start it.
    GroupDmMarkRead { group_id: String },

    // ── Intents (plan D3.3, ADR 0019) ──────────────────────────────
    /// Commit a held action with the proof its review asked for.
    IntentCommit {
        intent_id: rekindle_types::intent::IntentId,
        proof: rekindle_types::intent::IntentProof,
    },
    /// Drop a held action.
    IntentCancel {
        intent_id: rekindle_types::intent::IntentId,
    },
    /// Take back an action that ran, while its offer lasts (plan E1.1):
    /// the inverse the host stored goes through the gate as a request of
    /// its own.
    Undo {
        id: rekindle_types::intent::IntentId,
    },

    // ── Subscriptions ─────────────────────────────────────────────
    /// Subscribe to events matching filters. `from` is the last cursor this
    /// client processed (plan D3.2): what followed it is replayed, then
    /// `Resumed`, or `InvalidSession` when the host can no longer serve it.
    /// `None` subscribes to live events only.
    Subscribe {
        filters: Vec<SubscriptionFilter>,
        from: Option<rekindle_types::subscription_events::Cursor>,
    },
    /// Unsubscribe from events matching filters.
    Unsubscribe { filters: Vec<SubscriptionFilter> },

    // ── Prekeys ─────────────────────────────────────────────────
    /// This device's prekey record and its upkeep (plan E2.4):
    /// `dto::identity::PrekeyStatus`.
    PrekeyStatus,

    // ── Presence and profile (plan E1.1) ──────────────────────────
    /// Set our status. `Offline` is refused: only the host's teardown
    /// writes it; "Appear Offline" is `Invisible`, which peers cannot tell
    /// from it.
    PresenceSet {
        status: rekindle_types::presence::SessionStatus,
    },
    /// Set our status message (`MAX_STATUS_LEN`).
    PresenceSetMessage { message: String },
    /// Our own status, message, game and idle state
    /// (`dto::presence::OwnPresenceView`).
    PresenceGet,
    /// Our time in each detected game and when we last played it, the
    /// most recent first (`Vec<dto::presence::GameTotalView>`, plan E7.2).
    GameTotalsGet,
    /// A friend's time in each game, as they last shared it (plan E7.2:
    /// `friend_game_totals`, `Vec<dto::presence::GameTotalView>`, empty
    /// when they share none).
    FriendGameTotalsGet { public_key: String },
    /// Our bio (plan E7.2: `identity.bio`, `Option<String>`).
    ProfileBioGet,
    /// A friend's bio as their last `ProfileBio` stated it (plan E7.2:
    /// `friends.bio`, `Option<String>`).
    FriendBioGet { public_key: String },
    /// Change our bio (plan E7.2): blank removes it; 190 characters at most.
    /// It goes to every accepted friend as a ratchet-sealed `ProfileBio`,
    /// never in the profile record. The bio as stored comes back.
    ProfileSetBio { bio: String },
    /// Change the name we show: the identity's database, the registry and
    /// profile subkey 0.
    ProfileSetNickname { display_name: String },
    /// Turn DM read receipts on or off (plan E2.5, r10): the identity's
    /// `Preferences.read_receipts_enabled`; the preferences come back.
    SetReadReceiptsEnabled { enabled: bool },
    /// Share our game totals with friends, or stop (plan E7.2): the
    /// identity's `Preferences.share_game_totals`; turning it on sends them
    /// to every accepted friend. The preferences come back.
    SetShareGameTotals { enabled: bool },
    /// Scan for games or stop (plan E7.2): the identity's
    /// `Preferences.game_detection_enabled`, local only. Off, no process is
    /// scanned; the change takes effect at once. The preferences come back.
    SetGameDetection { enabled: bool },
    /// Show friends the game we play, or stop (plan E7.2): the identity's
    /// `Preferences.share_current_game`, split from detection, off by
    /// default; a hidden game is never shown. The preferences come back.
    SetShareCurrentGame { enabled: bool },
    /// Keep the local activity feed or stop (plan E7.15, Infoview design
    /// §3.3): the identity's `Preferences.keep_activity_log`, on by
    /// default, local only. Off clears the feed and no entry is written
    /// until it is turned on again. The preferences come back.
    SetKeepActivityLog { enabled: bool },
    /// The activity feed, newest first (plan E7.15, the Infoview's "View
    /// More"): up to `limit` (1–100) entries after row `before` in that
    /// order (`Vec<activity::ActivityListItem>`), each friend's names as
    /// the Host holds them now.
    ActivityList { before: Option<i64>, limit: u16 },
    /// A subject's Infoview (plan E7.15, IV4; design §2.2:
    /// `dto::infoview::InfoviewView`), composed by the Host from what this
    /// identity already holds. `window` now shows `subject`, and the Host
    /// pushes its view again as it changes (`InfoviewEvent::Updated`, to
    /// that window only). The desktop shell fills `window` from the calling
    /// webview's own label, never from the page (lead decision (48)); the
    /// CLI and TUI send their fixed labels.
    InfoviewGet {
        subject: crate::protocol::InfoviewSubject,
        window: String,
    },
    /// The games on the hide list (plan E7.2,
    /// `Vec<dto::presence::HiddenGameView>`).
    HiddenGamesGet,
    /// Hide a game from everyone (plan E7.2): never our shown game, never in
    /// the totals friends receive; still recorded locally. The hide list
    /// comes back.
    GameHide { game_id: u32 },
    /// Take a game off the hide list (plan E7.2). The hide list comes back.
    GameUnhide { game_id: u32 },
    /// The games we can launch (plan E7.15 IV3,
    /// `Vec<dto::presence::InstalledGameView>`): the bundled database's
    /// games Steam has installed (read only while game detection is on),
    /// with the games we have played, minus the hide list.
    InstalledGamesGet,
    /// Launch a game, to `server` when given (lead decision (50)): the one
    /// launch path for every frontend. `source` says where the address came
    /// from and so the intent class: a peer's address is TrustChanging, the
    /// review naming it; our own favourite or an installed game is Routine.
    /// `Installed` takes no server, every other source one.
    GameLaunch {
        game_id: u32,
        server: Option<rekindle_types::game_server::ServerAddress>,
        source: rekindle_types::game_server::JoinSource,
    },
    /// Our favourite game servers (plan E7.2), each with whether an invite
    /// may name it (`invitable`: not a hidden game, current-game sharing on).
    FavouriteServersGet,
    /// Add a favourite game server, or relabel one (plan E7.2): `server` is
    /// parsed on deserialize, `label` at most 64 characters. The list comes
    /// back.
    FavouriteServerAdd {
        game_id: u32,
        server: rekindle_types::game_server::ServerAddress,
        label: String,
    },
    /// Remove a favourite game server (plan E7.2). The list comes back.
    FavouriteServerRemove {
        game_id: u32,
        server: rekindle_types::game_server::ServerAddress,
    },
    /// Invite an accepted friend to join us on a game server (plan E7.2): a
    /// ratchet-sealed `GameInvite` (0x0170) through the outbox. `server`
    /// is parsed on decode, so no unparsed address enters. Refused for a
    /// hidden game and while `share_current_game` is off. The outbox id.
    GameInviteSend {
        public_key: String,
        game_id: u32,
        server: rekindle_types::game_server::ServerAddress,
    },
    /// Change our avatar: the image, of the kind its sender declares (the
    /// file picker's MIME type, the file's extension), is decoded by
    /// `rekindle-image` as that kind only, made a 128 px WebP, saved and
    /// published to profile subkey 3.
    ProfileSetAvatar {
        image: Vec<u8>,
        kind: rekindle_types::image_kind::ImageKind,
    },
    /// Our own or a friend's avatar (WebP bytes), from the database.
    AvatarGet { public_key: String },
    /// The skins (plan E7.7.2): the built-ins and the identity's library,
    /// with the identity's choice and the device login skin.
    SkinList,
    /// Import a `rekindle-skin/1` package: validated in
    /// `rekindle-skin-helper`, stored as its canonical package `pkg:<hash>`.
    SkinImport { bytes: Vec<u8> },
    /// Remove `pkg:<hash>` from the library; the identity's skin in use is
    /// refused.
    SkinRemove { id: String },
    /// The identity's skin: a built-in or a library package (a dedicated
    /// preference setter until E6.8's `PatchPreferences` absorbs it).
    SkinSelect { id: String },
    /// The device login skin, visible before unlock: a built-in, or a
    /// stripped render-only copy of a library package.
    SkinSetLogin { id: String },
    /// Settle the "Keep this skin?" trial of `pkg:<hash>` (plan E7.7.2
    /// "Apply"): `keep` makes it the identity's skin, otherwise the skin
    /// chosen before it comes back now. Without one the Host reverts it
    /// `rekindle_types::dto::skin_library::SKIN_TRIAL_MS` after the select.
    SkinKeep { id: String, keep: bool },
    /// What a window applies (`SkinApplied`): `id`, or with `None` the
    /// resolution (the identity's choice while unlocked, else the device
    /// login skin). Answered before unlock for the login skin only.
    SkinGet { id: Option<String> },
    /// One file of the `skin://` scheme by its URL path
    /// (`/{package}/{file}`, `rekindle_skin::serve`): a built-in, the
    /// device login copy, or (unlocked) a library package.
    SkinFile { path: String },
    /// The shown skin's window frame layouts with each tile's 1-bit mask
    /// (plan SK0 S3): the shell cuts its windows to their silhouettes
    /// from these, computed in Rust, never from the webview.
    SkinFrames,
    /// The skin editor (plan E7.7.5): the Host's report on a draft, every
    /// declared pair's ratio and pass/fail, the suggested passing colours
    /// and the draft's token stylesheet (a read; nothing is stored).
    SkinValidateDraft {
        draft: rekindle_types::dto::skin_draft::SkinDraft,
    },
    /// "Start from the current skin": `id`'s active variant as a draft, or
    /// with `None` the skin this device shows now.
    SkinDraftStart { id: Option<String> },
    /// Export a draft: built, validated in `rekindle-skin-helper` (refused
    /// while any check fails, naming it) and written to `path`, a file the
    /// user chose; answers the package's address.
    SkinExportDraft {
        draft: rekindle_types::dto::skin_draft::SkinDraft,
        path: String,
    },
    /// Region import: the draft's whole package (its source's assets and
    /// imported regions) through `rekindle-skin-helper`, refused with the
    /// rule it breaks; answers the package's address (nothing is stored).
    SkinDraftCheck {
        draft: rekindle_types::dto::skin_draft::SkinDraft,
    },
    /// The JSON Schema of a skin token file for DTCG tools (Figma plugins,
    /// Style Dictionary), written to `path`, a file the user chose.
    SkinExportSchema { path: String },
    /// Run the action a skin node's trigger names (plan SK2-A4, design
    /// §2.2): a feature action runs as its own request through the intent
    /// gate; any other answers `dto::skin_run::SkinRun`.
    RunSkinAction {
        window: rekindle_types::dto::skin_layout::SkinWindow,
        node: String,
        action: rekindle_types::dto::skin_action::SkinAction,
    },
    /// Set or clear what `window` has selected, the selection a
    /// selection-dependent skin action acts on.
    SkinSelectionSet {
        window: rekindle_types::dto::skin_layout::SkinWindow,
        selection: Option<rekindle_types::dto::skin_run::SkinSelection>,
    },
    /// Set game presence info.
    GamePresenceSet {
        game_name: String,
        game_id: Option<u32>,
        elapsed_seconds: u32,
        server_address: Option<String>,
    },
    /// Clear game presence.
    GamePresenceClear,

    // ── Audit (plan E1.1) ─────────────────────────────────────────
    /// Verify the identity's tamper-evident log end to end
    /// (`dto::audit::AuditVerified`).
    AuditVerify,
    /// Write the log's entries after `since` (`dto::audit::AuditEntryView`)
    /// as JSON to the absolute `path` the frontend picked
    /// (`dto::audit::AuditExported`). Protected: the user reviews it first.
    AuditExport { since: u64, path: String },

    // ── Links (plan E1.1) ─────────────────────────────────────────
    /// Open an https link in the system browser: reviewed as
    /// `OpenExternalUrl` (the IP address leaves), answered with the
    /// normalized URL, which the shell opens.
    LinkOpen { url: String },

    // ── Voice ─────────────────────────────────────────────────────
    /// Join a voice channel.
    VoiceJoin {
        community: String,
        channel: String,
        muted: bool,
        deafened: bool,
    },
    /// Leave the current voice session.
    VoiceLeave,

    // ── Network / Node ────────────────────────────────────────────
    /// Get detailed network status (peers, routes, circuits).
    NetworkStatus,
    /// Get peer snapshot for display.
    NetworkPeers,

    // ── Agent Management ──────────────────────────────────────────
    /// Register as a named agent with declared capabilities.
    AgentRegister {
        name: String,
        agent_type: AgentType,
        capabilities: Vec<String>,
    },
    /// Revoke an agent's registration.
    AgentRevoke { name: String },
    /// Reload authorization policy from disk.
    PolicyReload,
}

mod capability;
mod community;
mod debug;
mod lane;
mod response;
#[cfg(test)]
mod variant_names;

pub use capability::AgentCapability;
pub use community::{
    ActivityRequest, AuditRequest, ChannelMessageRequest, ChannelRequest, CommunityRequest,
    ExpressionRequest, LifecycleRequest, MemberRequest, PinRequest, RichRequest, SettingsRequest,
    UnreadRequest,
};
pub use lane::Lane;
pub use rekindle_types::dto::infoview::InfoviewSubject;

pub use response::{BusPayload, IpcResponse};
