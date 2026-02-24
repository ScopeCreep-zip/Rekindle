pub mod database;
pub mod error;
pub mod launcher;
pub mod platform;
pub mod rich_presence;
pub mod scanner;

pub use database::GameDatabase;
pub use error::GameDetectError;
pub use scanner::{DetectedGame, GameDetector};
