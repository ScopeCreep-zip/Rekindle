use std::sync::Arc;

use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use serde_json::json;
use tauri::Runtime;

use crate::platform::prelude::*;
use crate::server::response::{WebDriverErrorResponse, WebDriverResponse, WebDriverResult};
use crate::server::AppState;

// Element location lives in `element_find`; re-exported so routes keep one path.
pub use super::element_find::{
    find, find_all, find_all_from_element, find_from_element, FindElementRequest,
};

#[derive(Debug, Deserialize)]
pub struct SendKeysRequest {
    pub text: String,
}

/// POST `/session/{session_id}/element/{element_id}/click` - Click element
pub async fn click<R: Runtime + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path((session_id, element_id)): Path<(String, String)>,
) -> WebDriverResult {
    let sessions = state.sessions.read().await;
    let session = sessions.get(&session_id)?;

    let element = session
        .elements
        .get(&element_id)
        .ok_or_else(WebDriverErrorResponse::no_such_element)?;

    let js_var = element.js_ref.clone();
    let current_window = session.current_window.clone();
    let timeouts = session.timeouts.clone();
    let frame_context = session.frame_context.clone();
    drop(sessions);

    let executor = state.get_executor_for_window(&current_window, timeouts, frame_context)?;
    executor.click_element(&js_var).await?;

    Ok(WebDriverResponse::null())
}

/// POST `/session/{session_id}/element/{element_id}/clear` - Clear element
pub async fn clear<R: Runtime + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path((session_id, element_id)): Path<(String, String)>,
) -> WebDriverResult {
    let sessions = state.sessions.read().await;
    let session = sessions.get(&session_id)?;

    let element = session
        .elements
        .get(&element_id)
        .ok_or_else(WebDriverErrorResponse::no_such_element)?;

    let js_var = element.js_ref.clone();
    let current_window = session.current_window.clone();
    let timeouts = session.timeouts.clone();
    let frame_context = session.frame_context.clone();
    drop(sessions);

    let executor = state.get_executor_for_window(&current_window, timeouts, frame_context)?;
    executor.clear_element(&js_var).await?;

    Ok(WebDriverResponse::null())
}

/// POST `/session/{session_id}/element/{element_id}/value` - Send keys to element
pub async fn send_keys<R: Runtime + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path((session_id, element_id)): Path<(String, String)>,
    Json(request): Json<SendKeysRequest>,
) -> WebDriverResult {
    let sessions = state.sessions.read().await;
    let session = sessions.get(&session_id)?;

    let element = session
        .elements
        .get(&element_id)
        .ok_or_else(WebDriverErrorResponse::no_such_element)?;

    let js_var = element.js_ref.clone();
    let current_window = session.current_window.clone();
    let timeouts = session.timeouts.clone();
    let frame_context = session.frame_context.clone();
    drop(sessions);

    let executor = state.get_executor_for_window(&current_window, timeouts, frame_context)?;
    executor
        .send_keys_to_element(&js_var, &request.text)
        .await?;

    Ok(WebDriverResponse::null())
}

/// GET `/session/{session_id}/element/{element_id}/text` - Get element text
pub async fn get_text<R: Runtime + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path((session_id, element_id)): Path<(String, String)>,
) -> WebDriverResult {
    let sessions = state.sessions.read().await;
    let session = sessions.get(&session_id)?;

    let element = session
        .elements
        .get(&element_id)
        .ok_or_else(WebDriverErrorResponse::no_such_element)?;

    let js_var = element.js_ref.clone();
    let current_window = session.current_window.clone();
    let timeouts = session.timeouts.clone();
    let frame_context = session.frame_context.clone();
    drop(sessions);

    let executor = state.get_executor_for_window(&current_window, timeouts, frame_context)?;
    let text = executor.get_element_text(&js_var).await?;
    Ok(WebDriverResponse::success(text))
}

/// GET `/session/{session_id}/element/{element_id}/name` - Get element tag name
pub async fn get_tag_name<R: Runtime + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path((session_id, element_id)): Path<(String, String)>,
) -> WebDriverResult {
    let sessions = state.sessions.read().await;
    let session = sessions.get(&session_id)?;

    let element = session
        .elements
        .get(&element_id)
        .ok_or_else(WebDriverErrorResponse::no_such_element)?;

    let js_var = element.js_ref.clone();
    let current_window = session.current_window.clone();
    let timeouts = session.timeouts.clone();
    let frame_context = session.frame_context.clone();
    drop(sessions);

    let executor = state.get_executor_for_window(&current_window, timeouts, frame_context)?;
    let tag_name = executor.get_element_tag_name(&js_var).await?;
    Ok(WebDriverResponse::success(tag_name))
}

