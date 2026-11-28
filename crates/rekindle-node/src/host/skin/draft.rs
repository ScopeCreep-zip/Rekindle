//! The skin editor's requests (plan E7.7.5): validation runs in the Host,
//! never in a frontend. `SkinValidateDraft` answers `rekindle_skin::draft`'s
//! report; `SkinDraftStart` is "Start from the current skin" (its credits,
//! licence and assets carried forward); `SkinDraftCheck` runs the whole
//! package, imported regions included, through `rekindle-skin-helper`
//! (region import); `SkinExportDraft` does the same and writes the
//! canonical package to the file the user chose; `SkinExportSchema` writes
//! the token file's JSON Schema for DTCG tools.

use std::collections::BTreeMap;

use rekindle_skin::draft::{self, Assets};
use rekindle_skin::helper::{self, HelperError};
use rekindle_skin::output::Canonical;
use rekindle_types::dto::skin_draft::{SkinDraft, SkinDraftReport};

use super::{failed, helper_opts, serve, Answer};
use crate::daemon::dispatch::DaemonContext;
use rekindle_ipc::protocol::IpcResponse;

fn refused(e: impl std::fmt::Display) -> IpcResponse {
    IpcResponse::error(400, format!("skin refused: {e}"))
}

/// `SkinValidateDraft`: the report (the walk is arithmetic over typed
/// tokens, run off the async workers).
pub(super) async fn validate(draft: SkinDraft) -> Answer<SkinDraftReport> {
    rekindle_lifecycle::run_blocking(move || draft::report(&draft))
        .await
        .map_err(|e| failed("skin draft task", e))?
        .map_err(refused)
}

/// `SkinDraftStart`: `id`'s active variant, or the skin shown now.
pub(super) async fn start(ctx: &DaemonContext, id: Option<String>) -> Answer<SkinDraft> {
    let (id, bytes) = serve::package_bytes(ctx, id).await?;
    draft::from_package(&id, &bytes).map_err(|e| failed("skin unreadable", e))
}

/// The bytes the draft's package needs: its source's canonical package and
/// each imported region's image, read from the path the open dialog gave.
async fn assets(ctx: &DaemonContext, d: &SkinDraft) -> Answer<Assets> {
    let source = match &d.from {
        Some(id) => Some(serve::package_bytes(ctx, Some(id.clone())).await?.1),
        None => None,
    };
    let paths: Vec<(String, String)> = d
        .regions
        .iter()
        .map(|(id, r)| (id.clone(), r.path.clone()))
        .collect();
    let regions = rekindle_lifecycle::run_blocking(move || {
        paths
            .into_iter()
            .map(|(id, path)| std::fs::read(&path).map(|b| (id, b)))
            .collect::<std::io::Result<BTreeMap<_, _>>>()
    })
    .await
    .map_err(|e| failed("skin region task", e))?
    .map_err(|e| failed("skin region unreadable", e))?;
    Ok(Assets { source, regions })
}

/// `SkinDraftCheck`: the draft's whole package through the helper, the
/// only gate: a bad nine-slice or a failing pair is refused by name.
pub(super) async fn check(ctx: &DaemonContext, d: SkinDraft) -> Answer<Canonical> {
    let assets = assets(ctx, &d).await?;
    let built = draft::package(&d, &assets).map_err(refused)?;
    let opts = helper_opts(ctx)?;
    rekindle_lifecycle::run_blocking(move || helper::validate(&built.bytes, &opts))
        .await
        .map_err(|e| failed("skin helper task", e))?
        .map_err(|e| match e {
            HelperError::Refused(rule) => refused(rule),
            other => failed("skin not validated", other),
        })
}

async fn write(path: String, bytes: Vec<u8>) -> Answer<()> {
    rekindle_lifecycle::run_blocking(move || std::fs::write(path, bytes))
        .await
        .map_err(|e| failed("skin export task", e))?
        .map_err(|e| failed("skin not written", e))
}

/// `SkinExportDraft`: the helper validates the draft's package, and the
/// canonical bytes go to `path`; the answer is the package address.
pub(super) async fn export(ctx: &DaemonContext, d: SkinDraft, path: String) -> Answer<String> {
    let canonical = check(ctx, d).await?;
    let address = canonical.address();
    write(path, canonical.bytes).await?;
    Ok(address)
}

/// `SkinExportSchema`: the token file's JSON Schema, every themeable id
/// typed (`rekindle_skin::generate::json_schema`), written to `path`.
pub(super) async fn export_schema(path: String) -> Answer<()> {
    write(path, rekindle_skin::generate::json_schema().into_bytes()).await
}
