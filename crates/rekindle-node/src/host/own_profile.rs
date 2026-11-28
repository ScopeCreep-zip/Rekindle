//! Our own profile (plan E1.1): the name we show and our avatar. Each change
//! is saved in the identity's database (the name in the registry too),
//! published to our profile record and announced as
//! `PresenceEvent::SelfProfileChanged`.

use rekindle_image::{DecodeLimits, Frames};
use rekindle_types::dht_layout::profile as subkeys;
use rekindle_types::image_kind::ImageKind;
use rekindle_types::subscription_events::{PresenceEvent, SubscriptionEvent};

use crate::daemon::dispatch::DaemonContext;

/// The longest side of an avatar, px.
const AVATAR_MAX_DIM: u32 = 128;

/// What a picture offered as an avatar may cost to decode (plan E7.11):
/// sides up to 8192 px and 40 Mpx (an 8K frame, a 24 Mpx camera's photo),
/// 192 MiB held (40 Mpx of RGBA8 is 160 MB). An animated GIF gives its
/// first frame.
pub(crate) const AVATAR_SOURCE_LIMITS: DecodeLimits = DecodeLimits {
    max_width: 8192,
    max_height: 8192,
    max_pixels: 40_000_000,
    max_alloc: 192 * 1024 * 1024,
    frames: Frames::First,
};

/// What one profile subkey holds at most: `DFLT(11)`'s share of a record,
/// the cap the port reports for it (`RecordPort::subkey_cap`).
const PROFILE_SUBKEY_CAP: usize =
    rekindle_records::lease::max_subkey_bytes(rekindle_records::lease::SchemaShape {
        subkey_count: subkeys::SUBKEY_COUNT as u32,
    });

/// Show `display_name` (already a valid display name).
///
/// # Errors
/// No identity is unlocked, or the database or registry refused it. A
/// failed profile write is logged: the name is saved and shown, and the
/// own-records stage republishes it at the next unlock.
pub(crate) async fn set_nickname(ctx: &DaemonContext, display_name: &str) -> Result<(), String> {
    let owner = ctx
        .require_owner()
        .map_err(|_| "not unlocked".to_string())?;
    let db = ctx.db.current().map_err(|e| e.to_string())?;
    let (pk, name) = (owner.clone(), display_name.to_owned());
    db.call(move |conn| rekindle_db::repo::identity::set_display_name(conn, &pk, &name))
        .await
        .map_err(|e| format!("identity database: {e}"))?;
    super::identity::rename(&ctx.root, &owner, display_name)
        .map_err(|e| format!("identity registry: {e}"))?;
    if let Some(session) = ctx.session.write().as_mut() {
        display_name.clone_into(&mut session.identity.display_name);
    }
    announce(ctx, owner, Some(display_name.to_owned()), false);
    if let Err(e) = ctx
        .own_presence
        .write_profile(subkeys::DISPLAY_NAME, display_name.as_bytes().to_vec())
        .await
    {
        tracing::warn!(error = %e, "new display name not published");
    }
    Ok(())
}

/// Make `image`, declared to be `kind`, our avatar: a WebP of at most
/// 128 px a side.
///
/// # Errors
/// The image is not `kind`, is over [`AVATAR_SOURCE_LIMITS`] or does not
/// decode; its WebP is over the profile's subkey cap; or the database
/// refused it.
pub(crate) async fn set_avatar(
    ctx: &DaemonContext,
    image: Vec<u8>,
    kind: ImageKind,
) -> Result<(), String> {
    let owner = ctx
        .require_owner()
        .map_err(|_| "not unlocked".to_string())?;
    let webp = rekindle_lifecycle::run_blocking(move || compress_avatar(&image, kind))
        .await
        .map_err(|e| e.to_string())??;
    if webp.len() > PROFILE_SUBKEY_CAP {
        return Err(format!(
            "the avatar is {} bytes as WebP; a profile holds {PROFILE_SUBKEY_CAP}",
            webp.len()
        ));
    }
    let db = ctx.db.current().map_err(|e| e.to_string())?;
    let (pk, saved) = (owner.clone(), webp.clone());
    db.call(move |conn| rekindle_db::repo::identity::set_avatar(conn, &pk, &saved))
        .await
        .map_err(|e| format!("identity database: {e}"))?;
    announce(ctx, owner, None, true);
    if let Err(e) = ctx.own_presence.write_profile(subkeys::AVATAR, webp).await {
        tracing::warn!(error = %e, "new avatar not published");
    }
    Ok(())
}

/// Our own avatar or a friend's, as WebP bytes.
///
/// # Errors
/// No identity is unlocked, or the database refused the read.
pub(crate) async fn avatar(
    ctx: &DaemonContext,
    public_key: &str,
) -> Result<Option<Vec<u8>>, String> {
    let owner = ctx
        .require_owner()
        .map_err(|_| "not unlocked".to_string())?;
    let db = ctx.db.current().map_err(|e| e.to_string())?;
    let key = public_key.to_owned();
    db.call(move |conn| {
        if let Some(own) = rekindle_db::repo::identity::avatar(conn, &key)? {
            return Ok(Some(own));
        }
        rekindle_db::repo::friends::avatar(conn, &owner, &key)
    })
    .await
    .map_err(|e| format!("identity database: {e}"))
}

fn announce(ctx: &DaemonContext, public_key: String, display_name: Option<String>, avatar: bool) {
    ctx.publish_event(SubscriptionEvent::Presence(
        PresenceEvent::SelfProfileChanged {
            public_key,
            display_name,
            avatar_changed: avatar,
        },
    ));
}

/// Decode `raw` as `kind` only, under [`AVATAR_SOURCE_LIMITS`], and
/// re-encode it as a WebP of at most [`AVATAR_MAX_DIM`] a side.
/// CPU-bound: run off the executor.
fn compress_avatar(raw: &[u8], kind: ImageKind) -> Result<Vec<u8>, String> {
    let picture = rekindle_image::decode_limited(raw, kind, &AVATAR_SOURCE_LIMITS)
        .map_err(|e| e.to_string())?
        .into_first();
    rekindle_image::encode_webp(&rekindle_image::fit_within(picture, AVATAR_MAX_DIM))
        .map_err(|e| e.to_string())
}

#[cfg(test)]
#[path = "own_profile_tests.rs"]
mod tests;