/// GET `/session/{session_id}/element/{element_id}/attribute/{name}` - Get element attribute
pub async fn get_attribute<R: Runtime + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path((session_id, element_id, name)): Path<(String, String, String)>,
) -> WebDriverResult {
    let sessions = state.sessions.read().await;
    let session = sessions.get(&session_id)?;

    let element = session
        .elements
        .get(&element_id)
        .ok_or_else(WebDriverErrorResponse::no_such_element)?;

    let js_var = element.js_ref.clone();
    let current_window = session.current_window.clone();
    let timeouts = session.timeouts.clone();
    let frame_context = session.frame_context.clone();
    drop(sessions);

    let executor = state.get_executor_for_window(&current_window, timeouts, frame_context)?;
    let attr = executor.get_element_attribute(&js_var, &name).await?;
    Ok(WebDriverResponse::success(attr))
}

/// GET `/session/{session_id}/element/{element_id}/property/{name}` - Get element property
pub async fn get_property<R: Runtime + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path((session_id, element_id, name)): Path<(String, String, String)>,
) -> WebDriverResult {
    let sessions = state.sessions.read().await;
    let session = sessions.get(&session_id)?;

    let element = session
        .elements
        .get(&element_id)
        .ok_or_else(WebDriverErrorResponse::no_such_element)?;

    let js_var = element.js_ref.clone();
    let current_window = session.current_window.clone();
    let timeouts = session.timeouts.clone();
    let frame_context = session.frame_context.clone();
    drop(sessions);

    let executor = state.get_executor_for_window(&current_window, timeouts, frame_context)?;
    let prop = executor.get_element_property(&js_var, &name).await?;
    Ok(WebDriverResponse::success(prop))
}

/// GET `/session/{session_id}/element/{element_id}/displayed` - Is element displayed
pub async fn is_displayed<R: Runtime + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path((session_id, element_id)): Path<(String, String)>,
) -> WebDriverResult {
    let sessions = state.sessions.read().await;
    let session = sessions.get(&session_id)?;

    let element = session
        .elements
        .get(&element_id)
        .ok_or_else(WebDriverErrorResponse::no_such_element)?;

    let js_var = element.js_ref.clone();
    let current_window = session.current_window.clone();
    let timeouts = session.timeouts.clone();
    let frame_context = session.frame_context.clone();
    drop(sessions);

    let executor = state.get_executor_for_window(&current_window, timeouts, frame_context)?;
    let displayed = executor.is_element_displayed(&js_var).await?;
    Ok(WebDriverResponse::success(displayed))
}

/// GET `/session/{session_id}/element/{element_id}/enabled` - Is element enabled
pub async fn is_enabled<R: Runtime + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path((session_id, element_id)): Path<(String, String)>,
) -> WebDriverResult {
    let sessions = state.sessions.read().await;
    let session = sessions.get(&session_id)?;

    let element = session
        .elements
        .get(&element_id)
        .ok_or_else(WebDriverErrorResponse::no_such_element)?;

    let js_var = element.js_ref.clone();
    let current_window = session.current_window.clone();
    let timeouts = session.timeouts.clone();
    let frame_context = session.frame_context.clone();
    drop(sessions);

    let executor = state.get_executor_for_window(&current_window, timeouts, frame_context)?;
    let enabled = executor.is_element_enabled(&js_var).await?;
    Ok(WebDriverResponse::success(enabled))
}

/// GET `/session/{session_id}/element/active` - Get active element
pub async fn get_active<R: Runtime + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path(session_id): Path<String>,
) -> WebDriverResult {
    let mut sessions = state.sessions.write().await;
    let session = sessions.get_mut(&session_id)?;

    // Store element reference for the active element
    let element_ref = session.elements.store();
    let js_var = element_ref.js_ref.clone();
    let element_id = element_ref.id.clone();
    let current_window = session.current_window.clone();
    let timeouts = session.timeouts.clone();
    let frame_context = session.frame_context.clone();
    drop(sessions);

    let executor = state.get_executor_for_window(&current_window, timeouts, frame_context)?;
    let found = executor.get_active_element(&js_var).await?;
    if !found {
        return Err(WebDriverErrorResponse::no_such_element());
    }

    Ok(WebDriverResponse::success(json!({
        "element-6066-11e4-a52e-4f735466cecf": element_id
    })))
}

/// GET `/session/{session_id}/element/{element_id}/selected` - Is element selected
pub async fn is_selected<R: Runtime + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path((session_id, element_id)): Path<(String, String)>,
) -> WebDriverResult {
    let sessions = state.sessions.read().await;
    let session = sessions.get(&session_id)?;

    let element = session
        .elements
        .get(&element_id)
        .ok_or_else(WebDriverErrorResponse::no_such_element)?;

    let js_var = element.js_ref.clone();
    let current_window = session.current_window.clone();
    let timeouts = session.timeouts.clone();
    let frame_context = session.frame_context.clone();
    drop(sessions);

    let executor = state.get_executor_for_window(&current_window, timeouts, frame_context)?;
    let selected = executor.is_element_selected(&js_var).await?;
    Ok(WebDriverResponse::success(selected))
}

