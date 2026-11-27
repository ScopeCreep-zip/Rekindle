//! The skin requests (plan E7.7.2 "Host"): the identity's library, its
//! skin choice and the device login skin, one implementation on both
//! hosts. A package enters the library only as the canonical package
//! `rekindle-skin-helper` returns; an id that names no built-in and no
//! library row is refused.
//!
//! `SkinGet`, `SkinFile` (the `skin://` scheme's bytes) and
//! `SystemEvent::SkinChanged` are the serving-and-apply half ([`serve`]).

pub(crate) mod actions;
#[cfg(test)]
mod actions_tests;
#[cfg(test)]
mod call_fixtures;
pub(crate) mod capability;
#[cfg(test)]
mod capability_tests;
pub(crate) mod community;
#[cfg(test)]
mod community_row_tests;
#[cfg(test)]
mod community_tests;
mod draft;
pub(crate) mod failure;
pub(crate) mod ids;
mod launchers;
#[cfg(test)]
mod leave_tests;
pub(crate) mod login;
#[cfg(test)]
mod popup_selection_tests;
pub(crate) mod press_trace;
pub(crate) mod reads;
pub(crate) mod rooms;
mod serve;
mod shown;
pub(crate) mod signals;
#[cfg(test)]
mod signals_tests;
mod trial;
mod world;

use rekindle_db::repo::{preferences, skins};
use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_skin::helper::{self, HelperError, HelperOpts};
use rekindle_skin::{builtin, conform, manifest, package};
use rekindle_types::dto::skin::SkinManifest;
use rekindle_types::dto::skin_library::{SkinEntry, SkinLibrary};

use crate::daemon::dispatch::{state_error, DaemonContext};
use crate::daemon::DaemonState;
use ids::{SkinId, PKG};

pub(crate) use ids::check;
pub(crate) use shown::ShownGrounds;
pub(crate) use trial::TrialSlot;

/// The Host's skin state, one field of the context.
#[derive(Default)]
pub(crate) struct SkinState {
    /// Where `rekindle-skin-helper` is (plan E7.7.2): beside this
    /// executable, found on the first skin import.
    pub(crate) helper: std::sync::OnceLock<HelperOpts>,
    /// The library skin on trial ("Keep this skin?", plan E7.7.2), which
    /// the Host reverts unless it is kept.
    pub(crate) trial: TrialSlot,
    /// The role-tag grounds of the skin this device shows, so role colours
    /// read on an imported skin too (`shown`).
    pub(crate) grounds: ShownGrounds,
    /// Each skin window's selection (plan SK2-A4).
    pub(crate) selection: actions::Selections,
    /// Each open popup's launcher (plan SK2-N4).
    pub(crate) launchers: actions::Launchers,
    /// Each skin window's current view (SK2-G §8, `SearchFor`).
    pub(crate) views: actions::Views,
    /// Each skin window's state signals (plan SK2-A4 step 3).
    pub(crate) signals: signals::SignalState,
    /// The apply failures already told, so N windows failing one skin the
    /// same way publish one `SkinApplyFailed` (`failure`).
    pub(crate) failures: failure::ToldFailures,
}

