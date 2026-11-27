use super::*;
use parking_lot::Mutex;

/// A window that records the calls a reveal makes.
struct Fake {
    calls: Mutex<Vec<&'static str>>,
}

impl Revealable for Fake {
    fn show(&self) -> tauri::Result<()> {
        self.calls.lock().push("show");
        Ok(())
    }
    fn unminimize(&self) -> tauri::Result<()> {
        self.calls.lock().push("unminimize");
        Ok(())
    }
    fn set_focus(&self) -> tauri::Result<()> {
        self.calls.lock().push("focus");
        Ok(())
    }
}

/// A tray-hidden window is shown, restored even when the desktop
/// reports no minimized state, then focused (the COSMIC finding: the tray's show left it hidden).
#[test]
fn a_hidden_window_is_shown_restored_then_focused() {
    let window = Fake {
        calls: Mutex::new(Vec::new()),
    };
    reveal(&window);
    assert_eq!(*window.calls.lock(), ["show", "unminimize", "focus"]);
}
