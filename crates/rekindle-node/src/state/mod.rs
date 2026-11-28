//! Daemon state that outlives one request: the unlocked identity's signing
//! key. Its audit chain is `host::audit`'s. The identity's own stores (database, vault,
//! registry) are `host::identity`'s; its session is projected from them by
//! `host::session`.

pub mod signing_key;
