use rekindle_transport_ipc::v3::handoff::memfd::{
    create_memfd, write_and_seal, map_readonly, verify_content, MemfdError,
};

#[test]
fn create_memfd_returns_valid_fd() {
    let fd = create_memfd("test-create", 1024).expect("create_memfd failed");
    assert!(fd >= 0);
    // SAFETY: fd is a valid memfd we just created.
    unsafe { libc::close(fd); }
}

#[test]
fn create_memfd_zero_size() {
    let fd = create_memfd("test-zero", 0).expect("create_memfd with size 0 must succeed");
    assert!(fd >= 0);
    unsafe { libc::close(fd); }
}

#[test]
fn create_and_seal_roundtrip() {
    let data = vec![0xDE, 0xAD, 0xBE, 0xEF, 0xCA, 0xFE];
    let sealed = write_and_seal("test", &data).expect("seal failed");
    let mapping = map_readonly(sealed.fd(), sealed.size()).expect("map failed");
    assert_eq!(mapping.as_slice(), &data);
}

#[test]
fn verify_correct_hash() {
    let data = vec![0xAA; 1024];
    let expected = *blake3::hash(&data).as_bytes();
    let sealed = write_and_seal("test", &data).unwrap();
    let mapping = map_readonly(sealed.fd(), sealed.size()).unwrap();
    assert!(verify_content(&mapping, &expected).is_ok());
}

#[test]
fn verify_wrong_hash_fails() {
    let data = vec![0xBB; 1024];
    let sealed = write_and_seal("test", &data).unwrap();
    let mapping = map_readonly(sealed.fd(), sealed.size()).unwrap();
    let wrong = [0xFF; 32];
    let err = verify_content(&mapping, &wrong).unwrap_err();
    match err {
        MemfdError::VerifyFailed { expected, computed } => {
            assert_eq!(expected, wrong);
            assert_eq!(computed, *blake3::hash(&data).as_bytes());
        }
        other => panic!("Expected VerifyFailed, got {other:?}"),
    }
}

#[test]
fn size_matches() {
    let data = vec![0; 4096];
    let sealed = write_and_seal("test", &data).unwrap();
    assert_eq!(sealed.size(), 4096);
}

#[test]
fn empty_payload_works() {
    let sealed = write_and_seal("test", &[]).unwrap();
    assert_eq!(sealed.size(), 0);
    let expected = *blake3::hash(&[]).as_bytes();
    let mapping = map_readonly(sealed.fd(), sealed.size()).unwrap();
    assert!(verify_content(&mapping, &expected).is_ok());
}

#[test]
fn large_payload_1mib() {
    let data = vec![0x42; 1024 * 1024];
    let expected = *blake3::hash(&data).as_bytes();
    let sealed = write_and_seal("test", &data).unwrap();
    let mapping = map_readonly(sealed.fd(), sealed.size()).unwrap();
    assert!(verify_content(&mapping, &expected).is_ok());
    assert_eq!(mapping.as_slice().len(), 1024 * 1024);
}

#[test]
fn multiple_concurrent_memfds() {
    let mut sealeds = Vec::new();
    for i in 0u8..10 {
        let data = vec![i; 256];
        sealeds.push(write_and_seal("test", &data).unwrap());
    }
    for (i, sealed) in sealeds.iter().enumerate() {
        let mapping = map_readonly(sealed.fd(), sealed.size()).unwrap();
        let expected_data = vec![i as u8; 256];
        assert_eq!(mapping.as_slice(), &expected_data);
    }
}
