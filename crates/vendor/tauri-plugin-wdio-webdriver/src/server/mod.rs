use std::net::SocketAddr;
use std::sync::Arc;

use tauri::{AppHandle, Manager, Runtime};
use tokio::runtime::Runtime as TokioRuntime;
use tokio::sync::RwLock;

pub mod auth;
pub mod handlers;
pub mod response;
pub mod router;
pub mod trace;

use crate::platform::{create_executor, FrameId, PlatformExecutor};
use crate::server::response::WebDriverErrorResponse;
use crate::webdriver::{SessionManager, Timeouts};

/// Shared state for the `WebDriver` server
pub struct AppState<R: Runtime> {
    pub app: AppHandle<R>,
    pub sessions: RwLock<SessionManager>,
}

impl<R: Runtime + 'static> AppState<R> {
    pub fn new(app: AppHandle<R>) -> Self {
        Self {
            app,
            sessions: RwLock::new(SessionManager::new()),
        }
    }

    /// Get a platform executor for a specific window by label
    pub fn get_executor_for_window(
        &self,
        window_label: &str,
        timeouts: Timeouts,
        frame_context: Vec<FrameId>,
    ) -> Result<Arc<dyn PlatformExecutor<R>>, WebDriverErrorResponse> {
        let window = self.app.webview_windows().get(window_label).cloned();
        tracing::debug!(window = %window_label, found = window.is_some(), "executor for window");
        window
            .map(|window| create_executor(window, timeouts, frame_context))
            .ok_or_else(WebDriverErrorResponse::no_such_window)
    }

    /// Whether a window with this label is still registered
    pub fn has_window_label(&self, window_label: &str) -> bool {
        self.app.webview_windows().contains_key(window_label)
    }

    /// Get all window labels
    pub fn get_window_labels(&self) -> Vec<String> {
        self.app.webview_windows().keys().cloned().collect()
    }
}

/// Start the `WebDriver` HTTP server on the specified port, every route
/// behind [`auth::RequestGuard`]. Without a valid run token in
/// [`auth::TOKEN_ENV_VAR`] the server does not start.
pub fn start<R: Runtime + 'static>(app: AppHandle<R>, port: u16, routes: axum::Router) {
    let Some(guard) = auth::RequestGuard::from_env(port) else {
        tracing::error!(
            "WebDriver server not started: {} is unset or not a minted token",
            auth::TOKEN_ENV_VAR
        );
        return;
    };
    let guard = Arc::new(guard);
    std::thread::spawn(move || {
        let rt = match TokioRuntime::new() {
            Ok(rt) => rt,
            Err(e) => {
                tracing::error!("Failed to create Tokio runtime for WebDriver server: {e}");
                return;
            }
        };

        rt.block_on(async {
            let state = Arc::new(AppState::new(app));
            let router = auth::guarded(router::create_router(state).merge(routes), guard)
                .layer(axum::middleware::from_fn(trace::log_request));

            let addr = SocketAddr::from(([127, 0, 0, 1], port));

            let listener = match tokio::net::TcpListener::bind(addr).await {
                Ok(l) => l,
                Err(e) => {
                    tracing::error!(
                        "Failed to bind WebDriver server to {addr} — port may already be in use: {e}"
                    );
                    return;
                }
            };

            tracing::info!("WebDriver server listening on http://{addr}");

            if let Err(e) = axum::serve(listener, router).await {
                tracing::error!("WebDriver server error: {e}");
            }
        });
    });
}
