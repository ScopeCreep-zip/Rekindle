//! The inbound lanes (plan E4.1 S3), moved here from the desktop's
//! `services/veilid/` so both hosts share one copy: inbound `app_call`s
//! are served concurrently on [`AppCallLane`] (Veilid's 5 s answer
//! deadline), the slow control updates drain one FIFO
//! ([`ControlIngressQueue`]) and gossip another ([`GossipIngressQueue`]),
//! so a media datagram, handled synchronously on the dispatch loop
//! (`InboundHandler::on_media`), never queues behind any of them.

pub mod app_call;
pub mod control;
pub mod gossip;

pub use app_call::{AppCallLane, APP_CALL_CONCURRENCY};
pub use control::{ControlIngressQueue, CONTROL_INGRESS_QUEUE_MAX};
pub use gossip::{GossipIngressQueue, IngressItem, INGRESS_QUEUE_MAX};
