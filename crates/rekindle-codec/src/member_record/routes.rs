//! `M`/6, the member's routes (plan E3.2 S7c.8, the E3.4 `MemberRoutes`
//! row of the D15 subkey map): the general private route blob every
//! app_call to the member uses, signed by the pseudonym over
//! `rekindle-member-routes-v1 ‖ community ‖ memberRecord ‖ issuedAt ‖
//! general`. The member rewrites it whenever its route is replaced, so a
//! reader resolves the route at use time from the record, as a Matrix
//! invite names a server resolved when it is used, rather than holding a
//! blob frozen when it was handed out.

use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::domains;

use super::{check_len, check_size, put_lp, sign, verify, RecordScope};
use crate::capnp_codec::{bounds, capnp_err, pack, unpack_bounded};
use crate::member_record_capnp::member_routes;
use crate::CodecError;

/// The longest route blob `M`/6 carries: a 3-hop private route blob is
/// well under it (plan E3.4 "Routes").
pub const MEMBER_ROUTE_BLOB_MAX_BYTES: usize = 4 * 1024;
/// The bound on an encoded `MemberRoutes`: the blob, the signature and
/// framing.
pub const MEMBER_ROUTES_MAX_BYTES: usize = 5 * 1024;

/// A member's verified routes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberRoutes {
    /// The general private route blob.
    pub general: Vec<u8>,
    /// When the writer issued it, on its clock (ms).
    pub issued_at: u64,
}

fn signing_bytes(at: RecordScope<'_>, issued_at: u64, general: &[u8]) -> Result<Vec<u8>, CodecError> {
    let mut out = at.signing_prefix(domains::MEMBER_ROUTES)?;
    out.extend_from_slice(&issued_at.to_be_bytes());
    put_lp(&mut out, general)?;
    Ok(out)
}

impl MemberRoutes {
    /// Sign and encode for the record `at`.
    ///
    /// # Errors
    /// The blob is empty or over [`MEMBER_ROUTE_BLOB_MAX_BYTES`].
    pub fn encode(
        &self,
        at: RecordScope<'_>,
        pseudonym: &SigningKey,
    ) -> Result<Vec<u8>, CodecError> {
        if self.general.is_empty() {
            return Err(CodecError::Serialization(
                "member routes: no route blob".into(),
            ));
        }
        check_len(
            "member route blob",
            self.general.len(),
            MEMBER_ROUTE_BLOB_MAX_BYTES,
        )?;
        let sig = sign(pseudonym, &signing_bytes(at, self.issued_at, &self.general)?);
        let mut builder = capnp::message::Builder::new_default();
        {
            let mut root = builder.init_root::<member_routes::Builder<'_>>();
            root.set_general(&self.general);
            root.set_issued_at(self.issued_at);
            root.set_sig(&sig);
        }
        let out = pack(&builder)?;
        check_size("member routes", &out, MEMBER_ROUTES_MAX_BYTES)?;
        Ok(out)
    }
}

/// Decode `M`/6 and verify its signature by `pseudonym` for the record `at`.
///
/// # Errors
/// Over its bound, malformed, an empty or oversized blob, or a signature
/// that does not verify.
pub fn decode_member_routes(
    data: &[u8],
    at: RecordScope<'_>,
    pseudonym: &[u8; 32],
) -> Result<MemberRoutes, CodecError> {
    check_size("member routes", data, MEMBER_ROUTES_MAX_BYTES)?;
    let reader = unpack_bounded(data, bounds::MEMBER_ROUTES_BOUND)?;
    let root = reader
        .get_root::<member_routes::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;
    let general = root.get_general().map_err(|e| capnp_err(&e))?;
    if general.is_empty() {
        return Err(CodecError::Deserialization(
            "member routes: no route blob".into(),
        ));
    }
    check_len(
        "member route blob",
        general.len(),
        MEMBER_ROUTE_BLOB_MAX_BYTES,
    )
    .map_err(|e| CodecError::Deserialization(e.to_string()))?;
    let issued_at = root.get_issued_at();
    verify(
        pseudonym,
        &signing_bytes(at, issued_at, general)?,
        root.get_sig().map_err(|e| capnp_err(&e))?,
        "member routes",
    )?;
    Ok(MemberRoutes {
        general: general.to_vec(),
        issued_at,
    })
}

#[cfg(test)]
#[path = "routes_tests.rs"]
mod tests;
