//! Layer 0 — Origin. The `OriginSeed`, derivation tags, and the
//! `originate()` / `restore()` identity constructors.

pub mod tags;
pub mod seed;
pub mod originate;

pub use seed::OriginSeed;
pub use originate::{
    originate, originate_from_seed, restore,
    OriginatedIdentity, RestoredIdentity,
    IdentityRoot, PeerRef, RotationEpoch,
    DhSeed, RevocationCertificate,
};
