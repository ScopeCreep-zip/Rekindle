//! GTK allocation, safe Wayland margin application, and paint holds.

use std::cell::{Cell, RefCell};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use gtk::cairo::{RectangleInt, Region};
use gtk::gdk::{FrameClock, Window, WindowState};
use gtk::glib::{self, Propagation, SignalHandlerId, SourceId};
use gtk::prelude::{Cast, ContainerExt, GtkWindowExt, ObjectExt, WidgetExt};
use tauri::{LogicalSize, Manager, WebviewWindow};

use crate::windows::skin_load;

use super::super::logical::Margins;
use super::super::{
    desired, fitting, is_current, is_latest, logical::LogicalShape, publish, report_fit, settle,
    sizing, skin, Shaped,
};
use settle::{AllocationDecision, MarginUpdate, Settling, ShapeSettleError, PAINT_WAIT, SIZE_WAIT};

type Canvas = (i32, i32);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Stamp {
    incarnation: u64,
    serial: u64,
    canvas: Canvas,
    scale: i32,
    state: WindowState,
    region: u64,
}

struct PaintHold {
    root: Window,
    watchdog: Option<SourceId>,
}

impl Drop for PaintHold {
    fn drop(&mut self) {
        if let Some(watchdog) = self.watchdog.take() {
            watchdog.remove();
        }
        self.root.thaw_updates();
    }
}

#[derive(Clone, Copy)]
struct FailedAllocation {
    affected: [Canvas; 2],
    scale: i32,
    state: WindowState,
    serial: u64,
}

pub(super) struct Controller {
    target: WebviewWindow,
    shaped: Arc<Shaped>,
    wayland: bool,
    settling: RefCell<Settling>,
    hold: RefCell<Option<PaintHold>>,
    selected: Cell<u64>,
    fixed: Cell<bool>,
    incarnation: Cell<u64>,
    applied: Cell<Option<Stamp>>,
    degraded: Cell<bool>,
    failed: Cell<Option<FailedAllocation>>,
    csd_ready: Cell<bool>,
    next_paint: Cell<Option<Stamp>>,
    paint_handler: RefCell<Option<(FrameClock, SignalHandlerId)>>,
    layout_queued: Cell<bool>,
}

fn fixed(state: WindowState) -> bool {
    state.intersects(WindowState::MAXIMIZED | WindowState::FULLSCREEN | WindowState::TILED)
}

fn root_canvas(root: &Window) -> Canvas {
    (root.width(), root.height())
}

fn set_margins(root: &Window, margins: Margins) {
    root.set_shadow_width(margins.left, margins.right, margins.top, margins.bottom);
}

fn input(widget: &gtk::ApplicationWindow, shape: Option<&LogicalShape>) {
    let region = shape.map(|shape| {
        let rects: Vec<_> = shape
            .rects
            .iter()
            .map(|&(x, y, w, h)| RectangleInt::new(x, y, w, h))
            .collect();
        Region::create_rectangles(&rects)
    });
    widget.input_shape_combine_region(region.as_ref());
}

impl Controller {
    pub(super) fn new(target: WebviewWindow, shaped: Arc<Shaped>, wayland: bool) -> Self {
        Self {
            target,
            shaped,
            wayland,
            settling: RefCell::default(),
            hold: RefCell::default(),
            selected: Cell::new(0),
            fixed: Cell::default(),
            incarnation: Cell::default(),
            applied: Cell::default(),
            degraded: Cell::default(),
            failed: Cell::default(),
            csd_ready: Cell::new(!wayland),
            next_paint: Cell::default(),
            paint_handler: RefCell::default(),
            layout_queued: Cell::default(),
        }
    }

    pub(super) fn label(&self) -> &str {
        self.target.label()
    }

