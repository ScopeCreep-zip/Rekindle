//! The apply half (plan E7.7.2 "`skin://` scheme", "Apply"): `SkinGet`
//! answers what a window applies, `SkinFile` the bytes of one `skin://`
//! URL, and every change of the resolved skin is announced as
//! `SystemEvent::SkinChanged`.
//!
//! Resolution: the identity's choice while unlocked, else the device
//! login skin (whose own default is the built-in default). Before unlock
//! only the built-ins and the device login copy are reachable; a library
//! package needs the identity's database.

use rekindle_db::repo::{preferences, skins};
use rekindle_ipc::protocol::IpcResponse;
use rekindle_skin::builtin;
use rekindle_skin::serve::{self, Cache};
use rekindle_types::dto::skin_library::{SkinApplied, SkinFrames, SkinServed};
use rekindle_types::subscription_events::{SubscriptionEvent, SystemEvent};

use super::ids::{self, DEV, PKG};
use super::{failed, helper_opts, login, not_found, Answer};
use crate::daemon::dispatch::DaemonContext;

/// A canonical package and its address.
struct Held {
    address: String,
    bytes: Vec<u8>,
    cache: Cache,
}

/// The built-in `id`'s helper-validated canonical package.
async fn builtin_package(ctx: &DaemonContext, id: &str) -> Answer<Held> {
    let opts = helper_opts(ctx)?;
    let owned = id.to_owned();
    let canonical = rekindle_lifecycle::run_blocking(move || builtin::validated(&owned, &opts))
        .await
        .map_err(|e| failed("skin helper task", e))?
        .map_err(|e| failed("built-in skin not validated", e))?
        .ok_or_else(|| not_found(id))?;
    Ok(Held {
        address: canonical.address(),
        bytes: canonical.bytes,
        cache: Cache::Immutable,
    })
}

/// The device login copy at `hash`, if it is on disk and is what its name
/// says (the file sits outside the encrypted store).
fn login_copy(ctx: &DaemonContext, hash: &str) -> Answer<Option<Held>> {
    if !ids::is_hash(hash) {
        return Ok(None);
    }
    let path = ctx.root.skins().join(format!("{hash}.rkskin"));
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(failed("login skin", e)),
    };
    if blake3::hash(&bytes).to_hex().as_str() != hash {
        return Err(failed("login skin", "the copy does not match its address"));
    }
    Ok(Some(Held {
        address: hash.to_owned(),
        bytes,
        cache: Cache::NoStore,
    }))
}

/// The library package at `hash`, when unlocked and held.
async fn library_package(ctx: &DaemonContext, hash: &str) -> Answer<Option<Held>> {
    let (Ok(db), Ok(owner)) = (ctx.require_db(), ctx.require_owner()) else {
        return Ok(None);
    };
    let key = hash.to_owned();
    let bytes = db
        .call(move |conn| skins::bytes(conn, &owner, &key))
        .await
        .map_err(|e| failed("skin unreadable", e))?;
    Ok(bytes.map(|bytes| Held {
        address: hash.to_owned(),
        bytes,
        cache: Cache::NoStore,
    }))
}

/// The skin this device shows now: the library skin on trial, else the
/// stored choice, else (locked) the device login skin.
async fn resolved(ctx: &DaemonContext) -> Answer<String> {
    if let (Ok(db), Ok(owner)) = (ctx.require_db(), ctx.require_owner()) {
        if let Some(trial) = ctx.skin.trial.current(&owner) {
            return Ok(trial.id);
        }
        return db
            .call(move |conn| preferences::skin_choice(conn, &owner))
            .await
            .map(|c| c.skin_id)
            .map_err(|e| failed("skin choice unreadable", e));
    }
    login::read(&ctx.root).map_err(|e| failed("login skin", e))
}

/// The package a skin id names.
async fn package_of(ctx: &DaemonContext, id: &str) -> Answer<Held> {
    if id.starts_with(builtin::PREFIX) {
        return builtin_package(ctx, id).await;
    }
    let held = if let Some(hash) = id.strip_prefix(DEV) {
        login_copy(ctx, hash)?
    } else if let Some(hash) = id.strip_prefix(PKG) {
        library_package(ctx, hash).await?
    } else {
        None
    };
    held.ok_or_else(|| not_found(id))
}

