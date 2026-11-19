//! The panic shortcut through the `org.freedesktop.portal.GlobalShortcuts`
//! portal (plan E7.1), the only global-shortcut delivery a Wayland session
//! has (`rekindle_node::host::panic_shortcut`). The calls follow the
//! portal's spec (flatpak.github.io/xdg-desktop-portal, GlobalShortcuts
//! v1/v2): `CreateSession`, then `BindShortcuts`, which shows the
//! compositor's dialog where the user approves (or changes) the binding;
//! each answers through a `Request` object's `Response` signal. The bound
//! shortcut's `trigger_description` is what the user is shown, since the
//! compositor may bind another trigger than the one Rekindle prefers.
//! `Activated` fires the wipe; closing the session releases the binding.
//!
//! Compiled on every platform so every gate type-checks it; only a Wayland
//! session routes here (`panic_shortcut::route`).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};

use futures::StreamExt;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const DESTINATION: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const INTERFACE: &str = "org.freedesktop.portal.GlobalShortcuts";
/// The one shortcut Rekindle binds.
const SHORTCUT_ID: &str = "panic-wipe";

/// A bound portal session: the connection it lives on, its handle, and the
/// trigger the compositor reported.
pub struct PortalBinding {
    connection: zbus::Connection,
    session: OwnedObjectPath,
    /// The compositor's `trigger_description` of the bound shortcut.
    pub trigger: String,
}

/// Whether the GlobalShortcuts portal answers on the session bus (its
/// `version` property reads).
pub async fn offered() -> bool {
    let Ok(connection) = zbus::Connection::session().await else {
        return false;
    };
    let Ok(proxy) = portal(&connection).await else {
        return false;
    };
    proxy.get_property::<u32>("version").await.is_ok()
}

/// Bind the panic shortcut: `accelerator` (the stored chord) is the
/// preferred trigger; the user approves the binding in the compositor's
/// dialog.
///
/// # Errors
/// The portal is unreachable, the user cancelled the dialog, or nothing
/// was bound.
pub async fn bind(accelerator: &str) -> Result<PortalBinding, String> {
    let preferred = xdg_trigger(accelerator)?;
    let connection = zbus::Connection::session()
        .await
        .map_err(|e| format!("the session bus is unreachable: {e}"))?;
    let proxy = portal(&connection).await.map_err(|e| e.to_string())?;
    let token = next_token();
    let created = request(&connection, &token, || async {
        let options = HashMap::from([
            ("handle_token", Value::from(token.as_str())),
            ("session_handle_token", Value::from(token.as_str())),
        ]);
        proxy
            .call_method("CreateSession", &(options,))
            .await
            .map(drop)
    })
    .await?;
    let session = created
        .get("session_handle")
        .and_then(|v| String::try_from(v.clone()).ok())
        .and_then(|s| OwnedObjectPath::try_from(s).ok())
        .ok_or("the portal answered no session")?;
    let bind_token = next_token();
    let bound = request(&connection, &bind_token, || async {
        let shortcut = HashMap::from([
            (
                "description",
                Value::from("Rekindle quick wipe (panic shortcut)"),
            ),
            ("preferred_trigger", Value::from(preferred.as_str())),
        ]);
        let options = HashMap::from([("handle_token", Value::from(bind_token.as_str()))]);
        proxy
            .call_method(
                "BindShortcuts",
                &(&session, vec![(SHORTCUT_ID, shortcut)], "", options),
            )
            .await
            .map(drop)
    })
    .await;
    let binding = PortalBinding {
        connection,
        session,
        trigger: String::new(),
    };
    let trigger = match bound.and_then(|results| bound_trigger(&results)) {
        Ok(trigger) => trigger,
        Err(reason) => {
            binding.close().await;
            return Err(reason);
        }
    };
    Ok(PortalBinding { trigger, ..binding })
}

impl PortalBinding {
    /// Wait for the next press of the bound shortcut. `None` when the
    /// portal's signal stream ended (the bus closed).
    ///
    /// # Errors
    /// The `Activated` signal could not be subscribed to.
    pub async fn activations(
        &self,
    ) -> Result<impl futures::Stream<Item = ()> + Send + 'static, String> {
        let proxy = portal(&self.connection).await.map_err(|e| e.to_string())?;
        let stream = proxy
            .receive_signal("Activated")
            .await
            .map_err(|e| format!("the portal's activations are unreadable: {e}"))?;
        let session = self.session.clone();
        Ok(stream.filter_map(move |message| {
            let ours = message
                .body()
                .deserialize::<(OwnedObjectPath, String, u64, HashMap<String, OwnedValue>)>()
                .is_ok_and(|(s, id, _, _)| s == session && id == SHORTCUT_ID);
            std::future::ready(ours.then_some(()))
        }))
    }

    /// Release the binding (`org.freedesktop.portal.Session.Close`).
    pub async fn close(self) {
        let closed = zbus::Proxy::new(
            &self.connection,
            DESTINATION,
            self.session.as_ref(),
            "org.freedesktop.portal.Session",
        )
        .await;
        let result = match closed {
            Ok(proxy) => proxy.call_method("Close", &()).await.map(drop),
            Err(e) => Err(e),
        };
        if let Err(e) = result {
            tracing::warn!(error = %e, "the portal's panic shortcut session stays open");
        }
    }
}

async fn portal(connection: &zbus::Connection) -> zbus::Result<zbus::Proxy<'static>> {
    zbus::Proxy::new(connection, DESTINATION, PATH, INTERFACE).await
}

