//! ChatService delegation methods — forward to domain services.
//!
//! Every public method on ChatService that delegates to a domain service
//! (FriendshipService, MessagingService, CommunityService, IdentityService,
//! PresenceService) lives here. Each submodule groups delegations by domain.
//!
//! The pattern is uniform: one-line forwarding, no logic, no transformation.
//! If a delegation requires parameter mapping or error wrapping, that logic
//! belongs in the domain service method — not here.

mod friendship;
mod messaging;
mod community;
mod governance;
mod identity;
mod presence;
mod social;
mod system;
mod keys;
mod voice;
