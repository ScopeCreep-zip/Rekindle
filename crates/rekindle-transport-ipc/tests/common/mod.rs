// Shared test infrastructure. Not every test binary uses every item.

pub fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_env_filter("rekindle_transport_ipc=debug,warn")
        .try_init();
}

/// RAII guard that removes a socket file on drop, including during panics.
/// Use in every socket-level test to prevent stale socket file accumulation.
#[allow(dead_code)]
pub struct SocketGuard(std::path::PathBuf);

#[allow(dead_code)]
impl SocketGuard {
    pub fn new(path: std::path::PathBuf) -> Self {
        Self(path)
    }
}

impl Drop for SocketGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Assert two large byte slices are equal with first-diff diagnostic
/// instead of printing the entire payload on mismatch.
#[allow(dead_code)]
pub fn assert_payload_eq(received: &[u8], expected: &[u8]) {
    assert_eq!(
        received.len(), expected.len(),
        "payload length mismatch: got {}, want {}",
        received.len(), expected.len()
    );
    if received != expected {
        let pos = received.iter().zip(expected.iter())
            .position(|(a, b)| a != b)
            .unwrap_or(received.len());
        panic!(
            "payload mismatch at byte {pos}/{}: got 0x{:02x}, want 0x{:02x}",
            received.len(),
            received.get(pos).copied().unwrap_or(0),
            expected.get(pos).copied().unwrap_or(0),
        );
    }
}
