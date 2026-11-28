use super::{open, Build, Show};
use parking_lot::Mutex;
use std::sync::Arc;

type Log = Arc<Mutex<Vec<String>>>;

/// A builder and window that record the calls an open makes.
struct Fake(Log);

impl Build for Fake {
    type Window = Self;
    fn visible(self, visible: bool) -> Self {
        self.0.lock().push(format!("visible({visible})"));
        self
    }
    fn build(self) -> Result<Self, String> {
        self.0.lock().push("build".to_owned());
        Ok(self)
    }
}

impl Show for Fake {
    fn show(&self) -> tauri::Result<()> {
        self.0.lock().push("show".to_owned());
        Ok(())
    }
}

/// The window is built hidden and shown only after it is built and
/// prepared, so GTK first realizes it undecorated with its titlebar
/// set (the WL2 finding: built visible, it opened 37 px too tall).
#[test]
fn a_window_is_built_hidden_then_shown() {
    let log = Log::default();
    let window = open(Fake(log.clone()), |w| {
        w.0.lock().push("prepare".to_owned());
        true
    });
    assert!(window.is_ok());
    assert_eq!(*log.lock(), ["visible(false)", "build", "prepare", "show"]);
}

#[test]
fn a_managed_first_show_leaves_the_window_hidden() {
    let log = Log::default();
    open(Fake(log.clone()), |w| {
        w.0.lock().push("prepare".to_owned());
        false
    })
    .expect("build hidden");
    assert_eq!(*log.lock(), ["visible(false)", "build", "prepare"]);
}
