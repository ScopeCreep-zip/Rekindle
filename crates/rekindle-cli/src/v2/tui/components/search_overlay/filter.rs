//! Search filtering — substring match with nucleo-ready architecture.

use super::super::super::action::Action;

/// A searchable item in the results list.
#[derive(Debug, Clone)]
pub struct SearchItem {
    pub label: String,
    pub detail: String,
    pub action: Action,
}

/// Refresh filtered indices based on query. Empty query shows all.
pub fn filter_items(items: &[SearchItem], query: &str) -> Vec<usize> {
    if query.is_empty() {
        return (0..items.len()).collect();
    }
    let query_lower = query.to_lowercase();
    items.iter().enumerate()
        .filter(|(_, item)| {
            item.label.to_lowercase().contains(&query_lower)
                || item.detail.to_lowercase().contains(&query_lower)
        })
        .map(|(i, _)| i)
        .collect()
}
