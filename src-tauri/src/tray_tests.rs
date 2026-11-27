use super::*;

#[test]
fn set_custom_status_opens_the_status_message_editor() {
    assert_eq!(
        TrayAction::of("status-message"),
        Some(TrayAction::OpenSettings(Some(
            windows::SettingsTab::Profile
        )))
    );
    assert_eq!(
        TrayAction::of("settings"),
        Some(TrayAction::OpenSettings(None))
    );
    assert_eq!(
        TrayAction::of("status-offline"),
        Some(TrayAction::SetStatus(SessionStatus::Invisible))
    );
    assert_eq!(TrayAction::of("show"), Some(TrayAction::Show));
    assert_eq!(TrayAction::of("unknown"), None);
}

#[test]
fn the_tray_icon_carries_the_status_dot_and_only_the_app_mark_is_a_template() {
    assert_eq!(TrayIcon::of(None), TrayIcon::LoggedOut);
    assert_eq!(TrayIcon::of(Some(SessionStatus::Online)), TrayIcon::Online);
    assert_eq!(TrayIcon::of(Some(SessionStatus::Away)), TrayIcon::Away);
    assert_eq!(TrayIcon::of(Some(SessionStatus::Busy)), TrayIcon::Busy);
    assert_eq!(
        TrayIcon::of(Some(SessionStatus::Invisible)),
        TrayIcon::Offline
    );
    assert_eq!(
        TrayIcon::of(Some(SessionStatus::Offline)),
        TrayIcon::Offline
    );
    let icons = [
        TrayIcon::Online,
        TrayIcon::Away,
        TrayIcon::Busy,
        TrayIcon::Offline,
    ];
    for icon in icons {
        assert!(!icon.is_template(), "{icon:?} would lose its dot colour");
        assert!(Image::from_bytes(icon.bytes()).is_ok(), "{icon:?} decodes");
    }
    assert!(TrayIcon::LoggedOut.is_template());
    // Four distinct pictures, one per status.
    for (i, a) in icons.iter().enumerate() {
        for b in &icons[i + 1..] {
            assert_ne!(a.bytes(), b.bytes());
        }
    }
}
