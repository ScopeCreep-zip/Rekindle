use std::collections::{HashMap, HashSet};

use serde::Serialize;
use uuid::Uuid;

use super::element::ElementStore;
use crate::platform::{FrameId, PointerInputState};
use crate::server::response::WebDriverErrorResponse;

/// Tracks currently pressed keys and pointer buttons for action state
#[derive(Debug, Default, Clone)]
pub struct ActionState {
    /// Currently pressed keyboard keys (`WebDriver` key codes)
    pub pressed_keys: HashSet<String>,
    /// Currently pressed pointer buttons by source ID
    pub pressed_buttons: HashMap<String, HashSet<u32>>,
    /// Pointer sources whose `pointerdown` was canceled; their compatibility mouse
    /// events stay suppressed until they release every button.
    pub canceled_pointer_downs: HashSet<String>,
    /// Where each source's primary button went down; releasing it there clicks.
    pub click_anchors: HashMap<String, (i32, i32)>,
    /// Last pointer position in viewport coordinates; persists across `performActions` calls.
    pub pointer_position: (i32, i32),
}

impl ActionState {
    /// `None` when the button is already held, which W3C makes a no-op.
    pub fn press_pointer_button(
        &mut self,
        source_id: &str,
        button: u32,
    ) -> Option<PointerInputState> {
        let held = self
            .pressed_buttons
            .entry(source_id.to_string())
            .or_default();
        if held.contains(&button) {
            return None;
        }
        let chorded = !held.is_empty();
        held.insert(button);
        if button == 0 {
            self.click_anchors
                .insert(source_id.to_string(), self.pointer_position);
        }
        Some(PointerInputState {
            chorded,
            ..self.pointer_input_state(source_id)
        })
    }

    /// `None` when the button isn't held, which W3C makes a no-op.
    pub fn release_pointer_button(
        &mut self,
        source_id: &str,
        button: u32,
    ) -> Option<PointerInputState> {
        let held = self
            .pressed_buttons
            .get_mut(source_id)
            .filter(|held| held.contains(&button))?;
        held.remove(&button);
        let chorded = !held.is_empty();
        let input_state = PointerInputState {
            buttons: dom_buttons_mask(held.iter()),
            suppress_mouse_events: self.canceled_pointer_downs.contains(source_id),
            chorded,
            clicks: button == 0
                && self.click_anchors.get(source_id) == Some(&self.pointer_position),
        };
        if button == 0 {
            self.click_anchors.remove(source_id);
        }
        if !chorded {
            self.canceled_pointer_downs.remove(source_id);
        }
        Some(input_state)
    }

    pub fn cancel_pointer_down(&mut self, source_id: &str) {
        self.canceled_pointer_downs.insert(source_id.to_string());
    }

    /// Input state for an event that doesn't change the held buttons.
    pub fn pointer_input_state(&self, source_id: &str) -> PointerInputState {
        let buttons = self
            .pressed_buttons
            .get(source_id)
            .map_or(0, |pressed| dom_buttons_mask(pressed.iter()));
        PointerInputState {
            buttons,
            suppress_mouse_events: self.canceled_pointer_downs.contains(source_id),
            chorded: false,
            clicks: false,
        }
    }

    pub fn forget_pointer_input(&mut self) {
        self.pressed_buttons.clear();
        self.canceled_pointer_downs.clear();
        self.click_anchors.clear();
    }

    /// Sorted by source, then button, so releasing them is deterministic.
    pub fn held_pointer_buttons(&self) -> Vec<(String, u32)> {
        let mut held: Vec<(String, u32)> = self
            .pressed_buttons
            .iter()
            .flat_map(|(source_id, buttons)| {
                buttons
                    .iter()
                    .map(move |&button| (source_id.clone(), button))
            })
            .collect();
        held.sort();
        held
    }
}

fn dom_buttons_mask<'a>(buttons: impl Iterator<Item = &'a u32>) -> u32 {
    buttons.fold(0, |mask, &button| mask | dom_buttons_bit(button))
}

/// The DOM `buttons` bitmask swaps the middle and secondary buttons relative to `button`.
fn dom_buttons_bit(button: u32) -> u32 {
    match button {
        1 => 4,
        2 => 2,
        other => 1u32.checked_shl(other).unwrap_or(0),
    }
}

/// Session timeouts configuration
#[derive(Debug, Clone, Serialize)]
pub struct Timeouts {
    /// Implicit wait timeout in milliseconds
    pub implicit: u64,
    /// Page load timeout in milliseconds
    pub page_load: u64,
    /// Script execution timeout in milliseconds
    pub script: u64,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            implicit: 0,
            page_load: 300_000,
            script: 30_000,
        }
    }
}

/// Represents a `WebDriver` session
#[derive(Debug)]
pub struct Session {
    /// Unique session identifier
    pub id: String,
    /// Session timeouts
    pub timeouts: Timeouts,
    /// Element reference storage
    pub elements: ElementStore,
    /// Current window handle
    pub current_window: String,
    /// Current frame context (stack of frame selectors)
    pub frame_context: Vec<FrameId>,
    /// Increments whenever the selected top-level browsing context changes
    pub browsing_context_generation: u64,
    /// Action state tracking for pressed keys/buttons
    pub action_state: ActionState,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SwitchWindowError {
    BrowsingContextGenerationOverflow,
}

impl Session {
    pub fn new(initial_window: String) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            timeouts: Timeouts::default(),
            elements: ElementStore::new(),
            current_window: initial_window,
            frame_context: Vec::new(),
            browsing_context_generation: 0,
            action_state: ActionState::default(),
        }
    }

    pub fn switch_to_window(&mut self, window: String) -> Result<(), SwitchWindowError> {
        let browsing_context_generation = self
            .browsing_context_generation
            .checked_add(1)
            .ok_or(SwitchWindowError::BrowsingContextGenerationOverflow)?;

        self.current_window = window;
        self.frame_context.clear();
        self.browsing_context_generation = browsing_context_generation;

        Ok(())
    }
}

/// Manages `WebDriver` sessions
#[derive(Debug, Default)]
pub struct SessionManager {
    sessions: HashMap<String, Session>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
        }
    }

    /// Create a new session
    pub fn create(&mut self, initial_window: String) -> &Session {
        let session = Session::new(initial_window);
        let id = session.id.clone();
        self.sessions.insert(id.clone(), session);
        self.sessions.get(&id).expect("session was just inserted")
    }

    /// Get a session by ID
    pub fn get(&self, id: &str) -> Result<&Session, WebDriverErrorResponse> {
        self.sessions
            .get(id)
            .ok_or_else(|| WebDriverErrorResponse::invalid_session_id(id))
    }

    /// Get a mutable session by ID
    pub fn get_mut(&mut self, id: &str) -> Result<&mut Session, WebDriverErrorResponse> {
        self.sessions
            .get_mut(id)
            .ok_or_else(|| WebDriverErrorResponse::invalid_session_id(id))
    }

    /// Delete a session
    pub fn delete(&mut self, id: &str) -> bool {
        self.sessions.remove(id).is_some()
    }
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
