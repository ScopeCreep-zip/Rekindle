use super::*;

#[test]
fn encode_decode_roundtrip() {
    let value: (u32, String) = (42, "hello".into());
    let bytes = encode_frame(&value).unwrap();
    let decoded: (u32, String) = decode_frame(&bytes).unwrap();
    assert_eq!(decoded, value);
}

#[test]
fn decode_malformed_returns_err_not_panic() {
    // [RC-2] Garbage bytes must not panic.
    let garbage = vec![0xFF, 0xFE, 0xFD, 0xFC, 0xFB];
    let result: std::result::Result<(u32, String), _> = decode_frame(&garbage);
    assert!(result.is_err());
}

#[tokio::test]
async fn noise_message_roundtrip() {
    let mut buf = Vec::new();
    write_noise_message(&mut buf, b"hello").await.unwrap();
    assert_eq!(&buf[..2], &[0, 5], "16-bit big-endian length");
    let mut cursor = &buf[..];
    assert_eq!(read_noise_message(&mut cursor).await.unwrap(), b"hello");
}

/// A message longer than the Noise maximum cannot be framed.
#[tokio::test]
async fn oversized_message_is_refused() {
    let mut buf = Vec::new();
    assert!(matches!(
        write_noise_message(&mut buf, &vec![0u8; MAX_NOISE_MESSAGE + 1]).await,
        Err(IpcError::FrameTooLarge { .. })
    ));
    write_noise_message(&mut buf, &vec![0u8; MAX_NOISE_MESSAGE])
        .await
        .unwrap();
}

#[tokio::test]
async fn eof_before_length_is_connection_closed() {
    let mut cursor: &[u8] = &[];
    assert!(matches!(
        read_noise_message(&mut cursor).await,
        Err(IpcError::ConnectionClosed)
    ));
}
