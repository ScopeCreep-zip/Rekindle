use crate::platform::executor::{
    apply_window_change, has_logical_inner_size, has_logical_position,
    physical_window_chrome_to_logical, physical_window_rect_to_logical, FrameId, PlatformExecutor,
    WindowRect, WINDOW_CHANGE_TIMEOUT,
};
use crate::server::response::WebDriverErrorResponse;
use async_trait::async_trait;
use serde_json::Value;
use tauri::{LogicalPosition, LogicalSize, Runtime};

/// Window geometry and frame switching.
#[async_trait]
pub trait WindowOps<R: Runtime>: PlatformExecutor<R> {
    // =========================================================================
    // Window Management
    // =========================================================================

    /// Get window rectangle (position and size)
    async fn get_window_rect(&self) -> Result<WindowRect, WebDriverErrorResponse> {
        let scale_factor = self.window().scale_factor().map_err(|error| {
            WebDriverErrorResponse::unknown_error(&format!(
                "Failed to get window scale factor: {error}"
            ))
        })?;
        let position = self.window().outer_position().map_err(|error| {
            WebDriverErrorResponse::unknown_error(&format!(
                "Failed to get window position: {error}"
            ))
        })?;
        let size = self.window().outer_size().map_err(|error| {
            WebDriverErrorResponse::unknown_error(&format!("Failed to get window size: {error}"))
        })?;

        Ok(physical_window_rect_to_logical(
            position,
            size,
            scale_factor,
        ))
    }

    /// Set window rectangle (position and size)
    async fn set_window_rect(
        &self,
        rect: WindowRect,
    ) -> Result<WindowRect, WebDriverErrorResponse> {
        // Exit fullscreen/maximized state before setting rect
        // Otherwise the window manager may ignore our size/position request
        let is_fullscreen = self.window().is_fullscreen().map_err(|error| {
            WebDriverErrorResponse::unknown_error(&format!(
                "Failed to query window fullscreen state: {error}"
            ))
        })?;
        if is_fullscreen {
            apply_window_change(
                self.window(),
                "tauri://resize",
                "exit window fullscreen state",
                WINDOW_CHANGE_TIMEOUT,
                || self.window().set_fullscreen(false),
                || !self.window().is_fullscreen().unwrap_or(false),
            )
            .await?;
        }
        let is_maximized = self.window().is_maximized().map_err(|error| {
            WebDriverErrorResponse::unknown_error(&format!(
                "Failed to query window maximized state: {error}"
            ))
        })?;
        if is_maximized {
            apply_window_change(
                self.window(),
                "tauri://resize",
                "restore maximized window",
                WINDOW_CHANGE_TIMEOUT,
                || self.window().unmaximize(),
                || !self.window().is_maximized().unwrap_or(false),
            )
            .await?;
        }

        let scale_factor = self.window().scale_factor().map_err(|error| {
            WebDriverErrorResponse::unknown_error(&format!(
                "Failed to get window scale factor: {error}"
            ))
        })?;
        let position = self.window().outer_position().map_err(|error| {
            WebDriverErrorResponse::unknown_error(&format!(
                "Failed to get window position: {error}"
            ))
        })?;
        let outer_size = self.window().outer_size().map_err(|error| {
            WebDriverErrorResponse::unknown_error(&format!(
                "Failed to get outer window size: {error}"
            ))
        })?;
        let inner_size = self.window().inner_size().map_err(|error| {
            WebDriverErrorResponse::unknown_error(&format!(
                "Failed to get inner window size: {error}"
            ))
        })?;

        let current_rect = physical_window_rect_to_logical(position, outer_size, scale_factor);
        let chrome_size = physical_window_chrome_to_logical(outer_size, inner_size, scale_factor);

        if rect.x != current_rect.x || rect.y != current_rect.y {
            apply_window_change(
                self.window(),
                "tauri://move",
                "set window position",
                WINDOW_CHANGE_TIMEOUT,
                || {
                    self.window()
                        .set_position(LogicalPosition::new(rect.x, rect.y))
                },
                || has_logical_position(self.window(), rect.x, rect.y),
            )
            .await?;
        }

        if rect.width != current_rect.width || rect.height != current_rect.height {
            // Tauri sets the inner size. Chrome is measured in logical pixels before any
            // display move so physical metrics from different scale factors are never mixed.
            let inner_size = LogicalSize::new(
                rect.width.saturating_sub(chrome_size.width),
                rect.height.saturating_sub(chrome_size.height),
            );
            apply_window_change(
                self.window(),
                "tauri://resize",
                "set window size",
                WINDOW_CHANGE_TIMEOUT,
                || self.window().set_size(inner_size),
                || has_logical_inner_size(self.window(), inner_size),
            )
            .await?;
        }

        self.get_window_rect().await
    }

    /// Maximize window
    async fn maximize_window(&self) -> Result<WindowRect, WebDriverErrorResponse> {
        apply_window_change(
            self.window(),
            "tauri://resize",
            "maximize window",
            WINDOW_CHANGE_TIMEOUT,
            || self.window().maximize(),
            || self.window().is_maximized().unwrap_or(true),
        )
        .await?;
        self.get_window_rect().await
    }

    /// Minimize window
    async fn minimize_window(&self) -> Result<(), WebDriverErrorResponse> {
        let _ = self.window().minimize();
        Ok(())
    }

    /// Set window to fullscreen
    async fn fullscreen_window(&self) -> Result<WindowRect, WebDriverErrorResponse> {
        apply_window_change(
            self.window(),
            "tauri://resize",
            "enter window fullscreen state",
            WINDOW_CHANGE_TIMEOUT,
            || self.window().set_fullscreen(true),
            || self.window().is_fullscreen().unwrap_or(true),
        )
        .await?;
        self.get_window_rect().await
    }

    // =========================================================================
    // Frames
    // =========================================================================

    /// Switch to a frame by ID (index or element reference)
    async fn switch_to_frame(&self, id: FrameId) -> Result<(), WebDriverErrorResponse> {
        match id {
            FrameId::Index(index) => {
                let script = format!(
                    r"(function() {{
                        var frames = document.querySelectorAll('iframe, frame');
                        if ({index} >= frames.length) {{
                            return false;
                        }}
                        return true;
                    }})()"
                );
                let result = self.evaluate_js(&script).await?;
                if result.get("value") == Some(&Value::Bool(false)) {
                    return Err(WebDriverErrorResponse::no_such_frame());
                }
                Ok(())
            }
            FrameId::Element(js_var) => {
                let script = format!(
                    r"(function() {{
                        var el = window.{js_var};
                        if (!el || !el.isConnected) {{
                            throw new Error('stale element reference');
                        }}
                        if (el.tagName !== 'IFRAME' && el.tagName !== 'FRAME') {{
                            throw new Error('element is not a frame');
                        }}
                        return true;
                    }})()"
                );
                self.evaluate_js(&script).await?;
                Ok(())
            }
        }
    }

    /// Switch to parent frame
    async fn switch_to_parent_frame(&self) -> Result<(), WebDriverErrorResponse> {
        // No-op - frame context is managed by the session, not the executor
        Ok(())
    }
}

impl<R: Runtime, T: PlatformExecutor<R> + ?Sized> WindowOps<R> for T {}
