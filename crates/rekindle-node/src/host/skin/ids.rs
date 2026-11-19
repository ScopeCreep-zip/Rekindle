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
/// A 400 naming the field.
pub(crate) fn check(request: &IpcRequest) -> Result<(), IpcResponse> {
    match request {
        IpcRequest::SkinList
        | IpcRequest::SkinGet { id: None }
        | IpcRequest::SkinDraftStart { id: None } => Ok(()),
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
        IpcRequest::SkinValidateDraft { draft } => draft_bounds(draft),
        IpcRequest::SkinExportDraft { draft, path } => {
            draft_bounds(draft)?;
            if path.is_empty() || !std::path::Path::new(path).is_absolute() {
                return Err(refuse("expected an absolute export path"));
            }
            Ok(())
        }
        other => unreachable!("{} is not a skin request", other.name()),
    }
}

/// A draft within a token file's bounds (`rekindle_skin::tokens`): the
/// Host's report names every other rule.
fn draft_bounds(draft: &rekindle_types::dto::skin_draft::SkinDraft) -> Result<(), IpcResponse> {
    let max = rekindle_skin::tokens::MAX_TOKENS;
    if draft.tokens.len() as u64 > max || draft.name.len() > MAX_DRAFT_NAME_BYTES {
        return Err(IpcResponse::error(
            400,
            format!("invalid skin draft: at most {max} tokens and a name of {MAX_DRAFT_NAME_BYTES} bytes"),
        ));
    }
    Ok(())
}

/// A draft name's bytes: the manifest's 64 scalars at 4 bytes each.
const MAX_DRAFT_NAME_BYTES: usize = 256;

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn only_builtin_and_pkg_shapes_parse() {
        assert_eq!(
            parse("builtin:symbiosis"),
            Some(SkinId::Builtin("symbiosis"))
        );
        assert_eq!(parse(&format!("pkg:{HASH}")), Some(SkinId::Package(HASH)));
        for bad in [
            "builtin:",
            "builtin:Sym",
            "builtin:a/b",
            "pkg:",
            &format!("pkg:{}", HASH.to_uppercase()),
            &format!("pkg:{}", "0".repeat(63)),
            &format!("dev:{HASH}"),
            "symbiosis",
            "",
        ] {
            assert_eq!(parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn validate_request_refuses_bad_shapes() {
        let remove_builtin = IpcRequest::SkinRemove {
            id: "builtin:symbiosis".into(),
        };
        assert!(check(&remove_builtin).is_err());
        assert!(check(&IpcRequest::SkinImport { bytes: Vec::new() }).is_err());
        assert!(check(&IpcRequest::SkinSelect { id: "dev:x".into() }).is_err());
        assert!(check(&IpcRequest::SkinSetLogin {
            id: format!("dev:{HASH}")
        })
        .is_err());
        assert!(check(&IpcRequest::SkinSetLogin {
            id: "builtin:symbiosis".into()
        })
        .is_ok());
    }
}
