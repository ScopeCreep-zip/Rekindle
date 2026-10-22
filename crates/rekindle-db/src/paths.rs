//! The data root, resolved by `rekindle_utils::paths` (where the thin
//! frontends, which must not link this crate, reach it too).

pub use rekindle_utils::paths::{DataRoot, PathsError, IDENTIFIER};