/// Route one skin request (the router's arm for them).
pub(crate) async fn route(
    ctx: &DaemonContext,
    state: DaemonState,
    request: IpcRequest,
) -> IpcResponse {
    let reads = matches!(
        request,
        IpcRequest::SkinList
            | IpcRequest::SkinGet { .. }
            | IpcRequest::SkinFile { .. }
            | IpcRequest::SkinFrames
    );
    // A skin write is local: the identity's database and this device's
    // data root, with no record written to the network. So it needs only
    // an unlocked identity, and a detached network refuses none (walk 24:
    // ST9 and ST10, sent while the node was still attaching, were refused
    // "cannot perform write operation in state 'detached'").
    if !reads && !state.can_query() {
        return state_error(state, "write");
    }
    let answer = match request {
        IpcRequest::SkinList => list(ctx).await.map(|l| IpcResponse::ok(&l)),
        IpcRequest::SkinGet { id } => serve::get(ctx, id).await.map(|a| IpcResponse::ok(&a)),
        IpcRequest::SkinFile { path } => serve::file(ctx, &path).await.map(|f| IpcResponse::ok(&f)),
        IpcRequest::SkinFrames => serve::frames(ctx).await.map(|f| IpcResponse::ok(&f)),
        IpcRequest::SkinImport { bytes } => import(ctx, bytes).await.map(|e| IpcResponse::ok(&e)),
        IpcRequest::SkinRemove { id } => remove(ctx, id).await,
        IpcRequest::SkinSelect { id } => select(ctx, id).await,
        IpcRequest::SkinSetLogin { id } => set_login(ctx, id).await,
        IpcRequest::SkinKeep { id, keep } => match trial::keep(ctx, &id, keep).await {
            Ok(()) => list(ctx).await.map(|l| IpcResponse::ok(&l)),
            Err(refused) => Err(refused),
        },
        IpcRequest::SkinValidateDraft { draft } => {
            draft::validate(draft).await.map(|r| IpcResponse::ok(&r))
        }
        IpcRequest::SkinDraftStart { id } => {
            draft::start(ctx, id).await.map(|d| IpcResponse::ok(&d))
        }
        IpcRequest::SkinExportDraft { draft, path } => draft::export(ctx, draft, path)
            .await
            .map(|a| IpcResponse::ok(&a)),
        IpcRequest::SkinDraftCheck { draft } => draft::check(ctx, draft)
            .await
            .map(|c| IpcResponse::ok(&c.address())),
        IpcRequest::SkinExportSchema { path } => draft::export_schema(path)
            .await
            .map(|()| IpcResponse::ok(&())),
        other => unreachable!("{} is not a skin request", other.name()),
    };
    answer.unwrap_or_else(|refused| refused)
}

type Answer<T> = Result<T, IpcResponse>;

fn not_found(id: &str) -> IpcResponse {
    IpcResponse::error(
        404,
        format!("no skin {id}: no built-in and no library package"),
    )
}

fn failed(what: &str, e: impl std::fmt::Display) -> IpcResponse {
    IpcResponse::error(500, format!("{what}: {e}"))
}

/// The helper's location: beside this executable, found on first use.
fn helper_opts(ctx: &DaemonContext) -> Answer<HelperOpts> {
    if let Some(opts) = ctx.skin.helper.get() {
        return Ok(opts.clone());
    }
    let opts = HelperOpts::beside_current_exe().map_err(|e| failed("skin helper", e))?;
    Ok(ctx.skin.helper.get_or_init(|| opts).clone())
}

/// A package's manifest; every package here passed the pipeline.
fn manifest_of(bytes: &[u8]) -> Answer<SkinManifest> {
    package::read(bytes)
        .and_then(|p| manifest::parse(&p))
        .map_err(|e| failed("skin manifest unreadable", e))
}

/// `SkinImport`: the helper validates, the canonical package is stored.
async fn import(ctx: &DaemonContext, bytes: Vec<u8>) -> Answer<SkinEntry> {
    let (db, owner) = (ctx.require_db()?, ctx.require_owner()?);
    let opts = helper_opts(ctx)?;
    let (canonical, shorts) = rekindle_lifecycle::run_blocking(move || {
        helper::validate(&bytes, &opts).map(|c| {
            let shorts = conform::short_targets_of(&c.bytes);
            (c, shorts)
        })
    })
    .await
    .map_err(|e| failed("skin helper task", e))?
    .map_err(|e| match e {
        HelperError::Refused(rule) => IpcResponse::error(400, format!("skin refused: {rule}")),
        other => failed("skin not validated", other),
    })?;
    let short_targets = shorts.map_err(|e| failed("skin targets unreadable", e))?;
    let m = manifest_of(&canonical.bytes)?;
    let manifest_json = serde_json::to_string(&m).map_err(|e| failed("skin manifest", e))?;
    let hash = canonical.address();
    let imported_at = ctx.clock.now_ms_i64();
    let stored = hash.clone();
    let imported_at = db
        .call(move |conn| {
            skins::insert(
                conn,
                &owner,
                &skins::NewSkin {
                    hash: &stored,
                    manifest_json: &manifest_json,
                    bytes: &canonical.bytes,
                    imported_at,
                    origin: skins::Origin::Import,
                },
            )?;
            // A package already held keeps its first import time.
            Ok(skins::list(conn, &owner)?
                .into_iter()
                .find(|r| r.hash == stored)
                .map_or(imported_at, |r| r.imported_at))
        })
        .await
        .map_err(|e| failed("skin not stored", e))?;
    Ok(SkinEntry {
        id: format!("{PKG}{hash}"),
        name: m.name,
        imported_at: Some(imported_at),
        credits: rekindle_skin::skin_tree::brand::credits(m.credits),
        short_targets,
        conforming: short_targets == 0,
    })
}

