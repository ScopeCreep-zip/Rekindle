use crate::state::AppState;

/// Shut the Veilid node down (app exit, after the Host stopped: its
/// teardown released our routes and ended the session).
pub async fn shutdown_app(state: &AppState) {
    // Release our routes for good, so none persists to half-live after exit.
    let routes = state.own_routes.write().take();
    if let Some(routes) = routes {
        routes.shutdown().await;
    }

    let api = {
        let mut node = state.node.write();
        node.take().map(|nh| nh.api)
    };
    if let Some(api) = api {
        api.shutdown().await;
    }

    tracing::info!("veilid node shut down");
}
