//! Plan a Wayland margin update from an actual matching GTK allocation.
//! The adapter performs the setters and confirms another allocation before
//! publishing. A planned canvas is never itself a published allocation.

use std::time::{Duration, Instant};

use super::logical::Margins;

pub(super) const SIZE_WAIT: Duration = Duration::from_millis(250);
pub(super) const PAINT_WAIT: Duration = Duration::from_millis(500);
const MAX_TRANSITIONS: usize = 4;

type Canvas = (i32, i32);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Failure {
    InvalidAllocation,
    RepeatedTransition,
    TransitionLimit,
    SizeRefused,
    PaintTimeout,
    InvalidSurface,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Allocation {
    Await,
    Accept { override_size: bool },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Update {
    Ready,
    SetMargins {
        expected: Canvas,
        restore: Option<Canvas>,
    },
}

#[derive(Clone, Copy)]
struct SizeRequest {
    target: Canvas,
    deadline: Instant,
}

#[derive(Default)]
pub(super) struct Settling {
    margins: Margins,
    requested: Option<SizeRequest>,
    deferred: Option<Canvas>,
    transitions: Vec<(Canvas, Margins)>,
    expected: Option<Canvas>,
    corrected: bool,
}

impl Settling {
    pub fn select(&mut self, target: Canvas, fixed: bool, now: Instant) -> Option<Canvas> {
        self.reset_transition();
        self.requested = None;
        self.deferred = fixed.then_some(target);
        if fixed {
            return None;
        }
        self.requested = Some(SizeRequest {
            target,
            deadline: now + SIZE_WAIT,
        });
        Some(target)
    }

    pub fn state_changed(&mut self, fixed: bool, now: Instant) -> Option<Canvas> {
        self.reset_transition();
        if fixed {
            if let Some(request) = self.requested.take() {
                self.deferred = Some(request.target);
            }
            return None;
        }
        let target = self.deferred.take()?;
        self.select(target, false, now)
    }

    pub fn allocation(
        &self,
        canvas: Canvas,
        root: Canvas,
        fixed: bool,
        now: Instant,
    ) -> Allocation {
        if canvas.0 <= 0 || canvas.1 <= 0 || canvas != root {
            return Allocation::Await;
        }
        if let Some(request) = self.requested {
            if canvas != request.target && !fixed && self.expected.is_none() {
                return if now >= request.deadline {
                    Allocation::Accept {
                        override_size: true,
                    }
                } else {
                    Allocation::Await
                };
            }
        }
        Allocation::Accept {
            override_size: self.requested.is_some_and(|r| canvas != r.target),
        }
    }

    pub fn update(
        &mut self,
        canvas: Canvas,
        desired: Margins,
        fixed: bool,
    ) -> Result<Update, Failure> {
        if self.margins == desired {
            return Ok(Update::Ready);
        }
        let visible = self.margins.visible(canvas);
        if visible.0 <= 0 || visible.1 <= 0 {
            return Err(Failure::InvalidAllocation);
        }
        let transition = (canvas, desired);
        if self.transitions.contains(&transition) {
            return Err(Failure::RepeatedTransition);
        }
        if self.transitions.len() == MAX_TRANSITIONS {
            return Err(Failure::TransitionLimit);
        }
        self.transitions.push(transition);
        let restore =
            (!fixed && !self.corrected && self.requested.is_some_and(|r| r.target == canvas))
                .then_some(canvas);
        self.corrected |= restore.is_some();
        let expected = desired.canvas(visible);
        self.expected = Some(restore.unwrap_or(expected));
        self.margins = desired;
        Ok(Update::SetMargins { expected, restore })
    }

    pub fn applied(&mut self) {
        self.requested = None;
        self.reset_transition();
    }

    pub fn degraded(&mut self) {
        self.requested = None;
        self.deferred = None;
        self.reset_surface();
    }

    pub fn reset_surface(&mut self) {
        self.margins = Margins::default();
        self.reset_transition();
    }

    fn reset_transition(&mut self) {
        self.expected = None;
        self.corrected = false;
        self.transitions.clear();
    }
}

#[cfg(test)]
#[path = "settle_tests.rs"]
mod tests;
