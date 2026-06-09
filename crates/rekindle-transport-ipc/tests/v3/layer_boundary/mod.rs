//! Layer boundary violation tests.
//!
//! Source-level scans that verify the four-layer architecture:
//! handlers must not import wire types; codec must not import
//! handlers; dispatch must not import wire types.
//!
//! These tests scan .rs files for forbidden `use` patterns.
//! They run before any implementation exists (vacuously passing
//! on empty directories) and catch violations as code is added.

mod handler_isolation;
mod codec_isolation;
mod dispatch_isolation;
