//! Skin id shapes (plan E7.7.2), checked by `validate_request` before a
//! request reaches the gate: `builtin:<id>` (a manifest id: lowercase
//! ASCII letters, digits, `-` and `.`) and `pkg:<hash>` (64 lowercase hex,
//! the canonical package's BLAKE3). Whether the id names a built-in or a
//! library row is the handler's check.

use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_skin::builtin;

/// The prefix of a library package's id.
pub(crate) const PKG: &str = "pkg:";
/// The prefix of the device login skin's stripped copy.
pub(crate) const DEV: &str = "dev:";

/// A skin id, parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SkinId<'a> {
    /// `builtin:<id>`; `id` names a pack only if `builtin::find` says so.
    Builtin(&'a str),
    /// `pkg:<hash>`.
    Package(&'a str),
}

pub(crate) use rekindle_skin::serve::is_hash;

fn is_manifest_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.'))
}

/// `id` as a skin id, or `None` for any other shape.
pub(crate) fn parse(id: &str) -> Option<SkinId<'_>> {
    if let Some(rest) = id.strip_prefix(builtin::PREFIX) {
        return is_manifest_id(rest).then_some(SkinId::Builtin(rest));
    }
    id.strip_prefix(PKG)
        .filter(|h| is_hash(h))
        .map(SkinId::Package)
}

fn refuse(what: &str) -> IpcResponse {
    IpcResponse::error(400, format!("invalid skin id: {what}"))
}

/// `validate_request`'s arm for the skin requests.
///
/// # Errors
/// Each selection field within its bound: a key and an id are names, an
/// address `host:port`, a path a file path.
fn selection_fits(selection: &rekindle_types::dto::skin_run::SkinSelection) -> bool {
    use rekindle_types::dto::skin_run::SkinSelection as S;
    match selection {
        S::Friend { public_key } => public_key.len() <= 128,
        S::Server { address, .. } => address.len() <= 256,
        S::Transfer {
            attachment_id,
            target,
        } => attachment_id.len() <= 64 && target_fits(target),
        S::Group { group_id: id } => id.len() <= 64,
        S::Media { path } => path.len() <= 4096,
        S::FriendGroup { .. } => true,
        S::Community {
            community,
            channel,
            server,
        } => {
            community.len() <= 128
                && channel.as_ref().is_none_or(|c| c.len() <= 64)
                && server.as_ref().is_none_or(|s| s.len() <= 64)
        }
    }
}

/// A transfer's place: a community key and a channel id, each a name.
pub(crate) fn target_fits(target: &rekindle_types::dto::transfer::TransferTarget) -> bool {
    let rekindle_types::dto::transfer::TransferTarget::Channel {
        community,
        channel_id,
    } = target;
    community.len() <= 128 && channel_id.len() <= 64
}

/// Failure details are bounded as a media path, a short message, or the
/// same positive layout dimensions already checked by the skin helper.
fn cause_fits(cause: &rekindle_types::dto::skin_library::SkinApplyFailure) -> bool {
    use rekindle_types::dto::skin_library::SkinApplyFailure as F;
    match cause {
        F::ImageDecode { path } => !path.is_empty() && path.len() <= 4096,
        F::SheetError { detail } => detail.len() <= 1024,
        F::WindowTooSmall { width, height } => {
            (1..=rekindle_skin::shape::MAX_WINDOW_WIDTH).contains(&u32::from(*width))
                && (1..=rekindle_skin::shape::MAX_WINDOW_HEIGHT).contains(&u32::from(*height))
        }
    }
}

