//! 1:1 DM video (0x0220, until E4.3.8): the outbound fragmenting send and
//! the inbound reassembled frame's delivery. Group DMs are `rekindle-dm`'s
//! (plan E2.6), driven through the Host.

pub mod video;
