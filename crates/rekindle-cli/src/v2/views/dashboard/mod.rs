//! Dashboard view — the default view on bare `rekindle` invocation.
//! Four panels: Identity, Node Status, Communities, Friends summary.

pub mod events;
pub mod input;
pub mod render;
pub mod state;

pub use state::DashboardView;
