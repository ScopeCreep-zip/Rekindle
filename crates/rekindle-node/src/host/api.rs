//! The host's side of [`rekindle_ipc::HostApi`] (plan E1.1): the trait,
//! [`Caller`] and [`ConnId`] live in `rekindle-ipc`, so frontends name them
//! without the node.

use rekindle_events::EventStream;
use rekindle_ipc::error::IpcError;
use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_ipc::{Caller, HostApi, MediaReceiver, MediaStream};
use rekindle_types::subscription_events::Cursor;
use rekindle_types::video::{MediaFrame, UplinkFrame};

impl super::Host {
    /// Where the unlocked identity's own-records stage is (plan E2.5 FC3),
    /// for a window's first read; changes follow as
    /// `NetworkEvent::OwnRecordsChanged`.
    #[must_use]
    pub fn own_records_stage(&self) -> rekindle_transport::operations::friend::OwnRecordsStage {
        self.ctx.friend_stores.own_records.get()
    }

    /// Journal `event` as one of the session's standing events (plan E7.1:
    /// a security alert the shell raises at login): every window that opens
    /// later is handed it ([`Self::standing`]).
    pub fn publish_standing(
        &self,
        event: rekindle_types::subscription_events::SubscriptionEvent,
    ) -> Cursor {
        self.ctx.journal.publish_standing(event)
    }

    /// Journal `event` as the current value of the session state `key`
    /// (plan C7.29: the voice roster): a window that opens later is handed
    /// the latest value, then the live changes.
    pub fn publish_state(
        &self,
        key: &str,
        event: rekindle_types::subscription_events::SubscriptionEvent,
    ) -> Cursor {
        self.ctx.journal.publish_state(key, event)
    }

    /// Our session in `community_id` as the Host holds it (plan E3.4
    /// S8-pre ruling): status and game from our own presence, the channel
    /// we are in from the community. The shell's registry row reads it
    /// until S8a removes the row.
    #[must_use]
    pub fn own_community_session(
        &self,
        community_id: &str,
    ) -> rekindle_types::presence::MemberSession {
        super::adapters::presence::own_session(&self.ctx, community_id)
    }

    /// Our sharing consent in `community_id` (`set_presence_policy`),
    /// default-deny in a community we do not hold.
    #[must_use]
    pub fn presence_policy(
        &self,
        community_id: &str,
    ) -> rekindle_types::presence::PresenceSharingPolicy {
        super::adapters::presence::presence_policy(&self.ctx, community_id)
    }

    /// The session state `key` ended: no later window is handed it.
    pub fn clear_state(&self, key: &str) {
        self.ctx.journal.clear_state(key);
    }

    /// The uplink of encoded video frames the frontends hand the host
    /// (plan E4.1): taken once, by the video engine (S8).
    #[must_use]
    pub fn take_media_uplink(&self) -> Option<MediaReceiver<UplinkFrame>> {
        self.media.take_uplink()
    }

    /// Deliver one downlink video frame to every open `media_down` stream;
    /// returns how many it reached.
    pub fn publish_media_down(&self, frame: &MediaFrame) -> usize {
        self.media.push_down(frame)
    }

    /// The session's standing events with their cursors: the snapshot a GUI
    /// window that just opened is handed before its live events.
    #[must_use]
    pub fn standing(&self) -> Vec<rekindle_types::subscription_events::Sequenced> {
        self.ctx.journal.standing()
    }
}

#[async_trait::async_trait]
impl HostApi for super::Host {
    async fn call(&self, caller: Caller, req: IpcRequest) -> IpcResponse {
        self.ctx.call(req, &caller).await
    }

    async fn subscribe(&self, from: Option<Cursor>) -> Result<EventStream, IpcError> {
        Ok(self.ctx.journal.subscribe(from))
    }

    fn media_up(&self, frame: UplinkFrame) {
        self.media.push_up(frame);
    }

    fn media_down(&self) -> MediaStream {
        self.media.open_down()
    }
}
