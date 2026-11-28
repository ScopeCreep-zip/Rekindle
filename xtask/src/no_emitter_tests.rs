use super::scan;

#[test]
fn flags_emitter_imports_and_method_calls() {
    let bad = r#"
            use tauri::{Emitter, Manager};
            fn f(app: tauri::AppHandle, w: tauri::WebviewWindow) {
                let _ = app.emit("x", ());
                let _ = w.emit_to("buddy-list", "x", ());
                let _ = <tauri::AppHandle as tauri::Emitter>::emit_str(&app, "x", String::new());
            }
        "#;
    let hits = scan("f.rs", bad);
    assert_eq!(hits.len(), 4, "{hits:?}");
    assert!(hits[0].starts_with("f.rs:2 "), "{hits:?}");
}

#[test]
fn allows_the_router_entry_points() {
    let ok = r"
            fn f(app: &tauri::AppHandle) {
                crate::event_dispatch::emit(app, crate::event_dispatch::WebviewEvent::ProfileUpdated);
                emit_journaled(state, event);
                channel.send(envelope);
            }
        ";
    assert!(scan("f.rs", ok).is_empty());
}
