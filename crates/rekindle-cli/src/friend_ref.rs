//! A friend named on the command line (live finding AB2): their public
//! key, or the exact display name or nickname of one friend on the Host's
//! friend list (`IpcRequest::FriendList`). The Host takes only keys, so a
//! name is turned into its friend's key here; a name no friend has, or one
//! several friends share, is an error naming the matches, never a guess
//! (threat model: spoofable display fields). Key-only flags (block, verify,
//! safety number, session reset, requests) do not come through here.

use rekindle_client::DaemonClient;
use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::dto::friends::Friend;

use crate::helpers;

/// Hex characters in an identity public key.
const KEY_HEX_LEN: usize = 64;

/// Whether `given` is written as an identity public key (64 hex).
fn is_key(given: &str) -> bool {
    given.len() == KEY_HEX_LEN && given.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The key of the one friend in `friends` that `given` names: a friend's
/// key, or the exact display name or nickname of exactly one friend.
///
/// # Errors
/// No friend has that name, or more than one does (each is listed).
pub fn pick(friends: &[Friend], given: &str) -> anyhow::Result<String> {
    if let Some(friend) = friends.iter().find(|f| f.public_key == given) {
        return Ok(friend.public_key.clone());
    }
    let named: Vec<&Friend> = friends
        .iter()
        .filter(|f| f.display_name == given || f.nickname.as_deref() == Some(given))
        .collect();
    match named.as_slice() {
        [one] => Ok(one.public_key.clone()),
        [] => anyhow::bail!(
            "no friend is named \"{}\": give their display name, nickname or public key \
             (`rekindle friend list`)",
            rekindle_utils::text::sanitize_for_display(given)
        ),
        many => {
            let matches: Vec<String> = many.iter().map(|f| describe(f)).collect();
            anyhow::bail!(
                "{} friends are named \"{}\": {}; give the public key",
                many.len(),
                rekindle_utils::text::sanitize_for_display(given),
                matches.join(", ")
            )
        }
    }
}

/// A matching friend as an ambiguity error lists them: name, nickname and
/// full key (the key is what the user gives next).
fn describe(friend: &Friend) -> String {
    let name = rekindle_utils::text::sanitize_for_display(&friend.display_name);
    match friend.nickname.as_deref() {
        Some(nick) => format!(
            "{name} (\"{}\") {}",
            rekindle_utils::text::sanitize_for_display(nick),
            friend.public_key
        ),
        None => format!("{name} {}", friend.public_key),
    }
}

/// The Host's friend list.
async fn friends(client: &DaemonClient) -> anyhow::Result<Vec<Friend>> {
    let value = helpers::send(client, IpcRequest::FriendList).await?;
    Ok(serde_json::from_value(value)?)
}

/// The key `given` names ([`pick`]); a key is taken as given without
/// asking the Host, which judges it.
///
/// # Errors
/// The friend list could not be read, or [`pick`] refused the name.
pub async fn resolve(client: &DaemonClient, given: &str) -> anyhow::Result<String> {
    Ok(resolve_all(client, std::slice::from_ref(&given.to_owned()))
        .await?
        .remove(0))
}

/// The keys `given` names, in order, reading the friend list at most once.
///
/// # Errors
/// As [`resolve`], for any one of them.
pub async fn resolve_all(client: &DaemonClient, given: &[String]) -> anyhow::Result<Vec<String>> {
    if given.iter().all(|g| is_key(g)) {
        return Ok(given.to_vec());
    }
    let list = friends(client).await?;
    given
        .iter()
        .map(|g| {
            if is_key(g) {
                Ok(g.clone())
            } else {
                pick(&list, g)
            }
        })
        .collect()
}

/// [`resolve`] for an optional flag.
///
/// # Errors
/// As [`resolve`].
pub async fn resolve_opt(
    client: &DaemonClient,
    given: Option<&str>,
) -> anyhow::Result<Option<String>> {
    match given {
        Some(g) => Ok(Some(resolve(client, g).await?)),
        None => Ok(None),
    }
}

#[cfg(test)]
#[path = "friend_ref_tests.rs"]
mod tests;
