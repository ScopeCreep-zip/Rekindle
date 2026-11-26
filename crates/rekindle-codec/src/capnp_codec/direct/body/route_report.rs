//! 0x0150 RouteReport and 0x0151 GroupRouteReport (plan RV2, route
//! evidence source 7): the reporter's Veilid declared the owner's General
//! route blob dead (a failed remote route test) and a re-read of the
//! owner's profile returned that same blob. The report rides the stream
//! the owner already watches from the reporter (its 1:1 DM chain, or its
//! group chain inside a `GroupContent` entry), so it is authenticated by
//! that stream's seal and signature; it names the blob only by its BLAKE3.

use crate::capnp_codec::direct::fixed;
use crate::capnp_codec::CodecError;
use crate::direct_capnp::route_report;

/// The body of both kinds: the BLAKE3 of the route blob reported dead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteReport {
    pub blob_hash: [u8; 32],
}

impl RouteReport {
    pub(super) fn write(&self, mut b: route_report::Builder<'_>) {
        b.set_blob_hash(&self.blob_hash);
    }

    pub(super) fn read(r: route_report::Reader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            blob_hash: fixed(r.get_blob_hash(), "route blob hash")?,
        })
    }
}
