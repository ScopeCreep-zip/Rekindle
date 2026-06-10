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
/// 3. Nix dev-shell GUI-stack hygiene — Konductor's frontend shell leaks
///    its Nix webkitgtk/GTK/GLib/GStreamer onto LD_LIBRARY_PATH. Rekindle
///    targets the HOST WebKitGTK stack; WebKitWebProcess children inherit
///    this env, and resolving part of the stack from /nix/store and part
///    from the host mixes two GLib/GStreamer ABIs. The web process then
///    aborts during GStreamer core element registration
///    (`GStreamer:ERROR gst_register_core_elements`, "GstPadTemplate has
///    no property named 'caps'") the moment anything touches media — for
///    Rekindle that's the WebCodecs capability probe at login, which made
///    the app die right after login on dev machines. The scrub runs in
///    `main` (after our own libs are already resolved), so it governs the
///    children WebKit spawns: whichever stack the UI process loaded, the
///    web process resolves the same one via its RUNPATH instead of a mix.
///    Intended Nix runtime deps (REKINDLE_LIB_PATH: sodium/opus/alsa/dbus)
///    don't match the GUI-stack list and pass through. `.envrc` applies
///    the same filter for the UI process itself.
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

    scrub_nix_gui_stack_from_ld_library_path();
}

/// Nix-store package-name fragments of the GUI/WebKit/media stack that
/// must never leak into the webview's library search path (doc item 3
/// on [`linux_display_setup`]). Matched against the segment following
/// the store hash, e.g. `/nix/store/<hash>-webkitgtk-2.50.4/lib`.
#[cfg(target_os = "linux")]
const NIX_GUI_STACK_FRAGMENTS: &[&str] = &[
    "-webkitgtk-",
    "-gtk+3-",
    "-glib-",
    "-gdk-pixbuf-",
    "-pango-",
    "-cairo-",
    "-libsoup-",
    "-at-spi2-",
    "-librsvg-",
    "-libxkbcommon-",
    "-libayatana-",
    "-gstreamer-",
    "-gst-plugins-",
    "-graphene-",
    "-harfbuzz-",
    "-fontconfig-",
    "-freetype-",
    "-libepoxy-",
    "-enchant-",
];

/// Pure filter body — split out so the keep/drop policy is unit-testable
/// without mutating process env. Returns `None` when nothing was dropped.
#[cfg(target_os = "linux")]
fn filter_nix_gui_stack(current: &str) -> Option<String> {
    let kept: Vec<&str> = current
        .split(':')
        .filter(|entry| {
            !(entry.starts_with("/nix/store/")
                && NIX_GUI_STACK_FRAGMENTS
                    .iter()
                    .any(|fragment| entry.contains(fragment)))
        })
        .collect();
    let filtered = kept.join(":");
    (filtered != current).then_some(filtered)
}

#[cfg(target_os = "linux")]
fn scrub_nix_gui_stack_from_ld_library_path() {
    let Ok(current) = std::env::var("LD_LIBRARY_PATH") else {
        return;
    };
    if current.is_empty() {
        return;
    }
    if let Some(filtered) = filter_nix_gui_stack(&current) {
        // Note: runs before the tracing subscriber is installed, so this
        // event is dropped in normal startup — it fires only for callers
        // that re-run setup after init. The scrub itself is what matters.
        tracing::info!(
            was = %current,
            "scrubbed Nix GUI-stack entries from LD_LIBRARY_PATH for webview child processes"
        );
        std::env::set_var("LD_LIBRARY_PATH", filtered);
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::filter_nix_gui_stack;

    #[test]
    fn drops_nix_gui_stack_keeps_host_and_runtime_deps() {
        let input = concat!(
            "/nix/store/4i6mqkahmcjamn3gy5ni5r8hjidf7srh-webkitgtk-2.50.4+abi=4.1/lib:",
            "/nix/store/wy4c9khmxwp1vd2p6nbf1lpg0rpnk61v-glib-2.86.3/lib:",
            "/nix/store/dw1l57pjcr8ysf3r7xpx06mm8gg1xicd-gstreamer-1.26.5/lib:",
            "/nix/store/abc123-libsodium-1.0.20/lib:",
            "/nix/store/def456-libopus-1.5.2/lib:",
            "/usr/local/lib"
        );
        let filtered = filter_nix_gui_stack(input).expect("GUI entries must be dropped");
        assert_eq!(
            filtered,
            "/nix/store/abc123-libsodium-1.0.20/lib:/nix/store/def456-libopus-1.5.2/lib:/usr/local/lib",
            "REKINDLE_LIB_PATH deps and host paths pass through"
        );
    }

    #[test]
    fn clean_path_is_left_untouched() {
        assert_eq!(filter_nix_gui_stack("/usr/lib:/opt/lib"), None);
        assert_eq!(
            filter_nix_gui_stack("/nix/store/abc-libsodium-1.0.20/lib"),
            None
        );
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
