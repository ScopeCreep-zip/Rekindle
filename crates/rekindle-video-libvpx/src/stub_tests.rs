use super::*;

#[test]
fn new_reports_unsupported_without_feature() {
    match LibvpxEncoder::new(Codec::Vp9) {
        Err(VideoError::Unsupported(msg)) => assert!(msg.contains("libvpx")),
        Err(other) => panic!("expected Unsupported, got a different error: {other:?}"),
        Ok(_) => panic!("stub new() must never succeed without the libvpx feature"),
    }
}

#[test]
fn decoder_new_reports_unsupported_without_feature() {
    match LibvpxDecoder::new(Codec::Vp9) {
        Err(VideoError::Unsupported(msg)) => assert!(msg.contains("libvpx")),
        Err(other) => panic!("expected Unsupported, got a different error: {other:?}"),
        Ok(_) => panic!("stub new() must never succeed without the libvpx feature"),
    }
}