    pub(super) fn connect(self: &Rc<Self>, widget: &gtk::ApplicationWindow) {
        // tao installs a placeholder titlebar on Wayland. Clear it before
        // GTK can realize a client-decorated window and rewrite our margins.
        if self.wayland && widget.is_realized() {
            self.degrade(widget, ShapeSettleError::InvalidSurface);
        } else if self.wayland {
            widget.set_titlebar(None::<&gtk::Widget>);
        }
        let state = Rc::clone(self);
        widget.connect_realize(move |widget| state.realize(widget));
        let state = Rc::clone(self);
        widget.connect_size_allocate(move |widget, allocation| state.allocate(widget, allocation));
        if self.wayland {
            let state = Rc::clone(self);
            widget.connect_configure_event(move |widget, _| {
                // Let GtkWindow handle the configure before running layout.
                state.queue_layout(widget);
                false
            });
        }
        let state = Rc::clone(self);
        widget.connect_map(move |widget| {
            state.applied.set(None);
            state.stage(widget);
        });
        let state = Rc::clone(self);
        widget.connect_unmap(move |_| {
            state.suspend();
            skin_load::allocated(&state.target, None);
        });
        let state = Rc::clone(self);
        widget.connect_unrealize(move |_| {
            state.suspend();
            if let Some((clock, handler)) = state.paint_handler.borrow_mut().take() {
                clock.disconnect(handler);
            }
            state.csd_ready.set(!state.wayland);
            state.settling.borrow_mut().reset_surface();
        });
        widget.connect_scale_factor_notify(gtk::prelude::WidgetExt::queue_allocate);
        let state = Rc::clone(self);
        widget.connect_window_state_event(move |widget, event| {
            state.window_state(widget, event.new_window_state());
            Propagation::Proceed
        });
        self.stage(widget);
    }

    fn realize(self: &Rc<Self>, widget: &gtk::ApplicationWindow) {
        self.incarnation.set(self.incarnation.get() + 1);
        self.suspend();
        self.settling.borrow_mut().reset_surface();
        if self.wayland {
            let root = widget
                .window()
                .and_then(|root| root.downcast::<gdkwayland::WaylandWindow>().ok());
            match root {
                Some(root) if widget.titlebar().is_none() && !widget.is_decorated() => {
                    root.announce_csd();
                    self.csd_ready.set(true);
                }
                _ => self.degrade(widget, ShapeSettleError::InvalidSurface),
            }
        }
        if let Some(clock) = widget.frame_clock() {
            let pending = Rc::downgrade(self);
            let widget = widget.downgrade();
            let handler = clock.connect_after_paint(move |_| {
                if let (Some(controller), Some(widget)) = (pending.upgrade(), widget.upgrade()) {
                    controller.verify_paint(&widget);
                }
            });
            self.paint_handler.replace(Some((clock, handler)));
        }
        widget.queue_allocate();
    }

    fn verify_paint(&self, widget: &gtk::ApplicationWindow) {
        let (Some(stamp), Some(root)) = (self.next_paint.take(), widget.window()) else {
            return;
        };
        let measured = root_canvas(&root);
        let scale = root.scale_factor();
        tracing::debug!(
            label = self.label(),
            ?stamp,
            ?measured,
            scale,
            "painted GTK window allocation"
        );
        let matches = measured == stamp.canvas && scale == stamp.scale;
        let painted = self.settling.borrow_mut().painted(matches);
        match painted {
            Ok(true) => {}
            Ok(false) => {
                self.applied.set(None);
                widget.queue_allocate();
            }
            Err(reason) => self.degrade(widget, reason),
        }
    }

    fn suspend(&self) {
        let hold = self.hold.borrow_mut().take();
        drop(hold);
        self.applied.set(None);
        self.failed.set(None);
        self.next_paint.set(None);
    }

    fn window_state(self: &Rc<Self>, widget: &gtk::ApplicationWindow, state: WindowState) {
        let fixed = fixed(state);
        if self.fixed.replace(fixed) != fixed {
            let requested = self
                .settling
                .borrow_mut()
                .state_changed(fixed, Instant::now());
            if let Some(size) = requested {
                self.request(widget, size);
            }
        }
        widget.queue_allocate();
    }

