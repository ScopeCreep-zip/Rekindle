use super::{panic_shortcut, set_panic_shortcut};
use crate::repo::fixture::{self, OWNER};

#[test]
fn the_panic_shortcut_round_trips_and_refuses_a_malformed_accelerator() {
    let conn = fixture::conn();
    assert_eq!(panic_shortcut(&conn, OWNER).unwrap(), None);
    set_panic_shortcut(&conn, OWNER, Some("CommandOrControl+Shift+F12")).unwrap();
    assert_eq!(
        panic_shortcut(&conn, OWNER).unwrap().as_deref(),
        Some("CommandOrControl+Shift+F12")
    );
    for bad in ["F12", "Ctrl+", "Ctrl+Shift+F12; DROP", "+X"] {
        assert!(
            set_panic_shortcut(&conn, OWNER, Some(bad)).is_err(),
            "{bad}"
        );
    }
    set_panic_shortcut(&conn, OWNER, None).unwrap();
    assert_eq!(panic_shortcut(&conn, OWNER).unwrap(), None);
}
