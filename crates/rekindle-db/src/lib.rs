//! `rekindle-db`: Rekindle's storage foundation, shared by every host
//! (plan C5).
//!
//! - [`paths::DataRoot`]: the directories every host resolves its files in.
//! - [`lock::NodeLock`]: one Rekindle node per data root.
//! - [`open`] / [`SCHEMA`] / [`SCHEMA_VERSION`]: the schema and opening it.
//! - [`Db`]: the handle every query goes through.
//! - [`DbHandle`]: the logged-in identity's database, or none.
//! - [`SqliteFriendStore`]: the receive path's friend authority.
//! - [`repo`]: the queries on the tables more than one host uses.
//! - [`ratchet`] / [`prekeys`]: the 1:1 ratchet and our prekeys, each step
//!   one transaction on the `Db` thread (plan E2.3).

mod db;
mod friend_store;
mod handle;
pub mod lock;
mod open;
pub mod paths;
pub mod prekeys;
pub mod ratchet;
pub mod registry;
pub mod repo;
pub mod reset;
pub mod row;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use db::Db;
pub use friend_store::SqliteFriendStore;
pub use handle::{DbHandle, DbStillHeld, NotLoggedIn};
pub use open::{open, DbError, SCHEMA, SCHEMA_VERSION};
/// The database thread's call watch (`rekindle-asql`): a simulator
/// installs one so its paused clock sees database work (plan E2.B S8).
pub use rekindle_asql::{watch_calls, CallToken, CallWatch};
