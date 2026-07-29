//! Handle inbound DmaBufRef — reader side (Tier 3).
//! Correlates fd from sidechannel with metadata, delivers to FrameRouter.

use crate::v4::codec::streaming::dmabuf_ref as codec;
use crate::v4::handlers::HandlerError;
use crate::v4::router::{ConnectionInfo, FrameRouter};

pub fn handle(
    router: &dyn FrameRouter,
    info: &ConnectionInfo,
    payload: &[u8],
) -> Result<(), HandlerError> {
    let dmabuf = codec::decode(payload)
        .map_err(|e| HandlerError::CodecFailed(format!("{e}")))?;

    router.on_dmabuf_ref(info, &dmabuf, dmabuf.payload_id_hint);

    Ok(())
}
