//! QR codes the Host renders for every frontend (plans E2.3, E2.5): the
//! safety number's scannable form and our friend code. Each frontend draws
//! the rows (SVG in the webview, half blocks in a terminal), so no
//! frontend links a QR encoder.

use super::error::FriendError;

/// `data` as a QR code: one string per row, `1` a dark module, without the
/// quiet zone.
///
/// # Errors
/// `Failed` when `data` does not fit a QR code; `what` names it.
pub(super) fn rows(data: &[u8], what: &str) -> Result<Vec<String>, FriendError> {
    let code =
        qrcode::QrCode::new(data).map_err(|e| FriendError::Failed(format!("{what} QR: {e}")))?;
    let width = code.width();
    Ok(code
        .to_colors()
        .chunks(width)
        .map(|row| {
            row.iter()
                .map(|c| if *c == qrcode::Color::Dark { '1' } else { '0' })
                .collect()
        })
        .collect())
}

#[cfg(test)]
#[path = "qr_tests.rs"]
mod tests;
