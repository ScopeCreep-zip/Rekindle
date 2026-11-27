use super::*;

#[test]
fn wcag_vectors() {
    assert!((contrast([0, 0, 0], [255, 255, 255]) - 21.0).abs() < 1e-9);
    // The accent fill's documented 4.54:1 under white text.
    let r = contrast([0x16, 0x7b, 0xc0], [255, 255, 255]);
    assert!((r - 4.54).abs() < 0.01, "{r}");
    assert_eq!(
        mix([255, 255, 255, 255], [0, 0, 0, 255], 500),
        [128, 128, 128, 255]
    );
    assert_eq!(over([255, 255, 255, 0], [10, 20, 30]), [10, 20, 30]);
}
