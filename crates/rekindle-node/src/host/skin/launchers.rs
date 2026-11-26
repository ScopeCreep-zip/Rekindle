//! Each open popup's launcher (plan SK2-N4) and the one rule every
//! selection read follows (plan E7.7 HSC1): a popup's selection is its
//! launcher's. A popup (Alloy's info view, opened by INFO→ from the buddy
//! list) shows its launcher's selection, so its buttons are judged and
//! pressed on that selection too, never on one the popup itself never sets.

use std::collections::BTreeMap;

/// Each open popup's launcher, by popup label (plan SK2-N4), as the shell
/// said when it opened the popup (`Host::skin_window_opened`).
#[derive(Debug, Default)]
pub(crate) struct Launchers(parking_lot::Mutex<BTreeMap<String, String>>);

impl Launchers {
    pub(crate) fn set(&self, popup: &str, launcher: Option<&str>) {
        let mut all = self.0.lock();
        match launcher {
            Some(l) => all.insert(popup.to_owned(), l.to_owned()),
            None => all.remove(popup),
        };
    }

    /// The window that launched `popup`, when it is an open popup.
    pub(crate) fn of(&self, popup: &str) -> Option<String> {
        self.0.lock().get(popup).cloned()
    }

    /// The window whose selection `window` shows and acts on: a popup's
    /// launcher, every other window itself. The signal sets
    /// (`signals::window_sets`), the action table (`actions::resolve`), its
    /// reads (`reads::load`, `rooms::load`) and the Infoview's `Selection`
    /// all read a window's selection through this.
    pub(crate) fn selection_window(&self, window: &str) -> String {
        self.of(window).unwrap_or_else(|| window.to_owned())
    }
}
