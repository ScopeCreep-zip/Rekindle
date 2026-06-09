pub mod lane_channels;
pub mod encode;
pub mod decode;
#[allow(unsafe_code)]
pub mod epoch_signal;
pub mod read_task;
pub mod control_loop;
#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
pub mod uring_write_task;
#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
pub mod uring_read_task;
// Tokio async write task — macOS/Windows only (future).
// On Linux, the uring write task replaces it entirely.
// Compiled conditionally to avoid type conflicts with crossbeam LaneReceivers.
#[cfg(not(target_os = "linux"))]
pub mod write_task;