/// A 400 naming the field.
pub(crate) fn check(request: &IpcRequest) -> Result<(), IpcResponse> {
    match request {
        // A node id and a friend key are names, bounded like one.
        IpcRequest::RunSkinAction { node, .. } if node.len() > 256 => Err(IpcResponse::error(
            400,
            "skin node id longer than 256 bytes",
        )),
        // A window label is a name too (lead decision 48).
        IpcRequest::RunSkinAction { window, .. } | IpcRequest::SkinSelectionSet { window, .. }
            if window.is_empty() || window.len() > 64 =>
        {
            Err(IpcResponse::error(
                400,
                "skin window label not 1 to 64 bytes",
            ))
        }
        IpcRequest::SkinApplyFailed { window, .. } if window.is_empty() || window.len() > 64 => {
            Err(IpcResponse::error(
                400,
                "skin window label not 1 to 64 bytes",
            ))
        }
        // `builtin:<id>`, `pkg:<hash>` or `dev:<hash>` (`SkinApplied::id`).
        IpcRequest::SkinApplyFailed { skin, .. } if skin.is_empty() || skin.len() > 128 => {
            Err(IpcResponse::error(400, "skin id not 1 to 128 bytes"))
        }
        IpcRequest::SkinApplyFailed { cause, .. } if !cause_fits(cause) => Err(IpcResponse::error(
            400,
            "a skin apply failure field is out of bounds",
        )),
        IpcRequest::SkinSelectionSet {
            selection: Some(selection),
            ..
        } if !selection_fits(selection) => Err(IpcResponse::error(
            400,
            "a selection field is longer than its bound",
        )),
        IpcRequest::SkinList
        | IpcRequest::SkinFrames
        | IpcRequest::SkinGet { id: None }
        | IpcRequest::SkinDraftStart { id: None }
        | IpcRequest::RunSkinAction { .. }
        | IpcRequest::SkinSelectionSet { .. }
        | IpcRequest::SkinApplyFailed { .. } => Ok(()),
        IpcRequest::SkinGet { id: Some(id) } | IpcRequest::SkinDraftStart { id: Some(id) } => {
            parse(id)
                .map(drop)
                .ok_or_else(|| refuse("expected builtin:<id> or pkg:<64 hex>"))
        }
        // The scheme's path: one package address and one file, nothing
        // else (`rekindle_skin::serve::parse_path`).
        IpcRequest::SkinFile { path } => rekindle_skin::serve::parse_path(path)
            .map(drop)
            .ok_or_else(|| IpcResponse::error(404, "no such skin file")),
        IpcRequest::SkinImport { bytes } => {
            let max = rekindle_skin::limits::MAX_PACKAGE_BYTES;
            if bytes.is_empty() || bytes.len() as u64 > max {
                return Err(IpcResponse::error(
                    400,
                    format!("invalid skin package: 1 to {max} bytes"),
                ));
            }
            Ok(())
        }
        IpcRequest::SkinRemove { id } => match parse(id) {
            Some(SkinId::Package(_)) => Ok(()),
            Some(SkinId::Builtin(_)) => {
                Err(IpcResponse::error(400, "a built-in skin cannot be removed"))
            }
            None => Err(refuse("expected pkg:<64 hex>")),
        },
        // Only a library skin is ever on trial.
        IpcRequest::SkinKeep { id, .. } => match parse(id) {
            Some(SkinId::Package(_)) => Ok(()),
            _ => Err(refuse("expected pkg:<64 hex>")),
        },
        IpcRequest::SkinSelect { id } | IpcRequest::SkinSetLogin { id } => parse(id)
            .map(drop)
            .ok_or_else(|| refuse("expected builtin:<id> or pkg:<64 hex>")),
        IpcRequest::SkinValidateDraft { draft } | IpcRequest::SkinDraftCheck { draft } => {
            draft_bounds(draft)
        }
        IpcRequest::SkinExportDraft { draft, path } => {
            draft_bounds(draft)?;
            absolute(path, "export path")
        }
        IpcRequest::SkinExportSchema { path } => absolute(path, "export path"),
        other => unreachable!("{} is not a skin request", other.name()),
    }
}

fn absolute(path: &str, what: &str) -> Result<(), IpcResponse> {
    if path.is_empty() || !std::path::Path::new(path).is_absolute() {
        return Err(refuse(&format!("expected an absolute {what}")));
    }
    Ok(())
}

/// A draft within a package's bounds (`rekindle_skin::tokens`, the
/// registry's regions, the manifest's licence text): the Host's report and
/// the validator name every other rule. Its source is a skin id, and each
/// imported region is a registry region read from an absolute path.
fn draft_bounds(draft: &rekindle_types::dto::skin_draft::SkinDraft) -> Result<(), IpcResponse> {
    let max = rekindle_skin::tokens::MAX_TOKENS;
    let texts = [&draft.name, &draft.author, &draft.description];
    if draft.tokens.len() as u64 > max
        || texts.iter().any(|t| t.len() > MAX_DRAFT_TEXT_BYTES)
        || draft.license.len() > MAX_LICENSE_BYTES
    {
        return Err(IpcResponse::error(
            400,
            format!(
                "invalid skin draft: at most {max} tokens, {MAX_DRAFT_TEXT_BYTES}-byte texts and a {MAX_LICENSE_BYTES}-byte licence"
            ),
        ));
    }
    if let Some(from) = &draft.from {
        parse(from).ok_or_else(|| refuse("expected builtin:<id> or pkg:<64 hex>"))?;
    }
    for (id, region) in &draft.regions {
        if rekindle_skin::regions::slot_of(id).is_none() {
            return Err(refuse("expected a registry region"));
        }
        absolute(&region.path, "region image path")?;
    }
    Ok(())
}

/// A draft text's bytes: the manifest's description, 280 scalars at 4
/// bytes each (the name and author are 64).
const MAX_DRAFT_TEXT_BYTES: usize = 1120;
/// A licence's full text: the manifest's 64 Ki scalars at 4 bytes each.
const MAX_LICENSE_BYTES: usize = 256 * 1024;

#[cfg(test)]
#[path = "ids_tests.rs"]
mod tests;