/// The canonical package of `id`, or of the skin shown now (the
/// editor's "Start from the current skin").
/// Answers the id with the bytes.
pub(super) async fn package_bytes(
    ctx: &DaemonContext,
    id: Option<String>,
) -> Answer<(String, Vec<u8>)> {
    let id = match id {
        Some(id) => id,
        None => resolved(ctx).await?,
    };
    let bytes = package_of(ctx, &id).await?.bytes;
    Ok((id, bytes))
}

/// `SkinGet`: what a window applies for `id`, or for the resolution, whose
/// role-tag grounds the Host records ([`super::shown`]).
pub(super) async fn get(ctx: &DaemonContext, id: Option<String>) -> Answer<SkinApplied> {
    let shown = id.is_none();
    let id = match id {
        Some(id) => id,
        None => resolved(ctx).await?,
    };
    let held = package_of(ctx, &id).await?;
    if shown {
        let grounds = rekindle_skin::role_tag::SkinGrounds::of_package(&held.bytes)
            .map_err(|e| failed("skin grounds", format!("{e:?}")))?;
        super::shown::note(ctx, grounds);
    }
    // The stylesheet measures the skin's art (its label grounds): CPU work
    // kept off the Host's async workers, as every window asks at once.
    let (address, applied_address) = (held.address.clone(), held.address.clone());
    let applied =
        rekindle_lifecycle::run_blocking(move || serve::stylesheet(&held.bytes, &address))
            .await
            .map_err(|e| failed("skin stylesheet task", e))?
            .map_err(|e| failed("skin stylesheet", e))?;
    Ok(SkinApplied {
        id,
        base: applied.base,
        tokens_href: serve::tokens_href(&applied_address),
        regions: applied.regions,
        images: applied.images,
        palette: applied.palette,
        shells: applied.shells,
    })
}

/// `SkinFrames`: the shown skin's window frames, each tile's mask decoded
/// from the helper-validated package off the async workers (plan SK0 S3).
pub(super) async fn frames(ctx: &DaemonContext) -> Answer<SkinFrames> {
    let id = resolved(ctx).await?;
    let held = package_of(ctx, &id).await?;
    let windows = rekindle_lifecycle::run_blocking(move || serve::frames(&held.bytes))
        .await
        .map_err(|e| failed("skin frames task", e))?
        .map_err(|e| failed("skin frames", e))?;
    Ok(SkinFrames { id, windows })
}

/// `SkinFile`: one file of the package a `skin://` path names. Every
/// refusal is logged at WARN with the path and the refusal (its status and
/// cause), so a window's failed image or sheet names itself in the Host's
/// log (plan SK2-T1); the shell's `skin://` handler only copies the answer.
pub(super) async fn file(ctx: &DaemonContext, path: &str) -> Answer<SkinServed> {
    let answer = file_answer(ctx, path).await;
    if let Err(refusal) = &answer {
        tracing::warn!(path, ?refusal, "skin:// file not served");
    }
    answer
}

async fn file_answer(ctx: &DaemonContext, path: &str) -> Answer<SkinServed> {
    let missing = || IpcResponse::error(404, "no such skin file");
    let (address, _) = serve::parse_path(path).ok_or_else(missing)?;
    // A built-in is found by its address and only that pack is validated.
    let mut held = match builtin::by_address(address) {
        Some((_, pack)) => {
            Some(builtin_package(ctx, &format!("{}{}", builtin::PREFIX, pack.id)).await?)
        }
        None => None,
    };
    if held.is_none() {
        held = login_copy(ctx, address)?;
    }
    if held.is_none() {
        held = library_package(ctx, address).await?;
    }
    let held = held.ok_or_else(missing)?;
    // A `tokens.css` body is the stylesheet: off the async workers too.
    let path = path.to_owned();
    let body = rekindle_lifecycle::run_blocking(move || {
        let (_, file) = serve::parse_path(&path).ok_or(None)?;
        serve::body(&held.bytes, &held.address, file).map_err(Some)
    })
    .await
    .map_err(|e| failed("skin file task", e))?
    .map_err(|e| e.map_or_else(missing, |e| failed("skin file", e)))?
    .ok_or_else(missing)?;
    Ok(SkinServed {
        content_type: body.content_type.to_owned(),
        cache_control: held.cache.header().to_owned(),
        bytes: body.bytes,
    })
}

/// Tell every window the skin it shows now.
pub(super) async fn announce(ctx: &DaemonContext) -> Answer<()> {
    let skin = get(ctx, None).await?;
    ctx.publish_event(SubscriptionEvent::System(SystemEvent::SkinChanged { skin }));
    Ok(())
}
