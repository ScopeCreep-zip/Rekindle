use super::{forbidden, scan};

#[test]
fn flags_commands_that_take_shape_or_click_through_input() {
    let bad = r"
            #[tauri::command]
            pub fn set_window_region(rects: Vec<(i32, i32, u32, u32)>) {}
            #[tauri::command]
            pub async fn update_window(label: String, ignore_cursor: bool) {}
            #[command]
            fn skin_frame(state: State<'_, SharedState>, mask: Silhouette) {}
            #[tauri::command]
            fn toggle(state: State<'_, SharedState>, clickThrough: bool) {}
        ";
    let hits = scan("f.rs", bad);
    assert_eq!(hits.len(), 5, "{hits:?}");
    assert!(hits
        .iter()
        .any(|h| h.contains("`set_window_region`: name") && h.contains("window region")));
    assert!(hits.iter().any(|h| h.contains("ignore cursor")));
    assert!(hits.iter().any(|h| h.contains("\"silhouette\"")));
    assert!(hits.iter().any(|h| h.contains("click through")));
}

#[test]
fn allows_ordinary_commands_and_non_commands() {
    let ok = r"
            #[tauri::command]
            pub async fn send_message(state: State<'_, SharedState>, peer: String, body: String) {}
            #[tauri::command]
            pub fn start_resize_drag(direction: ResizeDirection) {}
            #[tauri::command]
            pub fn set_direct_mode(enabled: bool) {}
            #[tauri::command]
            pub async fn subscribe_events(after: Option<rekindle_types::subscription_events::Cursor>) {}
            #[tauri::command]
            pub async fn import_skin_region(region: String, fit: RegionFit) {}
            pub fn apply_shape(window: &WebviewWindow, rects: &[Rect]) {}
        ";
    assert!(scan("f.rs", ok).is_empty(), "{:?}", scan("f.rs", ok));
}

#[test]
fn words_split_camel_case_and_snake_case() {
    assert_eq!(
        forbidden("setIgnoreCursorEvents"),
        Some("ignore cursor".into())
    );
    assert_eq!(forbidden("window_shape"), Some("shape".into()));
    assert_eq!(forbidden("direct"), None);
    assert_eq!(forbidden("correct_rectangle"), None);
}

#[test]
fn checks_typed_host_forwarding_command_signatures() {
    let commands = r"
        host_command! {
            state;
            pub async fn set_topic(state: State<'_, SharedState>, topic: String) -> Result<()> {
                IpcRequest::SetTopic { topic }
            }
        }
        host_command! {
            state;
            pub async fn toggle(state: State<'_, SharedState>, click_through: bool) -> Result<()> {
                IpcRequest::Toggle { click_through }
            }
        }
    ";
    let hits = scan("command.rs", commands);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert!(hits[0].contains("`toggle`: parameter `click_through`"));
}
