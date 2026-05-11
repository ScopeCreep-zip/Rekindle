//! Social feature commands: reactions, pins, events, threads, game servers.

use rekindle_node::ipc::protocol::IpcRequest;

use crate::v2::cli::SocialCmd;
use crate::v2::output::format;
use crate::v2::output::OutputMode;
use crate::v2::transport::DaemonClient;

pub async fn dispatch(cmd: &SocialCmd, client: &DaemonClient, mode: OutputMode) -> anyhow::Result<()> {
    match cmd {
        SocialCmd::ReactionAdd { community, channel, message_id, emoji } => {
            let value = client.request_ok(IpcRequest::ReactionAdd {
                community: community.clone(),
                channel: channel.clone(),
                message_id: message_id.clone(),
                emoji: emoji.clone(),
            }).await?;
            format::print_structured(&value, mode)
        }
        SocialCmd::ReactionRemove { community, channel, message_id, emoji } => {
            let value = client.request_ok(IpcRequest::ReactionRemove {
                community: community.clone(),
                channel: channel.clone(),
                message_id: message_id.clone(),
                emoji: emoji.clone(),
            }).await?;
            format::print_structured(&value, mode)
        }
        SocialCmd::EventCreate { community, title, description, start_time, end_time, channel_id, max_attendees } => {
            let value = client.request_ok(IpcRequest::EventCreate {
                community: community.clone(),
                title: title.clone(),
                description: description.clone(),
                start_time: *start_time,
                end_time: *end_time,
                channel_id: channel_id.clone(),
                max_attendees: *max_attendees,
            }).await?;
            format::print_structured(&value, mode)
        }
        SocialCmd::EventUpdate { community, event_id, title, description, start_time, end_time, max_attendees } => {
            let value = client.request_ok(IpcRequest::EventUpdate {
                community: community.clone(),
                event_id: event_id.clone(),
                title: title.clone(),
                description: description.clone(),
                start_time: *start_time,
                end_time: *end_time,
                max_attendees: *max_attendees,
            }).await?;
            format::print_structured(&value, mode)
        }
        SocialCmd::EventDelete { community, event_id } => {
            let value = client.request_ok(IpcRequest::EventDelete {
                community: community.clone(),
                event_id: event_id.clone(),
            }).await?;
            format::print_structured(&value, mode)
        }
        SocialCmd::EventRsvp { community, event_id, status } => {
            let value = client.request_ok(IpcRequest::EventRsvp {
                community: community.clone(),
                event_id: event_id.clone(),
                status: status.clone(),
            }).await?;
            format::print_structured(&value, mode)
        }
        SocialCmd::EventRemind { community, event_id, title, minutes } => {
            let value = client.request_ok(IpcRequest::EventRemind {
                community: community.clone(),
                event_id: event_id.clone(),
                title: title.clone(),
                minutes_until: *minutes,
            }).await?;
            format::print_structured(&value, mode)
        }
        SocialCmd::ThreadCreate { community, channel, parent_message_id, title, auto_archive_seconds } => {
            let value = client.request_ok(IpcRequest::ThreadCreate {
                community: community.clone(),
                channel: channel.clone(),
                parent_message_id: parent_message_id.clone(),
                title: title.clone(),
                auto_archive_seconds: *auto_archive_seconds,
            }).await?;
            format::print_structured(&value, mode)
        }
        SocialCmd::ThreadMessage { community, thread_id, ciphertext, mek_generation, reply_to_id } => {
            let ct_bytes = hex::decode(ciphertext)
                .map_err(|e| anyhow::anyhow!("invalid hex ciphertext: {e}"))?;
            let value = client.request_ok(IpcRequest::ThreadMessage {
                community: community.clone(),
                thread_id: thread_id.clone(),
                ciphertext: ct_bytes,
                mek_generation: *mek_generation,
                reply_to_id: reply_to_id.clone(),
            }).await?;
            format::print_structured(&value, mode)
        }
        SocialCmd::ThreadArchive { community, thread_id, archived } => {
            let value = client.request_ok(IpcRequest::ThreadArchive {
                community: community.clone(),
                thread_id: thread_id.clone(),
                archived: *archived,
            }).await?;
            format::print_structured(&value, mode)
        }
        SocialCmd::GameServerAdd { community, game_id, label, address } => {
            let value = client.request_ok(IpcRequest::GameServerAdd {
                community: community.clone(),
                game_id: game_id.clone(),
                label: label.clone(),
                address: address.clone(),
            }).await?;
            format::print_structured(&value, mode)
        }
        SocialCmd::GameServerRemove { community, server_id } => {
            let value = client.request_ok(IpcRequest::GameServerRemove {
                community: community.clone(),
                server_id: server_id.clone(),
            }).await?;
            format::print_structured(&value, mode)
        }
    }
}
