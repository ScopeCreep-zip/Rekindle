//! The stored form of a [`SessionRecord`] (plan E2.3's
//! `ratchet_sessions.record`): `u8 format ‖ peer(32) ‖ opt(active) ‖
//! u8 count ‖ count × inactive`, each state in `dr::codec`'s form. The
//! bytes hold secrets, so they come back wiped on drop.

use zeroize::Zeroizing;

use super::{SessionRecord, ARCHIVED_STATES_MAX_LENGTH};
use crate::error::CryptoError;
use crate::signal::dr::{corrupt, Reader, SessionState};

/// The stored form's version.
const FORMAT: u8 = 1;

impl SessionRecord {
    /// The record's stored form.
    #[must_use]
    pub fn to_bytes(&self) -> Zeroizing<Vec<u8>> {
        let mut out = Zeroizing::new(vec![FORMAT]);
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