    pub(super) fn stage(self: &Rc<Self>, widget: &gtk::ApplicationWindow) {
        let generation = desired(self.target.app_handle());
        if is_latest(self.target.app_handle(), &generation)
            && self.selected.replace(generation.serial) != generation.serial
        {
            let hold = self.hold.borrow_mut().take();
            drop(hold);
            self.failed.set(None);
            let sizing = sizing(&generation, self.shaped.frame);
            if let Err(error) = self.target.set_size_constraints(sizing.constraints()) {
                tracing::warn!(%error, label = self.label(), "GTK size constraints not set");
                self.degrade(widget, ShapeSettleError::SizeRefused);
                return;
            }
            let size = LogicalSize::new(sizing.wanted.0, sizing.wanted.1).cast::<i32>();
            let is_fixed = widget.window().is_some_and(|root| fixed(root.state()));
            self.fixed.set(is_fixed);
            let request = self.settling.borrow_mut().select(
                (size.width, size.height),
                is_fixed,
                Instant::now(),
            );
            if let Some(size) = request {
                self.request(widget, size);
            }
        }
        self.queue_layout(widget);
    }

    fn request(self: &Rc<Self>, widget: &gtk::ApplicationWindow, size: Canvas) {
        if let Some(root) = widget.window().filter(|_| widget.is_mapped()) {
            self.freeze(widget, &root);
        }
        widget.resize(size.0, size.1);
        let controller = Rc::downgrade(self);
        let widget = widget.downgrade();
        glib::timeout_add_local_once(SIZE_WAIT, move || {
            if let (Some(controller), Some(widget)) = (controller.upgrade(), widget.upgrade()) {
                controller.queue_layout(&widget);
            }
        });
    }

