use super::{Signal, State};

#[test]
fn test_state_transitions() {
    // Tests based on the table in
    // https://datatracker.ietf.org/doc/html/draft-ietf-rmcat-gcc-02#section-5

    // Hold
    let hold = State::Hold;
    assert_eq!(hold.transition(Signal::Overuse), State::Decrease);
    assert_eq!(hold.transition(Signal::Normal), State::Increase);
    assert_eq!(hold.transition(Signal::Underuse), State::Hold);

    // Increase
    let increase = State::Increase;
    assert_eq!(increase.transition(Signal::Overuse), State::Decrease);
    assert_eq!(increase.transition(Signal::Normal), State::Increase);
    assert_eq!(increase.transition(Signal::Underuse), State::Hold);

    // Decrease
    let decrease = State::Decrease;
    assert_eq!(decrease.transition(Signal::Overuse), State::Decrease);
    assert_eq!(decrease.transition(Signal::Normal), State::Hold);
    assert_eq!(decrease.transition(Signal::Underuse), State::Hold);
}
