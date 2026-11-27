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
