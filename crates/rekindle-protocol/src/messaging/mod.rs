pub mod envelope;
pub mod receiver;
pub mod sender;

pub use envelope::{MessageEnvelope, MessagePayload};
pub use receiver::process_incoming;
