use super::*;

/// Write a valid `stem` keypair into `dir`.
fn write_keypair(dir: &Path, stem: &str) -> [u8; 32] {
    let public = [7u8; 32];
    let private = [9u8; 32];
    std::fs::write(dir.join(format!("{stem}.pub")), public).unwrap();
    std::fs::write(dir.join(format!("{stem}.key")), private).unwrap();
    std::fs::write(
        dir.join(format!("{stem}.checksum")),
        keypair_checksum(&public, &private),
    )
    .unwrap();
    public
}

#[tokio::test]
async fn checked_keypair_reads_an_intact_keypair() {
    let dir = tempfile::tempdir().unwrap();
    let public = write_keypair(dir.path(), "agent");
    let (private, read_public) = read_checked_keypair(dir.path(), "agent").await.unwrap();
    assert_eq!(read_public, public);
    assert_eq!(private.as_slice(), &[9u8; 32]);
}

#[tokio::test]
async fn checked_keypair_fails_closed() {
    let dir = tempfile::tempdir().unwrap();
    write_keypair(dir.path(), "agent");
    std::fs::remove_file(dir.path().join("agent.checksum")).unwrap();
    assert!(matches!(
        read_checked_keypair(dir.path(), "agent").await,
        Err(IpcError::KeyIncomplete { .. })
    ));

    write_keypair(dir.path(), "agent");
    std::fs::write(dir.path().join("agent.key"), b"short").unwrap();
    assert!(matches!(
        read_checked_keypair(dir.path(), "agent").await,
        Err(IpcError::KeyMalformed { got: 5, .. })
    ));

    write_keypair(dir.path(), "agent");
    std::fs::write(dir.path().join("agent.key"), [8u8; 32]).unwrap();
    assert!(matches!(
        read_checked_keypair(dir.path(), "agent").await,
        Err(IpcError::KeyTamperDetected { .. })
    ));
}

#[test]
fn validate_agent_name_accepts_valid() {
    assert!(validate_agent_name("rekindle-tui").is_ok());
    assert!(validate_agent_name("ai_agent_01").is_ok());
    assert!(validate_agent_name("Bot42").is_ok());
}

#[test]
fn validate_agent_name_rejects_invalid() {
    assert!(validate_agent_name("").is_err());
    assert!(validate_agent_name("../escape").is_err());
    assert!(validate_agent_name("has/slash").is_err());
    assert!(validate_agent_name("has space").is_err());
    assert!(validate_agent_name("has.dot").is_err());
}

#[test]
fn keypair_checksum_is_deterministic() {
    let kp = generate_keypair().unwrap();
    let pub_arr: [u8; 32] = kp.public().try_into().unwrap();
    let c1 = keypair_checksum(&pub_arr, kp.private());
    let c2 = keypair_checksum(&pub_arr, kp.private());
    assert_eq!(c1, c2);
}

#[test]
fn keypair_checksum_detects_tampering() {
    let kp = generate_keypair().unwrap();
    let pub_arr: [u8; 32] = kp.public().try_into().unwrap();
    let original = keypair_checksum(&pub_arr, kp.private());
    let mut tampered = kp.private().to_vec();
    tampered[0] ^= 0xFF;
    let tampered_checksum = keypair_checksum(&pub_arr, &tampered);
    assert_ne!(original, tampered_checksum);
}

#[test]
fn generate_keypair_produces_32_byte_keys() {
    let kp = generate_keypair().unwrap();
    assert_eq!(kp.private().len(), 32);
    assert_eq!(kp.public().len(), 32);
}

#[test]
fn zeroizing_keypair_into_inner_preserves_material() {
    let kp = generate_keypair().unwrap();
    let private_copy = kp.private().to_vec();
    let extracted = kp.into_inner();
    assert_eq!(extracted.private, private_copy);
}
