use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use rekindle_codec::community::envelope::CommunityEnvelope;
use rekindle_governance::op::OpStamp;
use rekindle_governance::state::{ChannelState, GovernanceState};
use rekindle_records::lease::{CommunityLeases, LeaseId};
use rekindle_types::attachment::AttachmentOffer;
use rekindle_types::channel_keys::ChannelKeyProvider;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::{ChannelId, RoleId};
use rekindle_types::message::SendState;

use super::{check_forum_post, ForumPost};
use crate::automod::AutoModCompiledCache;
use crate::deps::{
    ChannelInfoSnapshot, ChannelMessageRow, ChannelMessagingDeps, ChannelPost, ChannelSendOutcome,
    MemberMentionView, OutboxItem, PseudonymCredentials, RoleSnapshot, SentChannelMessageEcho,
    ThreadInfoSnapshot, ThreadStateSnapshot,
};
use crate::error::ChannelError;
use crate::event::ChannelEvent;

const CHANNEL_ID: &str = "01010101010101010101010101010101";
const COMMUNITY_ID: &str = "community";
const FORUM_TAG: &str = "help";

fn valid_post() -> ForumPost<'static> {
    ForumPost {
        channel_id: CHANNEL_ID,
        title: "How do I join?",
        forum_tag: Some(FORUM_TAG.into()),
        body: "I have an invite.",
    }
}

fn assert_refused(deps: &ForumDeps, post: &ForumPost<'_>, expected: &str) {
    match check_forum_post(deps, COMMUNITY_ID, post) {
        Err(ChannelError::InvalidForumPost(message)) => assert_eq!(message, expected),
        result => panic!("expected InvalidForumPost({expected:?}), got {result:?}"),
    }
}

#[test]
fn refuses_non_forum_channel() {
    let deps = ForumDeps::new("text", Some(vec![FORUM_TAG.into()]));
    assert_refused(&deps, &valid_post(), "the channel is not a forum");
}

#[test]
fn refuses_unknown_forum_tag() {
    let deps = ForumDeps::tagged_forum();
    let mut post = valid_post();
    post.forum_tag = Some("stale".into());
    assert_refused(&deps, &post, "the tag is not one of the forum's tags");
}

#[test]
fn refuses_tag_when_forum_has_no_tags() {
    for tags in [None, Some(Vec::new())] {
        let deps = ForumDeps::new("forum", tags);
        assert_refused(
            &deps,
            &valid_post(),
            "the tag is not one of the forum's tags",
        );
    }
}

#[test]
fn refuses_empty_title() {
    let deps = ForumDeps::tagged_forum();
    for title in ["", " \t\n"] {
        let post = ForumPost {
            title,
            ..valid_post()
        };
        assert_refused(&deps, &post, "the title is empty");
    }
}

#[test]
fn refuses_empty_starter() {
    let deps = ForumDeps::tagged_forum();
    for body in ["", " \t\n"] {
        let post = ForumPost {
            body,
            ..valid_post()
        };
        assert_refused(&deps, &post, "the starter is empty");
    }
}

#[test]
fn accepts_valid_forum_post() {
    let deps = ForumDeps::tagged_forum();
    check_forum_post(&deps, COMMUNITY_ID, &valid_post()).expect("valid forum post");
}

/// A governance snapshot is the only dependency used by the validator.
struct ForumDeps(GovernanceState);

impl ForumDeps {
    fn new(channel_type: &str, forum_tags: Option<Vec<String>>) -> Self {
        let mut state = GovernanceState::default();
        state.channels.insert(
            ChannelId::from_hex(CHANNEL_ID).expect("channel id"),
            ChannelState {
                name: "discussion".into(),
                channel_type: channel_type.into(),
                category_id: None,
                position: 0,
                topic: None,
                forum_tags,
                slowmode_seconds: None,
                nsfw: None,
                parent_voice_channel_id: None,
                created: OpStamp::MIN,
            },
        );
        Self(state)
    }

    fn tagged_forum() -> Self {
        Self::new("forum", Some(vec![FORUM_TAG.into()]))
    }
}

#[track_caller]
fn unexpected_dependency() -> ! {
    panic!("forum validation only reads governance state")
}

#[async_trait]
impl ChannelMessagingDeps for ForumDeps {
    fn now_secs(&self) -> u64 {
        unexpected_dependency()
    }

    fn identity_secret(&self) -> Option<[u8; 32]> {
        unexpected_dependency()
    }

    fn owner_key(&self) -> Option<String> {
        unexpected_dependency()
    }

    fn my_pseudonym_hex(&self, _community_id: &str) -> Option<String> {
        unexpected_dependency()
    }

    fn my_role_ids(&self, _community_id: &str) -> Vec<RoleId> {
        unexpected_dependency()
    }

    fn pseudonym_credentials(
        &self,
        _community_id: &str,
    ) -> Result<PseudonymCredentials, ChannelError> {
        unexpected_dependency()
    }

    fn channel_info(&self, _community_id: &str, _channel_id: &str) -> Option<ChannelInfoSnapshot> {
        unexpected_dependency()
    }

    fn keys(&self) -> Arc<dyn ChannelKeyProvider> {
        unexpected_dependency()
    }

    fn governance_state(&self, community_id: &str) -> Option<GovernanceState> {
        assert_eq!(community_id, COMMUNITY_ID);
        Some(self.0.clone())
    }

