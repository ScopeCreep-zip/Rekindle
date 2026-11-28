//! How the quick wipe's panic shortcut can be delivered on this desktop
//! (plan E7.1). The policy is the backend's: a shell reports what its
//! desktop offers ([`DesktopSession`], whether the GlobalShortcuts portal
//! answers) and registers by the [`PanicRoute`] decided here, so no shell
//! reports a shortcut armed that it cannot deliver.
//!
//! - macOS, Windows and an X11 session: the OS grab (`global-hotkey`'s
//!   `RegisterEventHotKey`, `RegisterHotKey`, `XGrabKey`) reaches the app
//!   whichever window has focus.
//! - A Wayland session: `XGrabKey` reaches Xwayland, and a Wayland
//!   compositor delivers keys to X11 clients only while an X11 window has
//!   focus (Mutter; cosmic-comp's `xwayland_eavesdropping` keyboard mode
//!   defaults to `None`), so the grab "succeeds" and never fires. The only
//!   delivery is the `org.freedesktop.portal.GlobalShortcuts` portal (the
//!   user approves the binding in the compositor's dialog); without it the
//!   chord is refused and the duress passphrase stays the trigger.

/// The kind of desktop session the shell runs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopSession {
    /// macOS or Windows: the OS hot-key API.
    Native,
    /// A Linux X11 session: `XGrabKey` on the real X server.
    X11,
    /// A Linux Wayland session (a compositor socket): no grab reaches the
    /// app; only the portal can deliver a global shortcut.
    Wayland,
}

impl DesktopSession {
    /// The session from the variables a Linux session sets: Wayland when
    /// the session says so (`XDG_SESSION_TYPE=wayland`) or a compositor
    /// socket is named (`WAYLAND_DISPLAY`), whichever GDK backend the app's
    /// own windows use, since an X11 window under Xwayland gets no grab
    /// either. Anything else is X11.
    #[must_use]
    pub fn from_linux_env(session_type: Option<&str>, wayland_display: Option<&str>) -> Self {
        let wayland = session_type.is_some_and(|t| t.eq_ignore_ascii_case("wayland"))
            || wayland_display.is_some_and(|d| !d.is_empty());
        if wayland {
            Self::Wayland
        } else {
            Self::X11
        }
    }
}

/// Why a panic shortcut cannot be armed on this desktop, as the user sees
/// it in the arm's answer and the unlock's alert.
pub const UNAVAILABLE: &str =
    "Panic shortcut not available on this desktop: use the duress passphrase";

/// How the shell registers the panic shortcut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanicRoute {
    /// The OS grab (the global-shortcut plugin).
    OsGrab,
    /// The GlobalShortcuts portal: the compositor binds it with the user's
    /// approval and reports the trigger it bound.
    Portal,
    /// No delivery exists: the chord is refused, never shown as armed.
    Unavailable(&'static str),
}

/// Decide the route for `session`, given whether the GlobalShortcuts portal
/// answered on the session bus.
#[must_use]
pub fn route(session: DesktopSession, portal_offered: bool) -> PanicRoute {
    match (session, portal_offered) {
        (DesktopSession::Native | DesktopSession::X11, _) => PanicRoute::OsGrab,
        (DesktopSession::Wayland, true) => PanicRoute::Portal,
        (DesktopSession::Wayland, false) => PanicRoute::Unavailable(UNAVAILABLE),
    }
}

#[cfg(test)]
#[path = "panic_shortcut_tests.rs"]
mod tests;
