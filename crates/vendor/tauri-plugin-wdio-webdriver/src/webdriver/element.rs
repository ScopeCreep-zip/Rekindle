use std::collections::HashMap;

use uuid::Uuid;

/// Represents a `WebDriver` element reference
#[derive(Debug, Clone)]
pub struct ElementRef {
    /// `WebDriver` element ID (returned to client)
    pub id: String,
    /// JavaScript variable name holding the element reference
    pub js_ref: String,
}

/// Storage for element references within a session
#[derive(Debug, Default)]
pub struct ElementStore {
    elements: HashMap<String, ElementRef>,
}

impl ElementStore {
    pub fn new() -> Self {
        Self {
            elements: HashMap::new(),
        }
    }

    /// Store a new element and return its reference
    pub fn store(&mut self) -> ElementRef {
        let id = Uuid::new_v4().to_string();
        // Remove hyphens from UUID for valid JS variable name
        let id_no_hyphens = id.replace('-', "");
        let js_ref = format!("__wd_el_{id_no_hyphens}");

        let elem_ref = ElementRef {
            id: id.clone(),
            js_ref,
        };

        self.elements.insert(id, elem_ref.clone());
        elem_ref
    }

    /// Get element by `WebDriver` ID
    pub fn get(&self, id: &str) -> Option<&ElementRef> {
        self.elements.get(id)
    }
}

#[cfg(test)]
#[path = "element_tests.rs"]
mod tests;
