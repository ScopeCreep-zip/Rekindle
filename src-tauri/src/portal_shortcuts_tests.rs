use super::*;

/// The portal's preferred trigger is the stored chord in the XDG
/// shortcuts specification's names.
#[test]
fn the_accelerator_becomes_the_xdg_trigger() {
    assert_eq!(
        xdg_trigger("CommandOrControl+Shift+F12").unwrap(),
        "CTRL+SHIFT+F12"
    );
    assert_eq!(xdg_trigger("Super+Alt+KeyW").unwrap(), "LOGO+ALT+w");
    assert_eq!(xdg_trigger("Ctrl+Shift+Digit9").unwrap(), "CTRL+SHIFT+9");
    assert_eq!(xdg_trigger("Ctrl+Q").unwrap(), "CTRL+q");
    assert!(xdg_trigger("Hyper+F1").is_err());
    assert!(xdg_trigger("Ctrl+").is_err());
}

/// The compositor's description of what it bound is what the user is
/// shown; an answer without our shortcut binds nothing.
#[test]
fn the_bound_trigger_is_the_compositors_description() {
    let fields = HashMap::from([(
        "trigger_description".to_string(),
        OwnedValue::from(zbus::zvariant::Str::from("Ctrl+Shift+F12")),
    )]);
    let shortcuts = vec![(SHORTCUT_ID.to_string(), fields)];
    let results = HashMap::from([(
        "shortcuts".to_string(),
        OwnedValue::try_from(Value::from(shortcuts)).unwrap(),
    )]);
    assert_eq!(bound_trigger(&results).unwrap(), "Ctrl+Shift+F12");
    assert!(bound_trigger(&HashMap::new()).is_err());
}
