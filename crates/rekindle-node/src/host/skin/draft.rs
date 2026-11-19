//! The skin editor's requests (plan E7.7.5): validation runs in the Host,
//! never in a frontend. `SkinValidateDraft` answers `rekindle_skin::draft`'s
//! report; `SkinDraftStart` is "Start from the current skin";
//! `SkinExportDraft` builds the package, has `rekindle-skin-helper` run the
//! whole pipeline on it (the validator is the only gate, so a failing pair
//! is refused with its named ratio) and writes the canonical package to
//! the file the user chose.

use rekindle_skin::draft;
use rekindle_skin::helper::{self, HelperError};
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
    let bytes = serve::package_bytes(ctx, id).await?;
    draft::from_package(&bytes).map_err(|e| failed("skin unreadable", e))
}

/// `SkinExportDraft`: the helper validates the draft's package, and the
/// canonical bytes go to `path`; the answer is the package address.
pub(super) async fn export(ctx: &DaemonContext, draft: SkinDraft, path: String) -> Answer<String> {
    let built = draft::package(&draft).map_err(refused)?;
    let opts = helper_opts(ctx)?;
    let canonical = rekindle_lifecycle::run_blocking(move || helper::validate(&built.bytes, &opts))
        .await
        .map_err(|e| failed("skin helper task", e))?
        .map_err(|e| match e {
            HelperError::Refused(rule) => refused(rule),
            other => failed("skin not validated", other),
        })?;
    let address = canonical.address();
    rekindle_lifecycle::run_blocking(move || std::fs::write(path, &canonical.bytes))
        .await
        .map_err(|e| failed("skin export task", e))?
        .map_err(|e| failed("skin not written", e))?;
    Ok(address)
}
