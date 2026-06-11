//! Sealed vault label type and exhaustive builder set.
//!
//! The `Label` type has a private field and no public constructor.
//! Only the builder functions in `builders` can construct it.
//! `rekindle-storage`'s vault API accepts `&Label` exclusively —
//! inline format strings cannot compile into vault keys.

pub mod label;
pub mod builders;

pub use label::Label;
pub use builders::*;