fn next_token() -> String {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    format!(
        "rekindle{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// Run one portal request: subscribe to its `Response` (the request path is
/// known from our unique name and `token`, so no answer is missed), make
/// the call, and return the results of a successful response.
async fn request<F, Fut>(
    connection: &zbus::Connection,
    token: &str,
    call: F,
) -> Result<HashMap<String, OwnedValue>, String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = zbus::Result<()>>,
{
    let sender = connection
        .unique_name()
        .ok_or("the session bus gave no name")?
        .trim_start_matches(':')
        .replace('.', "_");
    let path = format!("{PATH}/request/{sender}/{token}");
    let proxy = zbus::Proxy::new(
        connection,
        DESTINATION,
        path.as_str(),
        "org.freedesktop.portal.Request",
    )
    .await
    .map_err(|e| e.to_string())?;
    let mut responses = proxy
        .receive_signal("Response")
        .await
        .map_err(|e| e.to_string())?;
    call()
        .await
        .map_err(|e| format!("the portal refused the request: {e}"))?;
    let message = responses.next().await.ok_or("the portal never answered")?;
    let (code, results) = message
        .body()
        .deserialize::<(u32, HashMap<String, OwnedValue>)>()
        .map_err(|e| format!("the portal's answer is unreadable: {e}"))?;
    match code {
        0 => Ok(results),
        1 => Err("the shortcut was not approved".into()),
        _ => Err("the desktop could not bind the shortcut".into()),
    }
}

/// The bound panic shortcut's trigger description from `BindShortcuts`'
/// results.
fn bound_trigger(results: &HashMap<String, OwnedValue>) -> Result<String, String> {
    let shortcuts: Vec<(String, HashMap<String, OwnedValue>)> = results
        .get("shortcuts")
        .and_then(|v| v.try_clone().ok())
        .and_then(|v| v.try_into().ok())
        .ok_or("the desktop bound no shortcut")?;
    shortcuts
        .into_iter()
        .find(|(id, _)| id == SHORTCUT_ID)
        .and_then(|(_, fields)| fields.get("trigger_description").cloned())
        .and_then(|v| String::try_from(v).ok())
        .filter(|t| !t.is_empty())
        .ok_or_else(|| "the desktop bound the shortcut to no trigger".into())
}

/// The stored accelerator (`CommandOrControl+Shift+F12`) as the XDG
/// shortcuts specification's trigger (`CTRL+SHIFT+F12`): the portal's
/// `preferred_trigger`.
///
/// # Errors
/// A modifier the specification has no name for, or no key.
pub fn xdg_trigger(accelerator: &str) -> Result<String, String> {
    let mut parts: Vec<String> = Vec::new();
    let mut tokens = accelerator.split('+').map(str::trim).peekable();
    while let Some(token) = tokens.next() {
        if tokens.peek().is_none() {
            parts.push(xdg_key(token)?);
            break;
        }
        let modifier = match token.to_ascii_lowercase().as_str() {
            "commandorcontrol" | "cmdorctrl" | "commandorctrl" | "cmdorcontrol" | "control"
            | "ctrl" => "CTRL",
            "shift" => "SHIFT",
            "alt" | "option" => "ALT",
            "super" | "cmd" | "command" | "meta" => "LOGO",
            _ => return Err(format!("`{token}` is not a modifier")),
        };
        parts.push(modifier.into());
    }
    Ok(parts.join("+"))
}

fn xdg_key(token: &str) -> Result<String, String> {
    let key = token
        .strip_prefix("Key")
        .filter(|k| k.len() == 1)
        .or_else(|| token.strip_prefix("Digit").filter(|k| k.len() == 1))
        .unwrap_or(token);
    if key.is_empty() {
        return Err("the shortcut has no key".into());
    }
    Ok(if key.len() == 1 {
        key.to_ascii_lowercase()
    } else {
        key.to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The portal's preferred trigger is the stored chord in the XDG
    /// shortcuts specification's names.
    #[test]
    fn the_accelerator_becomes_the_xdg_trigger() {
        assert_eq!(
            xdg_trigger("CommandOrControl+Shift+F12").unwrap(),
            "CTRL+SHIFT+F12"
        );
        assert_eq!(xdg_trigger("Super+Alt+KeyW").unwrap(), "LOGO+ALT+w");
        assert_eq!(xdg_trigger("Ctrl+Shift+Digit9").unwrap(), "CTRL+SHIFT+9");
        assert_eq!(xdg_trigger("Ctrl+Q").unwrap(), "CTRL+q");
        assert!(xdg_trigger("Hyper+F1").is_err());
        assert!(xdg_trigger("Ctrl+").is_err());
    }

    /// The compositor's description of what it bound is what the user is
    /// shown; an answer without our shortcut binds nothing.
    #[test]
    fn the_bound_trigger_is_the_compositors_description() {
        let fields = HashMap::from([(
            "trigger_description".to_string(),
            OwnedValue::from(zbus::zvariant::Str::from("Ctrl+Shift+F12")),
        )]);
        let shortcuts = vec![(SHORTCUT_ID.to_string(), fields)];
        let results = HashMap::from([(
            "shortcuts".to_string(),
            OwnedValue::try_from(Value::from(shortcuts)).unwrap(),
        )]);
        assert_eq!(bound_trigger(&results).unwrap(), "Ctrl+Shift+F12");
        assert!(bound_trigger(&HashMap::new()).is_err());
    }
}
