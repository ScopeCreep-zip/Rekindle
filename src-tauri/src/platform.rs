//! Platform-specific environment setup that must run before `tauri::Builder`.

/// Linux WebKitGTK environment setup — must run before tauri::Builder.
///
/// 1. Wayland discovery — tmux/SSH/TTY sessions don't inherit WAYLAND_DISPLAY
///    from the compositor. Scan XDG_RUNTIME_DIR for the socket.
///
/// 2. NVIDIA + WebKitGTK workarounds — proprietary drivers have known issues
///    with WebKitGTK's DMABuf renderer and explicit sync on all distros.
///
///    NOTE: this is scoped to NVIDIA on purpose. Disabling the DMABuf
///    renderer unconditionally on Linux regressed input handling on
///    non-NVIDIA Mesa/Wayland (Pop!_OS): the webview rendered but did not
///    receive pointer events, because modern WebKitGTK (2.42+) treats the
///    DMABuf path as primary and the disabled-fallback path mis-routes input
///    on Wayland subsurfaces. The asymmetric-video fix lives in the codec
///    negotiation path (frontend probe removal), not here, so this guard
///    stays NVIDIA-only.
///
/// All vars are skipped if already set, so users can always override.
///
/// See: https://github.com/tauri-apps/tauri/issues/9394
#[cfg(target_os = "linux")]
pub fn linux_display_setup() {
    use std::path::Path;

    // Wayland display discovery
    if std::env::var("WAYLAND_DISPLAY")
        .unwrap_or_default()
        .is_empty()
    {
        if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
            if let Ok(entries) = std::fs::read_dir(&runtime_dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    if name.starts_with("wayland-") && !name.ends_with(".lock") {
                        std::env::set_var("WAYLAND_DISPLAY", &*name);
                        break;
                    }
                }
            }
        }
    }

    // NVIDIA workarounds — DMABuf renderer + explicit sync both break only on
    // the proprietary driver. Gate on the driver being present so non-NVIDIA
    // Mesa/Wayland keeps the (input-correct) DMABuf path.
    if Path::new("/proc/driver/nvidia/version").exists() {
        if std::env::var("WEBKIT_DISABLE_DMABUF_RENDERER").is_err() {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        }
        if std::env::var("__NV_DISABLE_EXPLICIT_SYNC").is_err() {
            std::env::set_var("__NV_DISABLE_EXPLICIT_SYNC", "1");
        }
    }
}

/// Enable in-webview camera/microphone capture on Linux (WebKitGTK).
///
/// WebKitGTK ships the MediaStream API (`navigator.mediaDevices.getUserMedia`
/// / `enumerateDevices`) gated off and silently *denies* every media
/// permission request unless the embedder opts in — so a Tauri webview shows a
/// blank camera and an empty device list with no OS prompt. We flip on
/// `enable-media-stream` and approve the two media-device permission request
/// types (capture + device enumeration), denying every other type
/// (geolocation, notifications, …) so we don't broaden the webview's authority.
///
/// Frames are captured via WebCodecs and shipped over Veilid, so no
/// WebRTC/gstreamer stack is required — only the capture API.
///
/// No-op on macOS/Windows: those webviews honour getUserMedia natively and
/// surface the OS permission prompt themselves.
#[cfg(target_os = "linux")]
pub fn enable_webview_media_capture(window: &tauri::WebviewWindow) {
    use webkit2gtk::glib::prelude::Cast;
    use webkit2gtk::{
        DeviceInfoPermissionRequest, PermissionRequestExt, SettingsExt, UserMediaPermissionRequest,
        WebViewExt,
    };

    let label = window.label().to_string();
    let result = window.with_webview(move |platform_webview| {
        let webview = platform_webview.inner();
        if let Some(settings) = WebViewExt::settings(&webview) {
            settings.set_enable_media_stream(true);
        }
        webview.connect_permission_request(|_webview, request| {
            let is_media_request = request
                .downcast_ref::<UserMediaPermissionRequest>()
                .is_some()
                || request
                    .downcast_ref::<DeviceInfoPermissionRequest>()
                    .is_some();
            if is_media_request {
                request.allow();
            } else {
                request.deny();
            }
            true
        });
    });
    if let Err(e) = result {
        tracing::warn!(window = %label, error = %e, "could not enable webview media capture");
    }
}

/// No-op outside Linux — native webviews handle media permissions themselves.
#[cfg(not(target_os = "linux"))]
pub fn enable_webview_media_capture(_window: &tauri::WebviewWindow) {}
