//! Phase 14.k — voice presence handlers (join / leave / roster).
//!
//! Implements §10.2 mesh↔MCU auto-switching at the 5+ participant
//! threshold and §10.7 always-MCU stage-channel transport reconcile.

mod ack;
mod join;
mod leave;
mod mcu;
mod roster;

pub(in crate::signaling) use ack::{handle_voice_join_ack, handle_voice_join_confirmed};
pub(in crate::signaling) use join::{handle_voice_join, send_confirmed_if_first};
pub(in crate::signaling) use leave::handle_voice_leave;
pub(crate) use mcu::maybe_switch_to_mcu;
pub(in crate::signaling) use roster::handle_voice_roster;
