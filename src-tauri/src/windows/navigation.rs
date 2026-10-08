//! Navigation and permission policy shared by every webview window.
//!
//! Every window is built through `super::base_builder`, which installs:
//! - [`is_app_origin`] as the top-level navigation guard, so a webview can
//!   never leave the bundled frontend (`file:`, `data:`, remote pages);
//! - a new-window handler that denies `window.open` and `target=_blank`;
//! - [`permission_for`] as the webview permission handler, so only windows
//!   whose [`WindowKind::captures_media`] is true can reach the camera,
//!   microphone or screen.

use tauri::webview::{PermissionKind, PermissionResponse};
use tauri::Url;

/// Whether `url` is the app's own frontend.
///
/// - `tauri://localhost`: the bundled frontend on macOS and Linux.
/// - `http(s)://tauri.localhost`: the bundled frontend on Windows (WebView2
///   serves custom protocols over http/https).
/// - `dev_origin`: the Vite dev server, passed only when `tauri::is_dev()`.
pub fn is_app_origin(url: &Url, dev_origin: Option<&Url>) -> bool {
    let host = url.host_str();
    match url.scheme() {
        "tauri" if host == Some("localhost") => return true,
        "http" | "https" if host == Some("tauri.localhost") => return true,
        _ => {}
    }
    dev_origin.is_some_and(|dev| dev.origin() == url.origin())
}

/// The webview permission decision for a window.
///
/// Capture windows get camera, microphone and screen capture. Everything
/// else, and every request from a non-capture window, is denied.
///
/// On Linux, WebKitGTK's device-enumeration request (`enumerateDevices`)
/// reaches wry as [`PermissionKind::Other`]. Capture windows answer it with
/// `Default` so the request falls through to the narrow native hook in
/// `platform::enable_webview_media_capture`, which allows device
/// enumeration and denies everything else.
pub fn permission_for(captures_media: bool, kind: PermissionKind) -> PermissionResponse {
    if !captures_media {
        return PermissionResponse::Deny;
    }
    match kind {
        PermissionKind::Camera | PermissionKind::Microphone | PermissionKind::DisplayCapture => {
            PermissionResponse::Allow
        }
        PermissionKind::Other if cfg!(target_os = "linux") => PermissionResponse::Default,
        _ => PermissionResponse::Deny,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn only_app_origins_navigate() {
        let dev = url("http://localhost:1430");
        assert!(is_app_origin(&url("tauri://localhost/chat?peer=ab"), None));
        assert!(is_app_origin(&url("http://tauri.localhost/login"), None));
        assert!(is_app_origin(&url("https://tauri.localhost/login"), None));
        assert!(is_app_origin(
            &url("http://localhost:1430/settings"),
            Some(&dev)
        ));

        assert!(!is_app_origin(&url("http://localhost:1430/settings"), None));
        assert!(!is_app_origin(&url("http://localhost:1431/"), Some(&dev)));
        assert!(!is_app_origin(&url("tauri://evil/"), None));
        assert!(!is_app_origin(&url("file:///etc/hosts"), Some(&dev)));
        assert!(!is_app_origin(&url("data:text/html,x"), Some(&dev)));
        assert!(!is_app_origin(&url("javascript:alert(1)"), Some(&dev)));
        assert!(!is_app_origin(&url("https://evil.example/"), Some(&dev)));
        assert!(!is_app_origin(
            &url("https://tauri.localhost.evil.example/"),
            None
        ));
        assert!(!is_app_origin(&url("about:blank"), Some(&dev)));
    }

    #[test]
    fn only_capture_windows_capture() {
        for kind in [
            PermissionKind::Camera,
            PermissionKind::Microphone,
            PermissionKind::DisplayCapture,
        ] {
            assert_eq!(permission_for(true, kind), PermissionResponse::Allow);
            assert_eq!(permission_for(false, kind), PermissionResponse::Deny);
        }
        for kind in [
            PermissionKind::Geolocation,
            PermissionKind::Notifications,
            PermissionKind::ClipboardRead,
            PermissionKind::FileSystemAccess,
        ] {
            assert_eq!(permission_for(true, kind), PermissionResponse::Deny);
        }
        assert_eq!(
            permission_for(false, PermissionKind::Other),
            PermissionResponse::Deny
        );
    }
}
