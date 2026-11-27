use super::*;

#[test]
fn a_second_unconfirmed_root_paint_is_a_failure() {
    let mut settling = Settling::default();
    assert_eq!(settling.painted(false), Ok(false));
    // An allocation pass alone cannot prove the root painted at its size.
    settling.applied();
    assert_eq!(
        settling.painted(false),
        Err(ShapeSettleError::PaintMismatch)
    );
    settling.reset_surface();
    assert_eq!(settling.painted(false), Ok(false));
    assert_eq!(settling.painted(true), Ok(true));
    assert_eq!(settling.painted(false), Ok(false));
}

fn left(value: i32) -> Margins {
    Margins {
        left: value,
        ..Margins::default()
    }
}

#[test]
fn an_explicit_switch_waits_for_the_real_canvas_and_corrects_the_delta_once() {
    let now = Instant::now();
    let mut state = Settling::default();
    assert_eq!(state.select((400, 650), false, now), Some((400, 650)));
    assert_eq!(
        state.allocation((333, 555), (333, 555), false, now),
        AllocationDecision::Await
    );
    assert_eq!(
        state.allocation((400, 650), (333, 555), false, now),
        AllocationDecision::Await
    );
    assert_eq!(
        state.allocation((400, 650), (400, 650), false, now),
        AllocationDecision::Accept {
            override_size: false
        }
    );
    assert_eq!(
        state.update((400, 650), left(112), false),
        Ok(MarginUpdate::SetMargins {
            expected: (512, 650),
            restore: Some((400, 650))
        })
    );
    assert_eq!(
        state.update((400, 650), left(112), false),
        Ok(MarginUpdate::Ready)
    );
    assert_eq!(
        state.update((400, 650), left(110), false),
        Ok(MarginUpdate::SetMargins {
            expected: (398, 650),
            restore: None
        })
    );
    state.applied();
    assert_eq!(state.select((333, 555), false, now), Some((333, 555)));
    assert_eq!(
        state.allocation((340, 650), (340, 650), false, now + SIZE_WAIT),
        AllocationDecision::Accept {
            override_size: true
        }
    );
}

#[test]
fn a_fixed_canvas_preserves_visible_extent_and_reissues_the_deferred_size_once() {
    let now = Instant::now();
    let mut state = Settling::default();
    assert_eq!(state.select((400, 650), true, now), None);
    assert_eq!(
        state.update((288, 650), left(112), true),
        Ok(MarginUpdate::SetMargins {
            expected: (400, 650),
            restore: None
        })
    );
    assert_eq!(
        state.allocation((288, 650), (400, 650), true, now),
        AllocationDecision::Await
    );
    assert_eq!(
        state.update((400, 650), left(112), true),
        Ok(MarginUpdate::Ready)
    );
    state.applied();
    assert_eq!(state.state_changed(false, now), Some((400, 650)));
    assert_eq!(state.state_changed(false, now), None);
}

#[test]
fn repeated_or_unbounded_margin_feedback_is_reported() {
    let mut state = Settling::default();
    for (canvas, margins) in [(300, 12), (312, 13), (313, 12), (312, 13)] {
        let result = state.update((canvas, 400), left(margins), true);
        if (canvas, margins) == (312, 13) && state.transitions.len() == 3 {
            assert_eq!(result, Err(ShapeSettleError::RepeatedTransition));
        } else {
            assert!(matches!(result, Ok(MarginUpdate::SetMargins { .. })));
        }
    }
    assert!(state.update((313, 400), left(15), true).is_ok());
    assert_eq!(
        state.update((315, 400), left(16), true),
        Err(ShapeSettleError::TransitionLimit)
    );
    state.reset_surface();
    assert_eq!(
        state.update((300, 400), Margins::default(), true),
        Ok(MarginUpdate::Ready)
    );
}

#[test]
fn empty_or_mismatched_allocations_never_publish_and_a_new_surface_reapplies() {
    let now = Instant::now();
    let mut state = Settling::default();
    assert_eq!(
        state.allocation((0, 0), (0, 0), false, now),
        AllocationDecision::Await
    );
    assert_eq!(
        state.allocation((320, 650), (321, 650), false, now),
        AllocationDecision::Await
    );
    assert!(matches!(
        state.update((400, 650), left(112), true),
        Ok(MarginUpdate::SetMargins { .. })
    ));
    assert_eq!(
        state.update((400, 650), left(112), true),
        Ok(MarginUpdate::Ready)
    );
    state.reset_surface();
    assert!(matches!(
        state.update((400, 650), left(112), true),
        Ok(MarginUpdate::SetMargins { .. })
    ));
}

#[test]
fn failure_and_timeout_recovery_clear_the_previous_request_and_margins() {
    let now = Instant::now();
    let mut state = Settling::default();
    state.select((400, 650), false, now);
    state.update((400, 650), left(112), false).unwrap();
    state.degraded();
    assert_eq!(
        state.allocation((320, 650), (320, 650), false, now),
        AllocationDecision::Accept {
            override_size: false
        }
    );
    assert_eq!(
        state.update((320, 650), Margins::default(), false),
        Ok(MarginUpdate::Ready)
    );
    state.select((400, 650), true, now);
    state.degraded();
    assert_eq!(state.state_changed(false, now), None);
}
