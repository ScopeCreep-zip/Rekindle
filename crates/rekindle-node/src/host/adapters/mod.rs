//! The Host's adapters: the shared crates' traits implemented over the
//! unlocked identity's vault and database (plan D1), and the friend and 1:1
//! operations' Host (plan E2.1).

pub mod friends;
pub mod governance;
pub mod group_dm;
pub mod mek;
pub mod outbox;