/// GET `/session/{session_id}/element/{element_id}/css/{property_name}` - Get CSS value
pub async fn get_css_value<R: Runtime + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path((session_id, element_id, property_name)): Path<(String, String, String)>,
) -> WebDriverResult {
    let sessions = state.sessions.read().await;
    let session = sessions.get(&session_id)?;

    let element = session
        .elements
        .get(&element_id)
        .ok_or_else(WebDriverErrorResponse::no_such_element)?;

    let js_var = element.js_ref.clone();
    let current_window = session.current_window.clone();
    let timeouts = session.timeouts.clone();
    let frame_context = session.frame_context.clone();
    drop(sessions);

    let executor = state.get_executor_for_window(&current_window, timeouts, frame_context)?;
    let value = executor
        .get_element_css_value(&js_var, &property_name)
        .await?;
    Ok(WebDriverResponse::success(value))
}

/// GET `/session/{session_id}/element/{element_id}/rect` - Get element rect
pub async fn get_rect<R: Runtime + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path((session_id, element_id)): Path<(String, String)>,
) -> WebDriverResult {
    let sessions = state.sessions.read().await;
    let session = sessions.get(&session_id)?;

    let element = session
        .elements
        .get(&element_id)
        .ok_or_else(WebDriverErrorResponse::no_such_element)?;

    let js_var = element.js_ref.clone();
    let current_window = session.current_window.clone();
    let timeouts = session.timeouts.clone();
    let frame_context = session.frame_context.clone();
    drop(sessions);

    let executor = state.get_executor_for_window(&current_window, timeouts, frame_context)?;
    let rect = executor.get_element_rect(&js_var).await?;
    Ok(WebDriverResponse::success(json!({
        "x": rect.x,
        "y": rect.y,
        "width": rect.width,
        "height": rect.height
    })))
}

/// GET `/session/{session_id}/element/{element_id}/computedrole` - Get computed ARIA role
pub async fn get_computed_role<R: Runtime + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path((session_id, element_id)): Path<(String, String)>,
) -> WebDriverResult {
    let sessions = state.sessions.read().await;
    let session = sessions.get(&session_id)?;

    let element = session
        .elements
        .get(&element_id)
        .ok_or_else(WebDriverErrorResponse::no_such_element)?;

    let js_var = element.js_ref.clone();
    let current_window = session.current_window.clone();
    let timeouts = session.timeouts.clone();
    let frame_context = session.frame_context.clone();
    drop(sessions);

    let executor = state.get_executor_for_window(&current_window, timeouts, frame_context)?;
    let role = executor.get_element_computed_role(&js_var).await?;
    Ok(WebDriverResponse::success(role))
}

/// GET `/session/{session_id}/element/{element_id}/computedlabel` - Get computed accessible name
pub async fn get_computed_label<R: Runtime + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path((session_id, element_id)): Path<(String, String)>,
) -> WebDriverResult {
    let sessions = state.sessions.read().await;
    let session = sessions.get(&session_id)?;

    let element = session
        .elements
        .get(&element_id)
        .ok_or_else(WebDriverErrorResponse::no_such_element)?;

    let js_var = element.js_ref.clone();
    let current_window = session.current_window.clone();
    let timeouts = session.timeouts.clone();
    let frame_context = session.frame_context.clone();
    drop(sessions);

    let executor = state.get_executor_for_window(&current_window, timeouts, frame_context)?;
    let label = executor.get_element_computed_label(&js_var).await?;
    Ok(WebDriverResponse::success(label))
}

/// GET `/session/{session_id}/element/{element_id}/screenshot` - Take element screenshot
pub async fn take_screenshot<R: Runtime + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path((session_id, element_id)): Path<(String, String)>,
) -> WebDriverResult {
    let sessions = state.sessions.read().await;
    let session = sessions.get(&session_id)?;

    let element = session
        .elements
        .get(&element_id)
        .ok_or_else(WebDriverErrorResponse::no_such_element)?;

    let js_var = element.js_ref.clone();
    let current_window = session.current_window.clone();
    let timeouts = session.timeouts.clone();
    let frame_context = session.frame_context.clone();
    drop(sessions);

    let executor = state.get_executor_for_window(&current_window, timeouts, frame_context)?;
    let screenshot = executor.take_element_screenshot(&js_var).await?;
    Ok(WebDriverResponse::success(screenshot))
}
