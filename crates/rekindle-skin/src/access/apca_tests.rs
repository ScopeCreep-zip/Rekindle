use super::lc;

/// The reference values the APCA-W3 README lists.
#[test]
fn apca_reference_vectors() {
    let close = |a: f64, b: f64| (a - b).abs() < 0.01;
    assert!(
        close(lc([0x88; 3], [0xff; 3]), 63.056),
        "{}",
        lc([0x88; 3], [0xff; 3])
    );
    assert!(
        close(lc([0xff; 3], [0x88; 3]), -68.541),
        "{}",
        lc([0xff; 3], [0x88; 3])
    );
    assert!(
        close(lc([0; 3], [0xaa; 3]), 58.146),
        "{}",
        lc([0; 3], [0xaa; 3])
    );
    assert!(close(lc([0x11; 3], [0x11; 3]), 0.0));
}
