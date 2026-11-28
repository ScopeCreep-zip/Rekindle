pub mod database;
pub mod error;
pub mod installed;
pub mod launcher;
pub mod platform;
pub mod rich_presence;
pub mod runtime;
pub mod scanner;
pub mod server_query;
#[cfg(test)]
mod test_support;

pub use database::GameDatabase;
pub use error::GameDetectError;
pub use runtime::{run as run_runtime, GameDetectorPublisher};
pub use scanner::{DetectedGame, GameDetector};
