//! The keys we hold from the other participants of a channel session, and
//! the forward derivation of a later ratchet step from one of them
//! (RFC 9605 §5.1).

use std::collections::{HashMap, HashSet, VecDeque};

use zeroize::Zeroizing;

use super::{generation, step, MissingKey, KEYS_PER_SENDER, MAX_RATCHET_AHEAD};
use crate::sframe;

/// A secret that may have sealed a frame: held as sent, or derived forward
/// from a held one. Installed only once a frame opens under it
/// ([`super::ChannelSenderKeys::confirm`]).
pub struct Candidate {
    pub index: u64,
    pub secret: Zeroizing<[u8; 32]>,
}

/// The secret at `target`, derived forward from `(held, secret)`: the same
/// generation, `target` 1 to [`MAX_RATCHET_AHEAD`] steps later.
fn derive(held: u64, secret: &Zeroizing<[u8; 32]>, target: u64) -> Option<Zeroizing<[u8; 32]>> {
    if generation(held) != generation(target) || step(target) <= step(held) {
        return None;
    }
    let ahead = step(target) - step(held);
    if ahead > MAX_RATCHET_AHEAD {
        return None;
    }
    let mut next = secret.clone();
    for _ in 0..ahead {
        next = sframe::ratchet(&next);
    }
    Some(next)
}

/// Each sender's last [`KEYS_PER_SENDER`] keys, oldest first.
#[derive(Default)]
pub(super) struct Received {
    by_sender: HashMap<String, VecDeque<(u64, Zeroizing<[u8; 32]>)>>,
}

impl Received {
    /// The same index from a sender replaces the old secret (a rejoin's
    /// new session); the oldest is forgotten past [`KEYS_PER_SENDER`].
    pub(super) fn install(&mut self, sender: &str, index: u64, secret: Zeroizing<[u8; 32]>) {
        let keys = self.by_sender.entry(sender.to_string()).or_default();
        keys.retain(|(i, _)| *i != index);
        keys.push_back((index, secret));
        while keys.len() > KEYS_PER_SENDER {
            keys.pop_front();
        }
    }

    pub(super) fn forget(&mut self, sender: &str) {
        self.by_sender.remove(sender);
    }

    /// The secret at exactly `index`, held or derived.
    pub(super) fn at(&self, sender: &str, index: u64) -> Option<Zeroizing<[u8; 32]>> {
        let keys = self.by_sender.get(sender)?;
        if let Some((_, secret)) = keys.iter().rev().find(|(i, _)| *i == index) {
            return Some(secret.clone());
        }
        keys.iter()
            .rev()
            .find_map(|(held, secret)| derive(*held, secret, index))
    }

    /// The secrets a frame whose KID carries `step_low` may be under: per
    /// held generation, newest first, that generation's step `step_low`.
    pub(super) fn candidates(
        &self,
        sender: &str,
        step_low: u8,
    ) -> Result<Vec<Candidate>, MissingKey> {
        let keys = self.by_sender.get(sender);
        let mut held: Vec<&(u64, Zeroizing<[u8; 32]>)> =
            keys.map(|k| k.iter().collect()).unwrap_or_default();
        held.sort_by(|a, b| b.0.cmp(&a.0));
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for (index, _) in &held {
            let target = (generation(*index) << 8) | u64::from(step_low);
            if seen.insert(target) {
                if let Some(secret) = self.at(sender, target) {
                    out.push(Candidate {
                        index: target,
                        secret,
                    });
                }
            }
        }
        if out.is_empty() {
            let newest = held.first().map_or(0, |(i, _)| generation(*i));
            return Err(MissingKey {
                sender: sender.to_string(),
                index: (newest << 8) | u64::from(step_low),
            });
        }
        Ok(out)
    }
}
