//! GTK owns the surface and runs the one allocation callback for both
//! Wayland and X11. X11 uses the widget input region and transparent alpha.
//! On Wayland Rekindle also owns the margins that produce xdg geometry.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gtk::prelude::*;
use tauri::WebviewWindow;

use super::{is_current, Shaped};

#[path = "gtk_allocation.rs"]
mod allocation;

thread_local! {
    static CONTROLLERS: RefCell<HashMap<String, Rc<allocation::Controller>>> = RefCell::default();
}

fn on_gtk_window(
    window: &WebviewWindow,
    work: impl FnOnce(gtk::ApplicationWindow) + Send + 'static,
) {
    let target = window.clone();
    if let Err(error) = window.run_on_main_thread(move || match target.gtk_window() {
        Ok(widget) => work(widget),
        Err(error) => tracing::warn!(%error, label = target.label(), "GTK window unavailable"),
    }) {
        tracing::warn!(%error, label = window.label(), "GTK allocation not queued");
    }
}

pub(super) fn attach(window: &WebviewWindow, shaped: &Arc<Shaped>) {
    let target = window.clone();
    let shaped = Arc::clone(shaped);
    on_gtk_window(window, move |widget| {
        if !is_current(&target, &shaped) {
            return;
        }
        let wayland = widget.display().is::<gdkwayland::WaylandDisplay>();
        let controller = Rc::new(allocation::Controller::new(target, shaped, wayland));
        controller.connect(&widget);
        CONTROLLERS.with(|controllers| {
            controllers
                .borrow_mut()
                .insert(controller.label().to_owned(), Rc::clone(&controller));
        });
        widget.connect_destroy(move |_| {
            CONTROLLERS.with(|controllers| {
                let mut controllers = controllers.borrow_mut();
                if controllers
                    .get(controller.label())
                    .is_some_and(|live| Rc::ptr_eq(live, &controller))
                {
                    controllers.remove(controller.label());
                }
            });
        });
    });
}

pub(super) fn queue(window: &WebviewWindow, shaped: &Arc<Shaped>) {
    let target = window.clone();
    let shaped = Arc::clone(shaped);
    on_gtk_window(window, move |widget| {
        if !is_current(&target, &shaped) {
            return;
        }
        let controller =
            CONTROLLERS.with(|controllers| controllers.borrow().get(target.label()).cloned());
        if let Some(controller) = controller {
            controller.stage(&widget);
        }
    });
}
