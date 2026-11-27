//! `CreateForumPost` from the TUI (plan E3.3, FT1; port recipe step 5): the
//! same Host request as the GUI and the CLI, its refusal shown as a toast.

use std::sync::Arc;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, RichRequest};
use rekindle_types::thread::ForumPostCreated;

use super::super::action::{Action, CommandResult};
use super::App;

impl App {
    /// Open a forum post in `channel` of `community`.
    pub(crate) fn spawn_forum_post(
        &self,
        community: String,
        channel: String,
        title: String,
        tag: Option<String>,
        body: String,
    ) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            let request =
                IpcRequest::Community(CommunityRequest::Rich(RichRequest::CreateForumPost {
                    community,
                    channel_id: channel,
                    title,
                    forum_tag: tag,
                    body,
                }));
            let created = match client.request_ok(request).await {
                Ok(value) => serde_json::from_value::<ForumPostCreated>(value)
                    .map_err(|error| error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            let action = match created {
                Ok(created) => Action::CommandComplete(Box::new(CommandResult::MessageSent {
                    message_id: created.starter_message_id,
                })),
                Err(error) => Action::CommandFailed {
                    context: "Forum post".into(),
                    error,
                },
            };
            let _ = tx.send(action);
        });
    }
}
