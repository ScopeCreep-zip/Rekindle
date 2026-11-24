//! Local search (plan E7.2 G11, SK2-G §8): the query under the message
//! search's cap, and a conversation named by a friend's key or a group
//! DM's 16-byte id.

use rekindle_ipc::protocol::IpcResponse;
use rekindle_types::dto::local_search::LocalSearchScope;

use super::pins::MAX_SEARCH_QUERY_LEN;
use super::{hex16, public_key, Checked};

pub(super) fn check(scope: &LocalSearchScope, query: &str) -> Checked {
    if query.len() > MAX_SEARCH_QUERY_LEN {
        return Err(IpcResponse::error(
            400,
            format!("a search query is at most {MAX_SEARCH_QUERY_LEN} bytes"),
        ));
    }
    match scope {
        LocalSearchScope::Friends | LocalSearchScope::Servers | LocalSearchScope::Files => Ok(()),
        // A group id is 32 hex; anything else must be a friend's key.
        LocalSearchScope::Conversation { conversation_id } if conversation_id.len() == 32 => {
            hex16(conversation_id, "conversation id")
        }
        LocalSearchScope::Conversation { conversation_id } => {
            public_key(conversation_id, "conversation id")
        }
    }
}
