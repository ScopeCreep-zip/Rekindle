//! Fleet-density registries — co-resident identity management and
//! remote peer resolution cache.

pub mod resident;
pub mod resolver;

pub use resident::{ResidentIdentity, ResidentSet};
pub use resolver::{PeerResolver, ResolvedPeer};