/// Why a library change was refused inside its transaction.
enum Refusal {
    Missing,
    InUse,
}

/// `SkinRemove`: never the identity's skin in use, nor the one on trial.
async fn remove(ctx: &DaemonContext, id: String) -> Answer<IpcResponse> {
    let Some(SkinId::Package(hash)) = ids::parse(&id) else {
        return Err(not_found(&id));
    };
    let (db, owner) = (ctx.require_db()?, ctx.require_owner()?);
    if ctx.skin.trial.current(&owner).is_some_and(|t| t.id == id) {
        return Err(in_use(&id));
    }
    let (hash, skin_id) = (hash.to_owned(), id.clone());
    let done = db
        .call(move |conn| {
            if preferences::skin_choice(conn, &owner)?.skin_id == skin_id {
                return Ok(Err(Refusal::InUse));
            }
            Ok(if skins::remove(conn, &owner, &hash)? {
                Ok(())
            } else {
                Err(Refusal::Missing)
            })
        })
        .await
        .map_err(|e| failed("skin not removed", e))?;
    match done {
        Ok(()) => list(ctx).await.map(|l| IpcResponse::ok(&l)),
        Err(Refusal::Missing) => Err(not_found(&id)),
        Err(Refusal::InUse) => Err(in_use(&id)),
    }
}

fn in_use(id: &str) -> IpcResponse {
    IpcResponse::error(
        409,
        format!("{id} is the skin in use: choose another first"),
    )
}

/// `SkinSelect`: the identity's skin. A built-in (or the stored skin
/// again) is stored at once; any other library skin goes on trial
/// ("Keep this skin?", [`trial`]) and is stored only when kept.
async fn select(ctx: &DaemonContext, id: String) -> Answer<IpcResponse> {
    let parsed = ids::parse(&id).ok_or_else(|| not_found(&id))?;
    if matches!(parsed, SkinId::Builtin(_)) && !conform::is_builtin(&id) {
        return Err(not_found(&id));
    }
    let (db, owner) = (ctx.require_db()?, ctx.require_owner()?);
    let hash = match parsed {
        SkinId::Package(h) => Some(h.to_owned()),
        SkinId::Builtin(_) => None,
    };
    let (skin_id, who, held) = (id.clone(), owner.clone(), hash.clone());
    let stored = db
        .call(move |conn| {
            if let Some(hash) = held {
                if !skins::contains(conn, &who, &hash)? {
                    return Ok(None);
                }
            }
            Ok(Some(preferences::skin_choice(conn, &who)?.skin_id))
        })
        .await
        .map_err(|e| failed("skin not chosen", e))?
        .ok_or_else(|| not_found(&id))?;
    match hash {
        Some(hash) if stored != skin_id => trial::begin(ctx, skin_id, hash).await?,
        _ => {
            db.call(move |conn| preferences::set_skin_id(conn, &owner, &skin_id).map(|()| owner))
                .await
                .map(|owner| trial::end(ctx, &owner))
                .map_err(|e| failed("skin not chosen", e))?;
            serve::announce(ctx).await?;
        }
    }
    list(ctx).await.map(|l| IpcResponse::ok(&l))
}

