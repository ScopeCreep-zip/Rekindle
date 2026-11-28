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

#[test]
fn late_host_skin_leaves_the_full_reveal_budget_for_the_new_window() {
    use std::cell::Cell;
    use std::future::{poll_fn, Future};
    use std::pin::pin;
    use std::task::{Context, Poll, Waker};
    use std::time::{Duration, Instant};

    use crate::windows::skin_reveal::{
        Show as RevealShow, SkinRevealEffect, SkinRevealGate, LOAD_LIMIT,
    };

    let now = Instant::now();
    let time = Cell::new(now);
    let built = Cell::new(false);
    let resolved = Cell::new(false);
    let mut opening = pin!(super::after_skin(
        poll_fn(|_| if resolved.get() {
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }),
        || {
            built.set(true);
            let mut gate = SkinRevealGate::default();
            gate.request(time.get(), RevealShow::First);
            Ok(gate)
        },
    ));
    let mut context = Context::from_waker(Waker::noop());
    assert!(opening.as_mut().poll(&mut context).is_pending());
    time.set(now + LOAD_LIMIT + Duration::from_secs(1));
    assert!(
        !built.get(),
        "skin selection must precede window construction"
    );
    resolved.set(true);
    let Poll::Ready(Ok(mut gate)) = opening.poll(&mut context) else {
        panic!("the prepared skin should release window construction");
    };
    let applied = time.get() + LOAD_LIMIT.saturating_sub(Duration::from_millis(1));
    gate.allocate(Some("skin://current"), time.get());
    gate.commit("skin://current".to_owned(), applied);
    assert_eq!(
        gate.poll(applied),
        SkinRevealEffect::Skinned(RevealShow::First)
    );
}
