//! Daemon state that outlives one request: the audit log and the unlocked
//! identity's signing key. The identity's own stores (database, vault,
//! registry) are `host::identity`'s; its session is projected from them by
//! `host::session`.

pub mod audit;
pub mod signing_key;
