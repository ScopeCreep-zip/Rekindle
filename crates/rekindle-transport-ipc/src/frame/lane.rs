//! Lane byte protocol for multiplexing control and bulk frames.
//!
//! Every frame on the wire is prefixed with a 1-byte lane discriminator.
//! The lane byte is in cleartext — an observer can distinguish control
//! from bulk traffic but cannot read the content.
//!
//! Lane values:
//! - `0x00` = Noise-encrypted control frame
//! - `0x01` = BulkCipher-encrypted data chunk
//! - `0x02` = BulkCipher-encrypted final chunk (carries blob digest)
//! - `0x03` = BulkCipher-encrypted window update (flow control)

use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Lane byte for Noise-encrypted control-plane frames.
pub const LANE_CONTROL: u8 = 0x00;

/// Read a single lane byte from the reader.
///
/// Returns `UnexpectedEof` on clean disconnect.
pub async fn read_lane_byte<R: AsyncReadExt + Unpin>(
    reader: &mut R,
) -> std::io::Result<u8> {
    let mut buf = [0u8; 1];
    reader.read_exact(&mut buf).await?;
    Ok(buf[0])
}

/// Write a lane byte. Does NOT flush.
pub async fn write_lane_byte<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    lane: u8,
) -> std::io::Result<()> {
    writer.write_all(&[lane]).await
}

/// Returns true for bulk-plane lanes (0x01..=0x03).
#[inline]
pub fn is_bulk_lane(lane: u8) -> bool {
    matches!(lane, 0x01..=0x03)
}