    fn queue_layout(self: &Rc<Self>, widget: &gtk::ApplicationWindow) {
        widget.queue_allocate();
        if !self.wayland || !widget.is_mapped() || self.layout_queued.replace(true) {
            return;
        }
        // GTK normally runs check_resize in the frame clock's layout phase.
        // An occluded Wayland surface can be waiting for wl_surface.frame.
        // Run the same GTK layout from the main loop without waiting for paint.
        let widget = widget.downgrade();
        let controller = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(controller) = controller.upgrade() {
                controller.layout_queued.set(false);
                if let Some(widget) = widget.upgrade().filter(WidgetExt::is_mapped) {
                    widget.check_resize();
                }
            }
        });
    }

    fn freeze(self: &Rc<Self>, widget: &gtk::ApplicationWindow, root: &Window) {
        if !self.wayland || self.hold.borrow().is_some() {
            return;
        }
        root.freeze_updates();
        let widget = widget.downgrade();
        let controller = Rc::downgrade(self);
        let watchdog = glib::timeout_add_local_once(PAINT_WAIT, move || {
            if let (Some(widget), Some(controller)) = (widget.upgrade(), controller.upgrade()) {
                if let Some(hold) = controller.hold.borrow_mut().as_mut() {
                    hold.watchdog = None;
                }
                controller.degrade(&widget, ShapeSettleError::PaintTimeout);
            }
        });
        self.hold.replace(Some(PaintHold {
            root: root.clone(),
            watchdog: Some(watchdog),
        }));
    }

    fn degrade(&self, widget: &gtk::ApplicationWindow, reason: ShapeSettleError) {
        let generation = desired(self.target.app_handle());
        let allocation = widget.allocation();
        if let Some(root) = widget.window() {
            if self.wayland && self.csd_ready.get() {
                set_margins(&root, Margins::default());
            }
            self.failed.set(Some(FailedAllocation {
                affected: [
                    (allocation.width(), allocation.height()),
                    root_canvas(&root),
                ],
                scale: widget.scale_factor(),
                state: root.state(),
                serial: generation.serial,
            }));
        }
        input(widget, None);
        self.settling.borrow_mut().degraded();
        let hold = self.hold.borrow_mut().take();
        drop(hold);
        self.next_paint.set(None);
        self.applied.set(None);
        self.degraded.set(true);
        widget.queue_draw();
        publish(&self.target, &self.shaped, &generation, None);
        tracing::error!(
            label = self.label(),
            generation = generation.serial,
            ?reason,
            "window_shape_degraded"
        );
        debug_assert!(!self.degraded.get(), "window_shape_degraded: {reason:?}");
    }

    fn allocate(self: &Rc<Self>, widget: &gtk::ApplicationWindow, allocation: &gtk::Allocation) {
        if !widget.is_mapped() || !self.csd_ready.get() || !is_current(&self.target, &self.shaped) {
            return;
        }
        let Some(root) = widget.window() else { return };
        let canvas = (allocation.width(), allocation.height());
        let scale = widget.scale_factor();
        let state = root.state();
        let generation = desired(self.target.app_handle());
        if is_latest(self.target.app_handle(), &generation)
            && self.selected.get() != generation.serial
        {
            self.stage(widget);
            return;
        }
        if self.failed.get().is_some_and(|failed| {
            failed.affected.contains(&canvas)
                && failed.scale == scale
                && failed.state == state
                && failed.serial == generation.serial
        }) {
            return;
        }
        self.degraded.set(false);
        if scale <= 0 || scale != root.scale_factor() {
            self.freeze(widget, &root);
            widget.queue_allocate();
            return;
        }
        let measured = root_canvas(&root);
        if self.wayland && canvas.0 > 0 && canvas.1 > 0 && canvas != measured {
            self.freeze(widget, &root);
            widget.queue_allocate();
            return;
        }
        let AllocationDecision::Accept { override_size } =
            self.settling
                .borrow()
                .allocation(canvas, measured, fixed(state), Instant::now())
        else {
            return;
        };
        let logical_canvas = (f64::from(canvas.0), f64::from(canvas.1));
        let fitted = fitting(&self.target, &self.shaped, &generation, logical_canvas);
        let logical = if let Some(fitted) = fitted.as_ref() {
            let size = LogicalSize::new(f64::from(canvas.0), f64::from(canvas.1))
                .to_physical(f64::from(scale));
            let projected = skin::compute(
                &fitted.frames,
                Some(self.shaped.frame),
                size,
                f64::from(scale),
            )
            .ok()
            .and_then(|shape| LogicalShape::project(&shape, canvas, scale));
            let Some(projected) = projected else {
                self.degrade(widget, ShapeSettleError::InvalidAllocation);
                return;
            };
            Some(projected)
        } else {
            None
        };
        let margins = logical
            .as_ref()
            .map_or_else(Margins::default, |shape| shape.margins);
        let mut hash = DefaultHasher::new();
        logical.hash(&mut hash);
        let stamp = Stamp {
            incarnation: self.incarnation.get(),
            serial: fitted.as_ref().map_or(0, |fit| fit.serial),
            canvas,
            scale,
            state,
            region: hash.finish(),
        };
        if self.wayland {
            let update = self
                .settling
                .borrow_mut()
                .update(canvas, margins, fixed(state));
            match update {
                Ok(MarginUpdate::Ready) => {}
                Ok(MarginUpdate::SetMargins { expected, restore }) => {
                    self.freeze(widget, &root);
                    set_margins(&root, margins);
                    let measured = root_canvas(&root);
                    tracing::trace!(
                        label = self.label(),
                        ?canvas,
                        ?margins,
                        ?expected,
                        ?measured,
                        ?restore,
                        "staged Wayland window margins"
                    );
                    if let Some(size) = restore.filter(|size| *size != measured) {
                        root.resize(size.0, size.1);
                    }
                    self.queue_layout(widget);
                    return;
                }
                Err(reason) => {
                    self.degrade(widget, reason);
                    return;
                }
            }
        }
        let changed = self.applied.get() != Some(stamp);
        if changed {
            self.freeze(widget, &root);
            input(widget, logical.as_ref());
            self.applied.set(Some(stamp));
            self.next_paint.set(Some(stamp));
            let bounds = logical.as_ref().map(|shape| shape.bounds);
            let region_rects = logical.as_ref().map_or(0, |shape| shape.rects.len());
            tracing::debug!(
                label = self.label(),
                backend = if self.wayland { "wayland" } else { "x11" },
                ?stamp,
                ?bounds,
                ?margins,
                region_rects,
                override_size,
                "applied GTK window allocation"
            );
        }
        self.settling.borrow_mut().applied();
        let hold = self.hold.borrow_mut().take();
        let thawed = hold.is_some();
        drop(hold);
        if changed || thawed {
            widget.queue_draw();
        }
        let fits = fitted.is_some();
        publish(&self.target, &self.shaped, &generation, fitted);
        report_fit(
            &self.target,
            &self.shaped,
            &generation,
            fits,
            logical_canvas,
        );
    }
}
