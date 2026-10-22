//! The community requests' `Debug` (plan E3.1), split by domain: each
//! domain file formats its own sub-enum, and a message body, free text or
//! picture shows its length only.

mod activities;
mod audit;
mod channels;
mod expressions;
mod lifecycle;
mod members;
mod messages;
mod pins;
mod rich;
mod settings;
mod unread;

use super::super::CommunityRequest;

impl std::fmt::Debug for CommunityRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Lifecycle(request) => request.fmt(f),
            Self::Settings(request) => request.fmt(f),
            Self::Channels(request) => request.fmt(f),
            Self::Messages(request) => request.fmt(f),
            Self::Unread(request) => request.fmt(f),
            Self::Audit(request) => request.fmt(f),
            Self::Activities(request) => request.fmt(f),
            Self::Members(request) => request.fmt(f),
            Self::Pins(request) => request.fmt(f),
            Self::Rich(request) => request.fmt(f),
            Self::Expressions(request) => request.fmt(f),
        }
    }
}
