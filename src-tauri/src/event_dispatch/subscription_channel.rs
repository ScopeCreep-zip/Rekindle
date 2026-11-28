//! Frontend channel routing for host subscription events.

use rekindle_types::subscription_events::{
    ChannelMessageEvent, SubscriptionEvent, SystemEvent, TypingContext, TypingEvent,
};

/// Which frontend channel a [`SubscriptionEvent`] is consumed on.
///
/// Many families share a channel because the frontend's handlers are
/// organized by window, not by family.
pub(super) fn subscription_channel(event: &SubscriptionEvent) -> &'static str {
    match event {
        SubscriptionEvent::Presence(_) => "presence-event",
        // Every window applies it, not only the community dispatcher.
        SubscriptionEvent::System(SystemEvent::SkinChanged { .. }) => "skin-event",
        SubscriptionEvent::System(SystemEvent::SkinTrialChanged { .. }) => "skin-trial-event",
        SubscriptionEvent::System(SystemEvent::SkinApplyFailed { .. }) => "skin-apply-event",
        SubscriptionEvent::System(SystemEvent::QuickWipeChanged { .. }) => "quick-wipe-event",
        SubscriptionEvent::System(SystemEvent::ActivityAppended { .. }) => "activity-event",
        SubscriptionEvent::System(SystemEvent::ServersViewChanged { .. }) => "servers-event",
        SubscriptionEvent::System(SystemEvent::Infoview(_)) => "infoview-event",
        SubscriptionEvent::System(SystemEvent::SkinSignals { .. }) => "skin-signals-event",
        SubscriptionEvent::System(SystemEvent::SkinShell { .. }) => "skin-shell-event",
        SubscriptionEvent::Voice(_) => "voice-event",
        SubscriptionEvent::Video(_) => "video-event",
        // `ChannelMessage` and `Typing` split by context: the community
        // half goes to the community dispatcher, the direct half to the
        // chat handlers.
        SubscriptionEvent::ChannelMessage(
            ChannelMessageEvent::New { .. }
            | ChannelMessageEvent::Edited { .. }
            | ChannelMessageEvent::Deleted { .. }
            | ChannelMessageEvent::ChannelSendStatus { .. }
            | ChannelMessageEvent::PostAdmissionChanged { .. }
            | ChannelMessageEvent::ChannelHistoryGap { .. }
            | ChannelMessageEvent::ChannelHistoryGapClosed { .. }
            | ChannelMessageEvent::AttachmentDownloaded { .. }
            | ChannelMessageEvent::ExpressionAssetReady { .. },
        )
        | SubscriptionEvent::Typing(
            TypingEvent::Started {
                context: TypingContext::Channel { .. },
                ..
            }
            | TypingEvent::Stopped {
                context: TypingContext::Channel { .. },
                ..
            },
        )
        | SubscriptionEvent::Membership(_)
        | SubscriptionEvent::Governance(_)
        | SubscriptionEvent::Crypto(_)
        | SubscriptionEvent::Social(_)
        // System events (announcements, raids, automod, lockdowns, sync,
        // audit) are handled by the community dispatcher.
        | SubscriptionEvent::System(_)
        | SubscriptionEvent::UnreadChanged {
            context: rekindle_types::subscription_events::UnreadContext::Channel { .. },
            ..
        } => "community-event",
        // A DM's or the friend requests' unread count goes to the friend
        // and chat handlers.
        SubscriptionEvent::UnreadChanged { .. }
        | SubscriptionEvent::ChannelMessage(_)
        | SubscriptionEvent::Typing(_)
        | SubscriptionEvent::Call(_)
        | SubscriptionEvent::Friend(_)
        | SubscriptionEvent::GroupDm(_) => "chat-event",
        SubscriptionEvent::Notification(_) => "notification-event",
        SubscriptionEvent::IntentPending(_) => "intent-event",
        SubscriptionEvent::Network(_) => "network-status",
        SubscriptionEvent::Transfer(_) => "transfer-event",
    }
}
