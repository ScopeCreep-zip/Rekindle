//! Socket E2E test harness — thin re-export from the SSOT fixture module.

#[allow(unused_imports)]
pub use rekindle_transport_ipc::fixture::{
    IpcFixture, IpcFixtureConfig, BoundServer,
    payload, get_rss_bytes, handshake_config, init_tracing,
    TEST_TIMEOUT,
};

/// Connect a server+client pair with default test config.
pub async fn connected_pair() -> IpcFixture {
    init_tracing();
    IpcFixture::connect(IpcFixtureConfig::for_test()).await
}

/// Connect a server+client pair with custom config.
#[allow(dead_code)]
pub async fn connected_pair_with_config(config: IpcFixtureConfig) -> IpcFixture {
    init_tracing();
    IpcFixture::connect(config).await
}
