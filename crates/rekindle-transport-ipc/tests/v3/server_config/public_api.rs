/// ServerConfig is importable from the public API path.
/// This is a compile-time gate — if it fails, downstream crates
/// cannot construct server configs at all.
#[test]
fn server_config_importable() {
    let _config: rekindle_transport_ipc::v3::context::ServerConfig =
        rekindle_transport_ipc::v3::context::ServerConfig::new();
}

/// BulkCounters is importable from the public API path.
/// Node crate needs this to construct and inject counters.
#[test]
fn bulk_counters_importable() {
    let _counters = rekindle_transport_ipc::v3::bulk::counters::BulkCounters::new();
}

/// ConnectionHandle is importable from the public API path.
/// Node crate's DaemonRouter holds ConnectionHandle per connection.
#[test]
fn connection_handle_importable() {
    // ConnectionHandle is a struct — we verify the path resolves.
    // We can't construct one without a real connection, but the type
    // must be nameable for DaemonRouter's field type.
    fn _assert_type_exists(_: &rekindle_transport_ipc::v3::server::ConnectionHandle) {}
}

/// SessionConfig is importable (regression gate — already public).
#[test]
fn session_config_importable() {
    let _config = rekindle_transport_ipc::v3::context::SessionConfig::default();
}

/// HandshakeConfig is importable (regression gate — already public).
#[test]
fn handshake_config_importable() {
    let _config = rekindle_transport_ipc::v3::session::handshake::HandshakeConfig::default();
}

/// FrameRouter trait is importable (node implements this).
#[test]
fn frame_router_importable() {
    fn _assert_trait_exists<T: rekindle_transport_ipc::v3::router::FrameRouter>() {}
}

/// OutboundFrame is importable (DaemonRouter constructs these to send replies).
#[test]
fn outbound_frame_importable() {
    fn _assert_type_exists(_: &rekindle_transport_ipc::v3::context::OutboundFrame) {}
}
