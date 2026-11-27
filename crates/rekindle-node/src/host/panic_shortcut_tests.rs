use super::*;

/// Every session kind with and without the portal (Linux finding: the
/// X11 grab reported armed on GNOME Wayland and COSMIC, and never
/// fired).
#[test]
fn the_route_follows_the_session_and_the_portal() {
    for portal in [false, true] {
        assert_eq!(route(DesktopSession::Native, portal), PanicRoute::OsGrab);
        assert_eq!(route(DesktopSession::X11, portal), PanicRoute::OsGrab);
    }
    assert_eq!(route(DesktopSession::Wayland, true), PanicRoute::Portal);
    assert_eq!(
        route(DesktopSession::Wayland, false),
        PanicRoute::Unavailable(UNAVAILABLE)
    );
}

#[test]
fn a_compositor_socket_or_the_session_type_means_wayland() {
    use DesktopSession::{Wayland, X11};
    let cases = [
        (Some("wayland"), None, Wayland),
        (Some("x11"), Some("wayland-0"), Wayland),
        (None, Some("wayland-1"), Wayland),
        (Some("x11"), None, X11),
        (Some("tty"), Some(""), X11),
        (None, None, X11),
    ];
    for (session_type, display, expected) in cases {
        assert_eq!(
            DesktopSession::from_linux_env(session_type, display),
            expected,
            "{session_type:?} {display:?}"
        );
    }
}