/// `SkinSetLogin`: a built-in, or a stripped copy of a library package.
async fn set_login(ctx: &DaemonContext, id: String) -> Answer<IpcResponse> {
    match ids::parse(&id).ok_or_else(|| not_found(&id))? {
        SkinId::Builtin(_) => {
            if !conform::is_builtin(&id) {
                return Err(not_found(&id));
            }
            login::set_builtin(&ctx.root, &id).map_err(|e| failed("login skin", e))?;
        }
        SkinId::Package(hash) => {
            let (db, owner) = (ctx.require_db()?, ctx.require_owner()?);
            let hash = hash.to_owned();
            let bytes = db
                .call(move |conn| skins::bytes(conn, &owner, &hash))
                .await
                .map_err(|e| failed("skin unreadable", e))?
                .ok_or_else(|| not_found(&id))?;
            let copy = rekindle_skin::login::strip(&bytes).map_err(|e| failed("login copy", e))?;
            login::set_copy(&ctx.root, &copy).map_err(|e| failed("login skin", e))?;
        }
    }
    serve::announce(ctx).await?;
    list(ctx).await.map(|l| IpcResponse::ok(&l))
}

/// The built-ins (the conforming alternate right after Symbiosis), then the
/// library rows, each with its short-target count from the package as
/// served: a library skin's from its bytes, never its stored manifest.
fn entries(rows: Vec<(skins::SkinRow, Option<Vec<u8>>)>) -> Answer<Vec<SkinEntry>> {
    let mut out = Vec::new();
    for pack in &builtin::PACKS {
        let m = manifest_of(pack.bytes)?;
        let id = format!("{}{}", builtin::PREFIX, pack.id);
        let alternate = (id == conform::SOURCE).then(|| m.credits.clone());
        let name = rekindle_skin::skin_tree::brand::name(&m.name);
        out.push(builtin_entry(id, name, m.credits)?);
        if let Some(credits) = alternate {
            out.push(builtin_entry(
                conform::ID.to_owned(),
                conform::NAME.to_owned(),
                credits,
            )?);
        }
    }
    for (row, bytes) in rows {
        let id = format!("{PKG}{}", row.hash);
        let m: SkinManifest =
            serde_json::from_str(&row.manifest_json).map_err(|e| failed("skin manifest", e))?;
        let bytes = bytes.ok_or_else(|| failed("skin unreadable", &id))?;
        let short_targets =
            conform::short_targets_of(&bytes).map_err(|e| failed("skin targets unreadable", e))?;
        out.push(SkinEntry {
            id,
            name: rekindle_skin::skin_tree::brand::name(&m.name),
            imported_at: Some(row.imported_at),
            credits: rekindle_skin::skin_tree::brand::credits(m.credits),
            short_targets,
            conforming: short_targets == 0,
        });
    }
    Ok(out)
}

/// A built-in's entry, its short-target count as served.
fn builtin_entry(
    id: String,
    name: String,
    credits: Option<rekindle_types::dto::skin::SkinCredits>,
) -> Answer<SkinEntry> {
    let short_targets = conform::builtin_short_targets(&id)
        .map_err(|e| failed("skin targets unreadable", e))?
        .ok_or_else(|| not_found(&id))?;
    Ok(SkinEntry {
        id,
        name,
        imported_at: None,
        credits: rekindle_skin::skin_tree::brand::credits(credits),
        short_targets,
        conforming: short_targets == 0,
    })
}

/// `SkinList`: the built-ins, then the library; the choice and login skin.
async fn list(ctx: &DaemonContext) -> Answer<SkinLibrary> {
    let (db, owner) = (ctx.require_db()?, ctx.require_owner()?);
    let trial = ctx.skin.trial.current(&owner);
    let (rows, choice) = db
        .call(move |conn| {
            let mut held = Vec::new();
            for row in skins::list(conn, &owner)? {
                let bytes = skins::bytes(conn, &owner, &row.hash)?;
                held.push((row, bytes));
            }
            Ok((held, preferences::skin_choice(conn, &owner)?))
        })
        .await
        .map_err(|e| failed("skins unreadable", e))?;
    // Counting a package's short targets lays it out: off the async workers.
    let skins_out = rekindle_lifecycle::run_blocking(move || entries(rows))
        .await
        .map_err(|e| failed("skin list task", e))??;
    let login = login::read(&ctx.root).map_err(|e| failed("login skin", e))?;
    Ok(SkinLibrary {
        skins: skins_out,
        active: choice.skin_id,
        login,
        trial,
    })
}
