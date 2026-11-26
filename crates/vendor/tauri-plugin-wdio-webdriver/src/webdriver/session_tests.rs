use super::{ActionState, FrameId, PointerInputState, Session, SwitchWindowError};

fn input(buttons: u32, suppress_mouse_events: bool) -> PointerInputState {
    PointerInputState {
        buttons,
        suppress_mouse_events,
        chorded: false,
        clicks: false,
    }
}

fn chord(buttons: u32, suppress_mouse_events: bool) -> PointerInputState {
    PointerInputState {
        chorded: true,
        ..input(buttons, suppress_mouse_events)
    }
}

fn clicking(state: PointerInputState) -> PointerInputState {
    PointerInputState {
        clicks: true,
        ..state
    }
}

#[test]
fn held_buttons_use_the_dom_buttons_bitmask() {
    let mut state = ActionState::default();

    assert_eq!(
        state.press_pointer_button("mouse", 0),
        Some(input(1, false))
    );
    assert_eq!(
        state.press_pointer_button("mouse", 2),
        Some(chord(3, false))
    );
    assert_eq!(
        state.press_pointer_button("mouse", 1),
        Some(chord(7, false))
    );
    assert_eq!(state.pointer_input_state("mouse"), input(7, false));

    assert_eq!(
        state.release_pointer_button("mouse", 0),
        Some(clicking(chord(6, false)))
    );
    assert_eq!(
        state.release_pointer_button("mouse", 1),
        Some(chord(2, false))
    );
    assert_eq!(
        state.release_pointer_button("mouse", 2),
        Some(input(0, false))
    );
    assert_eq!(state.pointer_input_state("mouse"), input(0, false));
}

#[test]
fn held_buttons_are_tracked_per_source() {
    let mut state = ActionState::default();

    assert_eq!(state.press_pointer_button("a", 0), Some(input(1, false)));
    assert_eq!(state.press_pointer_button("b", 2), Some(input(2, false)));

    assert_eq!(state.pointer_input_state("a"), input(1, false));
    assert_eq!(state.pointer_input_state("b"), input(2, false));
}

#[test]
fn pressing_a_held_button_again_does_nothing() {
    let mut state = ActionState::default();
    state.press_pointer_button("mouse", 0);

    assert_eq!(state.press_pointer_button("mouse", 0), None);
    assert_eq!(state.pointer_input_state("mouse"), input(1, false));
}

#[test]
fn canceled_pointer_down_suppresses_mouse_events_through_the_final_release() {
    let mut state = ActionState::default();
    state.press_pointer_button("mouse", 0);
    state.cancel_pointer_down("mouse");

    assert_eq!(state.pointer_input_state("mouse"), input(1, true));
    assert_eq!(
        state.release_pointer_button("mouse", 0),
        Some(clicking(input(0, true)))
    );
    assert_eq!(state.pointer_input_state("mouse"), input(0, false));
    assert_eq!(
        state.press_pointer_button("mouse", 0),
        Some(input(1, false))
    );
}

#[test]
fn suppression_outlasts_a_release_that_leaves_a_button_held() {
    let mut state = ActionState::default();
    state.press_pointer_button("mouse", 0);
    state.cancel_pointer_down("mouse");

    assert_eq!(state.press_pointer_button("mouse", 2), Some(chord(3, true)));
    assert_eq!(
        state.release_pointer_button("mouse", 0),
        Some(clicking(chord(2, true)))
    );
    assert_eq!(state.pointer_input_state("mouse"), input(2, true));
    assert_eq!(
        state.release_pointer_button("mouse", 2),
        Some(input(0, true))
    );
    assert_eq!(state.pointer_input_state("mouse"), input(0, false));
}

#[test]
fn releasing_an_unpressed_button_does_nothing() {
    let mut state = ActionState::default();
    state.press_pointer_button("mouse", 0);
    state.cancel_pointer_down("mouse");

    assert_eq!(state.release_pointer_button("mouse", 2), None);
    assert_eq!(state.pointer_input_state("mouse"), input(1, true));
}

#[test]
fn a_primary_release_clicks_only_where_its_press_was() {
    let mut state = ActionState::default();
    state.press_pointer_button("mouse", 0);
    state.pointer_position = (5, 5);
    assert_eq!(
        state.release_pointer_button("mouse", 0),
        Some(input(0, false))
    );

    state.press_pointer_button("mouse", 0);
    assert_eq!(
        state.release_pointer_button("mouse", 0),
        Some(clicking(input(0, false)))
    );
}

#[test]
fn forgetting_pointer_input_lets_the_next_press_dispatch() {
    let mut state = ActionState::default();
    state.press_pointer_button("mouse", 0);
    state.cancel_pointer_down("mouse");

    state.forget_pointer_input();

    assert_eq!(
        state.press_pointer_button("mouse", 0),
        Some(input(1, false))
    );
}

#[test]
fn held_pointer_buttons_are_listed_by_source_then_button() {
    let mut state = ActionState::default();
    for (source_id, button) in [("b", 0), ("a", 2), ("b", 1), ("a", 0), ("a", 1)] {
        state.press_pointer_button(source_id, button);
    }

    let held = state.held_pointer_buttons();

    let expected = [("a", 0), ("a", 1), ("a", 2), ("b", 0), ("b", 1)];
    assert_eq!(
        held,
        expected.map(|(source_id, button)| (source_id.to_string(), button))
    );
}

#[test]
fn switching_windows_updates_context_and_generation_together() {
    let mut session = Session::new("child-left".to_string());
    session.frame_context.push(FrameId::Index(0));

    assert_eq!(session.switch_to_window("child-right".to_string()), Ok(()));

    assert_eq!(session.current_window, "child-right");
    assert!(session.frame_context.is_empty());
    assert_eq!(session.browsing_context_generation, 1);
}

#[test]
fn switching_windows_at_max_generation_leaves_session_unchanged() {
    let mut session = Session::new("child-left".to_string());
    session.frame_context.push(FrameId::Index(0));
    session.browsing_context_generation = u64::MAX;

    let result = session.switch_to_window("child-right".to_string());

    assert_eq!(
        result,
        Err(SwitchWindowError::BrowsingContextGenerationOverflow)
    );
    assert_eq!(session.current_window, "child-left");
    assert!(matches!(
        session.frame_context.as_slice(),
        [FrameId::Index(0)]
    ));
    assert_eq!(session.browsing_context_generation, u64::MAX);
}
