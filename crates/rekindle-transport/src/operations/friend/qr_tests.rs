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
