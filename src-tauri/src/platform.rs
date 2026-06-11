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
/// macOS has its own arm below (host-app TCC authorization); Windows'
/// WebView2 surfaces the capture prompt itself.
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

/// Enable in-webview camera/microphone capture on macOS (WKWebView).
///
/// WKWebView mints its camera/mic sandbox extensions from the HOST
/// app's TCC authorization — when it's missing at capture time, WebKit
/// logs "Could not create a 'com.apple.webkit.camera' sandbox
/// extension" and `getUserMedia` yields nothing. Establish the
/// authorization deterministically up front (Apple: "Requesting
/// Authorization for Media Capture on macOS") instead of relying on
/// WebKit's UIProcess to trigger the prompt mid-`getUserMedia`.
///
/// Notes:
/// - `Info.plist` must carry `NSCameraUsageDescription` /
///   `NSMicrophoneUsageDescription` (it does — tauri-build embeds it
///   into the dev binary and merges it at bundle time).
/// - Host TCC authorization is necessary but NOT sufficient: camera
///   frames are pulled by WebKit's sandboxed GPU helper, which resolves
///   its capture sandbox extension against the host's BUNDLE identity.
///   `tauri dev`'s bare binary has none, so even with host TCC
///   authorized + the per-origin prompt granted, `getUserMedia` rejects
///   with "No AVVideoCaptureSource device". Video capture on macOS
///   requires the bundled `.app` (`pnpm tauri build --debug`); confirmed
///   by tauri-apps/tauri#11951. See `docs/contributor/development.md`.
/// - In `tauri dev` the TCC grant is additionally attributed to the
///   RESPONSIBLE PROCESS — the terminal/IDE that launched the binary —
///   so the prompt names that app, not Rekindle.
/// - The pre-auth stays: in the bundled app it front-loads the TCC
///   prompt deterministically and surfaces denial state in the logs.
/// - The status precheck + `Once` guards keep this idempotent across
///   every window builder that calls it.
#[cfg(target_os = "macos")]
pub fn enable_webview_media_capture(_window: &tauri::WebviewWindow) {
    use objc2_av_foundation::{AVMediaTypeAudio, AVMediaTypeVideo};

    // SAFETY: extern NSString constant from AVFoundation — valid for
    // the process lifetime once the framework is loaded (tauri links it
    // transitively via WebKit/AppKit).
    let video = unsafe { AVMediaTypeVideo };
    // SAFETY: same extern-constant contract as `AVMediaTypeVideo` above.
    let audio = unsafe { AVMediaTypeAudio };
    request_av_authorization("camera", video);
    request_av_authorization("microphone", audio);
}

/// Check-then-request one AVFoundation media authorization, logging the
/// outcome under `rekindle_video::permissions`. Denied/Restricted warns
/// once per process (eight windows call this; one actionable line
/// beats eight copies).
#[cfg(target_os = "macos")]
fn request_av_authorization(
    kind: &'static str,
    media_type: Option<&'static objc2_av_foundation::AVMediaType>,
) {
    use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice};

    static DENIED_WARNED: std::sync::Once = std::sync::Once::new();

    let Some(media_type) = media_type else {
        tracing::warn!(
            target: "rekindle_video::permissions",
            kind,
            "AVFoundation media-type constant unavailable — capture authorization skipped"
        );
        return;
    };
    // SAFETY: class method on AVCaptureDevice with a valid media type.
    let status = unsafe { AVCaptureDevice::authorizationStatusForMediaType(media_type) };
    match status {
        AVAuthorizationStatus::Authorized => {
            tracing::debug!(target: "rekindle_video::permissions", kind, "capture authorized");
        }
        AVAuthorizationStatus::NotDetermined => {
            let handler = block2::RcBlock::new(move |granted: objc2::runtime::Bool| {
                if granted.as_bool() {
                    tracing::info!(
                        target: "rekindle_video::permissions",
                        kind,
                        "capture authorization granted"
                    );
                } else {
                    tracing::warn!(
                        target: "rekindle_video::permissions",
                        kind,
                        "capture authorization DENIED by the user — enable it in \
                         System Settings → Privacy & Security"
                    );
                }
            });
            // SAFETY: documented AVCaptureDevice request API; the
            // completion block is retained by AVFoundation until called.
            unsafe {
                AVCaptureDevice::requestAccessForMediaType_completionHandler(media_type, &handler);
            }
            tracing::info!(
                target: "rekindle_video::permissions",
                kind,
                "capture authorization requested (TCC prompt shown)"
            );
        }
        AVAuthorizationStatus::Denied | AVAuthorizationStatus::Restricted => {
            DENIED_WARNED.call_once(|| {
                tracing::warn!(
                    target: "rekindle_video::permissions",
                    kind,
                    ?status,
                    "capture access denied/restricted — System Settings → Privacy & \
                     Security → Camera/Microphone. Under `tauri dev` the grant belongs \
                     to the TERMINAL/IDE that launched the binary (TCC responsible \
                     process); `tccutil reset Camera` clears a half-granted state."
                );
            });
        }
        _ => {
            tracing::warn!(
                target: "rekindle_video::permissions",
                kind,
                ?status,
                "unknown AVFoundation authorization status"
            );
        }
    }
}

/// No-op on Windows — WebView2 surfaces the capture permission prompt
/// itself and TCC-style host authorization doesn't exist there.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn enable_webview_media_capture(_window: &tauri::WebviewWindow) {}
