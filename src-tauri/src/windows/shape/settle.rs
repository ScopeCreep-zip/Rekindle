//! Plan a Wayland margin update from an actual matching GTK allocation.
//! The adapter performs the setters and confirms another allocation before
//! publishing. A planned canvas is never itself a published allocation.

use std::time::{Duration, Instant};

use super::logical::Margins;

pub(super) const SIZE_WAIT: Duration = Duration::from_millis(250);
#[cfg(target_os = "linux")]
pub(super) const PAINT_WAIT: Duration = Duration::from_millis(500);
const MAX_TRANSITIONS: usize = 4;

type Canvas = (i32, i32);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ShapeSettleError {
    InvalidAllocation,
    RepeatedTransition,
    TransitionLimit,
    #[cfg(target_os = "linux")]
    SizeRefused,
    #[cfg(target_os = "linux")]
    PaintTimeout,
    PaintMismatch,
    #[cfg(target_os = "linux")]
    InvalidSurface,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AllocationDecision {
    Await,
    Accept { override_size: bool },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MarginUpdate {
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
    paint_retry: bool,
}

impl Settling {
    pub fn select(&mut self, target: Canvas, fixed: bool, now: Instant) -> Option<Canvas> {
        self.reset_transition();
        self.paint_retry = false;
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
        self.paint_retry = false;
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
    ) -> AllocationDecision {
        if canvas.0 <= 0 || canvas.1 <= 0 || canvas != root {
            return AllocationDecision::Await;
        }
        if let Some(request) = self
            .requested
            .filter(|request| canvas != request.target && !fixed && self.expected.is_none())
        {
            return if now >= request.deadline {
                AllocationDecision::Accept {
                    override_size: true,
                }
            } else {
                AllocationDecision::Await
            };
        }
        AllocationDecision::Accept {
            override_size: self.requested.is_some_and(|r| canvas != r.target),
        }
    }

    pub fn update(
        &mut self,
        canvas: Canvas,
        desired: Margins,
        fixed: bool,
    ) -> Result<MarginUpdate, ShapeSettleError> {
        if self.margins == desired {
            return Ok(MarginUpdate::Ready);
        }
        let visible = self.margins.visible(canvas);
        if visible.0 <= 0 || visible.1 <= 0 {
            return Err(ShapeSettleError::InvalidAllocation);
        }
        let transition = (canvas, desired);
        if self.transitions.contains(&transition) {
            return Err(ShapeSettleError::RepeatedTransition);
        }
        if self.transitions.len() == MAX_TRANSITIONS {
            return Err(ShapeSettleError::TransitionLimit);
        }
        self.transitions.push(transition);
        let restore =
            (!fixed && !self.corrected && self.requested.is_some_and(|r| r.target == canvas))
                .then_some(canvas);
        self.corrected |= restore.is_some();
        let expected = desired.canvas(visible);
        self.expected = Some(restore.unwrap_or(expected));
        self.margins = desired;
        Ok(MarginUpdate::SetMargins { expected, restore })
    }

    pub fn applied(&mut self) {
        self.requested = None;
        self.reset_transition();
    }

    /// A root paint can see a newer native allocation. Allow one resample.
    pub fn painted(&mut self, matches: bool) -> Result<bool, ShapeSettleError> {
        if matches {
            self.paint_retry = false;
            return Ok(true);
        }
        if std::mem::replace(&mut self.paint_retry, true) {
            return Err(ShapeSettleError::PaintMismatch);
        }
        Ok(false)
    }

    pub fn degraded(&mut self) {
        self.requested = None;
        self.deferred = None;
        self.reset_surface();
    }

    pub fn reset_surface(&mut self) {
        self.margins = Margins::default();
        self.paint_retry = false;
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
