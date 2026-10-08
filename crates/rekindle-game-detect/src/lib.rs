pub mod database;
pub mod error;
pub mod launcher;
pub mod platform;
pub mod rich_presence;
pub mod runtime;
pub mod scanner;

pub use database::GameDatabase;
pub use error::GameDetectError;
pub use runtime::{run as run_runtime, GameDetectorPublisher, DEFAULT_POLL_INTERVAL};
pub use scanner::{DetectedGame, GameDetector};
