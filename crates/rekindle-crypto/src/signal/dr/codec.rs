//! The stored form of a [`SessionState`]: fixed-width fields, big-endian
//! integers, counts bounded by the ratchet's own limits, so a corrupt or
//! hostile blob is an error rather than an allocation.
//!
//! `ad(66) ‖ base_key(32) ‖ sk_scka(32) ‖ root(32) ‖ dh_self(32) ‖
//! opt(remote(32)) ‖ opt(sender_key(32) ‖ u32 n) ‖ u32 pn ‖ u8 count ‖
//! count × (their(32) ‖ key(32) ‖ u32 n ‖ u16 skipped ‖ skipped × (u32 n ‖
//! key(32)))`, where `opt(x)` is a `0`/`1` byte then `x` when `1`.

use super::state::{Chain, ReceiverChain, SessionState, SkippedKey};
use super::{MAX_MESSAGE_KEYS, MAX_RECEIVER_CHAINS};
use crate::error::CryptoError;
use crate::signal::pqxdh::agreement::AD_LEN;

impl SessionState {
    /// Append the stored form to `out`.
    pub(crate) fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.ad);
        out.extend_from_slice(&self.base_key);
        out.extend_from_slice(&self.sk_scka);
        out.extend_from_slice(&self.root_key);
        out.extend_from_slice(&self.dh_self);
        match &self.remote {
            Some(remote) => {
                out.push(1);
                out.extend_from_slice(remote);
            }
            None => out.push(0),
        }
        match &self.sender {
            Some(chain) => {
                out.push(1);
                write_chain(chain, out);
            }
            None => out.push(0),
        }
        out.extend_from_slice(&self.pn.to_be_bytes());
        out.push(u8::try_from(self.receivers.len()).expect("receiving chains are bounded"));
        for receiver in &self.receivers {
            out.extend_from_slice(&receiver.their);
            write_chain(&receiver.chain, out);
            let skipped = u16::try_from(receiver.skipped.len()).expect("skipped keys are bounded");
            out.extend_from_slice(&skipped.to_be_bytes());
            for key in &receiver.skipped {
                out.extend_from_slice(&key.n.to_be_bytes());
                out.extend_from_slice(&key.key);
            }
        }
    }

    /// Read one stored state from `reader`.
    pub(crate) fn read(reader: &mut Reader<'_>) -> Result<Self, CryptoError> {
        let ad = reader.array::<AD_LEN>()?;
        let base_key = reader.array()?;
        let sk_scka = reader.array()?;
        let root_key = reader.array()?;
        let dh_self = reader.array()?;
        let remote = reader.flag()?.then(|| reader.array()).transpose()?;
        let sender = reader.flag()?.then(|| read_chain(reader)).transpose()?;
        let pn = reader.u32()?;
        let count = usize::from(reader.u8()?);
        if count > MAX_RECEIVER_CHAINS {
            return Err(corrupt("too many receiving chains"));
        }
        let mut receivers = Vec::with_capacity(count);
        for _ in 0..count {
            let their = reader.array()?;
            let chain = read_chain(reader)?;
            let skipped = usize::from(reader.u16()?);
            if skipped > MAX_MESSAGE_KEYS {
                return Err(corrupt("too many skipped keys"));
            }
            let skipped = (0..skipped)
                .map(|_| {
                    Ok(SkippedKey {
                        n: reader.u32()?,
                        key: reader.array()?,
                    })
                })
                .collect::<Result<_, CryptoError>>()?;
            receivers.push(ReceiverChain {
                their,
                chain,
                skipped,
            });
        }
        Ok(Self {
            ad,
            base_key,
            sk_scka,
            root_key,
            dh_self,
            remote,
            sender,
            pn,
            receivers,
        })
    }
}

fn write_chain(chain: &Chain, out: &mut Vec<u8>) {
    out.extend_from_slice(&chain.key);
    out.extend_from_slice(&chain.n.to_be_bytes());
}

fn read_chain(reader: &mut Reader<'_>) -> Result<Chain, CryptoError> {
    Ok(Chain {
        key: reader.array()?,
        n: reader.u32()?,
    })
}

/// A stored session that does not parse.
pub(crate) fn corrupt(what: &str) -> CryptoError {
    CryptoError::SessionError(format!("corrupt session record: {what}"))
}

/// A cursor over a stored record.
pub(crate) struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    /// Whether every byte has been read.
    pub(crate) fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub(crate) fn array<const N: usize>(&mut self) -> Result<[u8; N], CryptoError> {
        let (head, rest) = self
            .bytes
            .split_first_chunk::<N>()
            .ok_or_else(|| corrupt("truncated"))?;
        self.bytes = rest;
        Ok(*head)
    }

    pub(crate) fn u8(&mut self) -> Result<u8, CryptoError> {
        Ok(self.array::<1>()?[0])
    }

    fn u16(&mut self) -> Result<u16, CryptoError> {
        Ok(u16::from_be_bytes(self.array()?))
    }

    fn u32(&mut self) -> Result<u32, CryptoError> {
        Ok(u32::from_be_bytes(self.array()?))
    }

    /// A `0`/`1` presence byte.
    pub(crate) fn flag(&mut self) -> Result<bool, CryptoError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(corrupt("bad presence byte")),
        }
    }
}
