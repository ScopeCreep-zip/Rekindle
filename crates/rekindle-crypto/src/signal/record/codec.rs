//! The stored form of a [`SessionRecord`] (plan E2.3's
//! `ratchet_sessions.record`): `u8 format ‖ peer(32) ‖ opt(active) ‖
//! u8 count ‖ count × inactive`, each state in `dr::codec`'s form. The
//! bytes hold secrets, so they come back wiped on drop.

use zeroize::Zeroizing;

use super::{SessionRecord, ARCHIVED_STATES_MAX_LENGTH};
use crate::error::CryptoError;
use crate::signal::dr::{corrupt, Reader, SessionState};

/// The stored form's version: 2 stores each skipped key's time (audit
/// 2026-10-20 C-7).
const FORMAT: u8 = 2;

impl SessionRecord {
    /// The record's stored form.
    #[must_use]
    ///
    /// The buffer is sized once from every state's stored length, so it
    /// never reallocates (a reallocation would free an unwiped copy).
    pub fn to_bytes(&self) -> Zeroizing<Vec<u8>> {
        let len = 1
            + 32
            + 1
            + self.active.as_ref().map_or(0, SessionState::stored_len)
            + 1
            + self
                .inactive
                .iter()
                .map(SessionState::stored_len)
                .sum::<usize>();
        let mut out = Zeroizing::new(Vec::with_capacity(len));
        out.push(FORMAT);
        out.extend_from_slice(&self.peer);
        match &self.active {
            Some(state) => {
                out.push(1);
                state.write(&mut out);
            }
            None => out.push(0),
        }
        out.push(u8::try_from(self.inactive.len()).expect("inactive sessions are bounded"));
        for state in &self.inactive {
            state.write(&mut out);
        }
        debug_assert_eq!(out.len(), len, "the stored length is exact");
        out
    }

    /// Parse [`Self::to_bytes`]' form; anything else, including trailing
    /// bytes, is an error.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CryptoError> {
        let mut reader = Reader::new(bytes);
        if reader.u8()? != FORMAT {
            return Err(corrupt("unknown format"));
        }
        let peer = reader.array()?;
        let active = reader
            .flag()?
            .then(|| SessionState::read(&mut reader))
            .transpose()?;
        let count = usize::from(reader.u8()?);
        if count > ARCHIVED_STATES_MAX_LENGTH {
            return Err(corrupt("too many inactive sessions"));
        }
        let inactive = (0..count)
            .map(|_| SessionState::read(&mut reader))
            .collect::<Result<_, _>>()?;
        if !reader.is_empty() {
            return Err(corrupt("trailing bytes"));
        }
        Ok(Self {
            peer,
            active,
            inactive,
        })
    }
}