    fn automod_compiled_cache_get(&self, _community_id: &str) -> Option<Arc<AutoModCompiledCache>> {
        unexpected_dependency()
    }

    fn automod_compiled_cache_set(&self, _community_id: &str, _cache: Arc<AutoModCompiledCache>) {
        unexpected_dependency()
    }

    fn thread_state(&self, _community_id: &str, _thread_id: &str) -> Option<ThreadStateSnapshot> {
        unexpected_dependency()
    }

    fn member_profile(&self, _community_id: &str, _pseudonym_hex: &str) -> MemberMentionView {
        unexpected_dependency()
    }

    fn list_member_profiles(&self, _community_id: &str) -> HashMap<String, MemberMentionView> {
        unexpected_dependency()
    }

    fn community_roles(&self, _community_id: &str) -> Vec<RoleSnapshot> {
        unexpected_dependency()
    }

    fn compute_my_permissions(&self, _community_id: &str) -> u64 {
        unexpected_dependency()
    }

    fn mark_last_send_at(&self, _community_id: &str, _channel_id: &str, _now_ms: i64) {
        unexpected_dependency()
    }

    async fn release_record(&self, _lease: LeaseId) {
        unexpected_dependency()
    }

    async fn read_outbox_entries(&self, _community_id: &str, _stream_id: &str) -> Vec<OutboxItem> {
        unexpected_dependency()
    }

    async fn community_records_ready(&self, _community_id: &str, _leases: CommunityLeases) {
        unexpected_dependency()
    }

    async fn queue_post(&self, _post: &ChannelPost) -> Result<(), ChannelError> {
        unexpected_dependency()
    }

    async fn fail_post(&self, _message: &str) {
        unexpected_dependency()
    }

    async fn queue_waiting_post(
        &self,
        _post: &ChannelPost,
        _deadline_ms: i64,
    ) -> Result<(), ChannelError> {
        unexpected_dependency()
    }

    async fn waiting_posts(&self, _community_id: &str) -> Vec<ChannelPost> {
        unexpected_dependency()
    }

    async fn release_waiting_post(&self, _message: &str) -> bool {
        unexpected_dependency()
    }

    async fn persist_sent_message(
        &self,
        _community_id: &str,
        _channel_id: &str,
        _outcome: &ChannelSendOutcome,
        _body: &str,
    ) -> Result<(), ChannelError> {
        unexpected_dependency()
    }

    async fn persist_forwarded_message(
        &self,
        _community_id: &str,
        _channel_id: &str,
        _outcome: &ChannelSendOutcome,
        _body: &str,
        _original_author_pseudonym: &str,
    ) -> Result<(), ChannelError> {
        unexpected_dependency()
    }

    async fn persist_slowmode_state(
        &self,
        _community_id: &str,
        _channel_id: &str,
        _now_ms: i64,
    ) -> Result<(), ChannelError> {
        unexpected_dependency()
    }

    async fn find_channel_message_by_id(
        &self,
        _channel_id: &str,
        _message_id: &str,
    ) -> Option<ChannelMessageRow> {
        unexpected_dependency()
    }

    async fn persist_thread_row(
        &self,
        _community_id: &str,
        _thread: &ThreadInfoSnapshot,
    ) -> Result<(), ChannelError> {
        unexpected_dependency()
    }

    async fn load_thread_metadata(
        &self,
        _community_id: &str,
        _thread_id: &str,
    ) -> Option<ThreadInfoSnapshot> {
        unexpected_dependency()
    }

    fn upload_expression_to_cache(
        &self,
        _community_id: &str,
        _expression_id: [u8; 16],
        _bytes: &[u8],
        _filename: String,
        _mime_type: String,
    ) -> Result<AttachmentOffer, ChannelError> {
        unexpected_dependency()
    }

    fn read_expression_bytes(
        &self,
        _community_id: &str,
        _offer: &AttachmentOffer,
    ) -> Option<Vec<u8>> {
        unexpected_dependency()
    }

    fn send_to_mesh(
        &self,
        _community_id: &str,
        _envelope: &CommunityEnvelope,
    ) -> Result<(), ChannelError> {
        unexpected_dependency()
    }

    async fn write_governance_entry(
        &self,
        _community_id: &str,
        _entry: GovernanceEntry,
    ) -> Result<(), ChannelError> {
        unexpected_dependency()
    }

    fn require_channel_permission(
        &self,
        _community_id: &str,
        _channel_id: Option<&str>,
        _perm_bits: u64,
    ) -> Result<(), ChannelError> {
        unexpected_dependency()
    }

    fn emit_event(&self, _event: ChannelEvent) {
        unexpected_dependency()
    }

    fn emit_chat_event_local(&self, _echo: &SentChannelMessageEcho) {
        unexpected_dependency()
    }

    fn emit_delivery_succeeded(&self, _community_id: &str, _channel_id: &str, _message_id: &str) {
        unexpected_dependency()
    }

    fn emit_delivery_failed(&self, _community_id: &str, _channel_id: &str, _message_id: &str) {
        unexpected_dependency()
    }

    fn emit_send_state(
        &self,
        _community_id: &str,
        _channel_id: &str,
        _message_id: &str,
        _state: SendState,
    ) {
        unexpected_dependency()
    }
}
