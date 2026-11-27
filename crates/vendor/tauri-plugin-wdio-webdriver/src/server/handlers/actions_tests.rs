use super::*;

fn parse(json: &str) -> ActionsRequest {
    serde_json::from_str(json).expect("valid actions request")
}

#[test]
fn action_count_returns_actions_len_per_source() {
    let req = parse(
        r#"{"actions":[
                {"type":"key","id":"k","actions":[{"type":"keyDown","value":"a"},{"type":"keyUp","value":"a"}]},
                {"type":"none","id":"n","actions":[{"type":"pause","duration":0}]}
            ]}"#,
    );
    let counts: Vec<usize> = req
        .actions
        .iter()
        .map(ActionSequence::action_count)
        .collect();
    assert_eq!(counts, vec![2, 1]);
}

#[test]
fn tick_count_is_the_max_actions_across_sources() {
    // The number of ticks is max(actions) across sources, so a shorter source contributes
    // nothing on later ticks while a longer one keeps going.
    let req = parse(
        r#"{"actions":[
                {"type":"key","id":"k","actions":[{"type":"keyDown","value":"a"}]},
                {"type":"pointer","id":"p","actions":[
                    {"type":"pointerMove","x":0,"y":0},{"type":"pointerDown","button":0},{"type":"pointerUp","button":0}
                ]}
            ]}"#,
    );
    let ticks = req
        .actions
        .iter()
        .map(ActionSequence::action_count)
        .max()
        .unwrap_or(0);
    assert_eq!(ticks, 3);
}

#[test]
fn tick_count_is_zero_for_no_sources() {
    let req = parse(r#"{"actions":[]}"#);
    let ticks = req
        .actions
        .iter()
        .map(ActionSequence::action_count)
        .max()
        .unwrap_or(0);
    assert_eq!(ticks, 0);
}

#[test]
fn tick_duration_is_max_across_sources_not_sum() {
    // W3C §17.4.3: a tick lasts the max pause/move/scroll duration across sources, not their sum.
    let req = parse(
        r#"{"actions":[
                {"type":"none","id":"n","actions":[{"type":"pause","duration":100},{"type":"pause","duration":50}]},
                {"type":"pointer","id":"p","actions":[
                    {"type":"pointerMove","x":0,"y":0,"duration":200},{"type":"pause","duration":30}
                ]},
                {"type":"wheel","id":"w","actions":[{"type":"scroll","x":0,"y":0,"deltaX":0,"deltaY":0,"duration":80}]}
            ]}"#,
    );
    let tick_ms = |tick: usize| {
        req.actions
            .iter()
            .filter_map(|s| s.duration_at(tick))
            .max()
            .unwrap_or(0)
    };
    // tick 0: max(100, 200, 80) = 200 — not the sum (380).
    assert_eq!(tick_ms(0), 200);
    // tick 1: max(50, 30) = 50 — the wheel source is exhausted and contributes nothing.
    assert_eq!(tick_ms(1), 50);
}
