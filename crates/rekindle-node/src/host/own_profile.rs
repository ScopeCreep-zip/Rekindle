//! Our own profile (plan E1.1): the name we show and our avatar. Each change
//! is saved in the identity's database (the name in the registry too),
//! published to our profile record and announced as
//! `PresenceEvent::SelfProfileChanged`.

use std::io::Cursor;

use rekindle_types::dht_layout::profile as subkeys;
use rekindle_types::subscription_events::{PresenceEvent, SubscriptionEvent};

use crate::daemon::dispatch::DaemonContext;

/// The longest side of an avatar, px.
const AVATAR_MAX_DIM: u32 = 128;

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

/// Make `image` our avatar: a WebP of at most 128 px a side.
///
/// # Errors
/// The image does not decode, its WebP is over the profile's subkey cap,
/// or the database refused it.
pub(crate) async fn set_avatar(ctx: &DaemonContext, image: Vec<u8>) -> Result<(), String> {
    let owner = ctx
        .require_owner()
        .map_err(|_| "not unlocked".to_string())?;
    let webp = tokio::task::spawn_blocking(move || compress_avatar(&image))
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

/// Decode any supported image and re-encode it as a WebP of at most
/// [`AVATAR_MAX_DIM`] a side. CPU-bound: run off the executor.
fn compress_avatar(raw: &[u8]) -> Result<Vec<u8>, String> {
    let img = image::ImageReader::new(Cursor::new(raw))
        .with_guessed_format()
        .map_err(|e| format!("image format: {e}"))?
        .decode()
        .map_err(|e| format!("image decode: {e}"))?;
    let resized = if img.width() > AVATAR_MAX_DIM || img.height() > AVATAR_MAX_DIM {
        img.resize(
            AVATAR_MAX_DIM,
            AVATAR_MAX_DIM,
            image::imageops::FilterType::Lanczos3,
        )
    } else {
        img
    };
    let mut webp = Vec::new();
    resized
        .write_to(&mut Cursor::new(&mut webp), image::ImageFormat::WebP)
        .map_err(|e| format!("WebP encode: {e}"))?;
    Ok(webp)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A large picture becomes a small WebP that fits the profile.
    #[test]
    fn an_avatar_is_a_small_webp() {
        let picture = image::RgbImage::from_fn(640, 480, |x, y| {
            let byte = |v: u32| u8::try_from(v % 256).unwrap_or(0);
            image::Rgb([byte(x), byte(y), 7])
        });
        let mut png = Vec::new();
        image::DynamicImage::ImageRgb8(picture)
            .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let webp = compress_avatar(&png).unwrap();
        assert!(webp.len() <= PROFILE_SUBKEY_CAP);
        let back = image::load_from_memory_with_format(&webp, image::ImageFormat::WebP).unwrap();
        assert!(back.width() <= AVATAR_MAX_DIM && back.height() <= AVATAR_MAX_DIM);
        assert!(compress_avatar(b"not an image").is_err());
    }
}
