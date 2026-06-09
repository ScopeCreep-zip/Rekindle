//! Wire layout stability tests.
//!
//! These tests lock the byte-level wire format. They fail if anyone
//! changes a field offset, struct size, or constant value without
//! updating the test — forcing the conversation about wire compat.

mod envelope_size;
mod header_size;
mod cache_line;
mod envelope_offsets;
mod header_offsets;
mod lane_values;
mod frame_class_values;
mod frame_kind_uniqueness;
mod alignment;
mod wire_version;
mod reserved_rejection;
