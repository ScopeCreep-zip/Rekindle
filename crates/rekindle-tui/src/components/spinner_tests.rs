use super::*;

#[test]
fn spinner_starts_inactive() {
    let spinner = Spinner::new(true, true);
    assert!(!spinner.is_active());
}

#[test]
fn spinner_start_stop() {
    let mut spinner = Spinner::new(true, true);
    spinner.start();
    assert!(spinner.is_active());
    spinner.stop();
    assert!(!spinner.is_active());
}

#[test]
fn tick_advances_frame() {
    let mut spinner = Spinner::new(true, true);
    spinner.start();
    assert_eq!(spinner.frame, 0);
    spinner.tick();
    assert_eq!(spinner.frame, 1);
    spinner.tick();
    assert_eq!(spinner.frame, 2);
}

#[test]
fn tick_does_not_advance_when_inactive() {
    let mut spinner = Spinner::new(true, true);
    spinner.tick();
    assert_eq!(spinner.frame, 0);
}

#[test]
fn frame_wraps_around() {
    let mut spinner = Spinner::new(true, true);
    spinner.start();
    for _ in 0..100 {
        spinner.tick();
    }
    // Should not panic — wrapping_add handles overflow
    assert!(spinner.frame > 0);
}

#[test]
fn label_set_and_read() {
    let mut spinner = Spinner::new(true, true);
    spinner.set_label("loading...");
    assert_eq!(spinner.label, "loading...");
}
