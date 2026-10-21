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
mod tests {
    use super::rows;

    fn assert_square(rows: &[String]) {
        assert!(rows.len() >= 21, "a QR code is at least 21 modules wide");
        assert!(rows.iter().all(|row| row.len() == rows.len()));
        assert!(rows
            .iter()
            .all(|row| row.chars().all(|c| c == '0' || c == '1')));
        // The finder pattern's corner module is dark.
        assert!(rows[0].starts_with('1'));
    }

    #[test]
    fn the_scannable_form_fits_a_square_qr() {
        let number = rekindle_crypto::SafetyNumber::new(&[1; 32], &[2; 32]);
        assert_square(&rows(&number.scannable(), "safety number").unwrap());
    }

    #[test]
    fn the_longest_friend_code_fits_a_square_qr() {
        let longest = "x".repeat(rekindle_types::friend_code::MAX_FRIEND_CODE_URL_LEN);
        assert_square(&rows(longest.as_bytes(), "friend code").unwrap());
    }
}
