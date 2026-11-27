use super::*;

#[test]
fn a_header_round_trips_and_garbage_is_refused() {
    let h = VaultHeader::fresh();
    assert_eq!(VaultHeader::decode(&h.encode()).unwrap(), h);
    assert!(h.params().is_ok());
    assert!(VaultHeader::decode(&[0u8; VAULT_HEADER_LEN]).is_err());
    assert!(VaultHeader::decode(&h.encode()[..10]).is_err());
}

#[test]
fn a_header_is_never_replaced() {
    let dir = tempfile::TempDir::new().unwrap();
    let vault = dir.path().join("a.vault");
    assert!(matches!(read(&vault), Err(VaultError::HeaderMissing)));
    write_new(&vault, &VaultHeader::fresh()).unwrap();
    assert!(matches!(
        write_new(&vault, &VaultHeader::fresh()),
        Err(VaultError::AlreadyExists)
    ));
}
