//! The stored form of a [`SessionRecord`] (plan E2.3's
//! `ratchet_sessions.record`): `u8 format ‖ peer(32) ‖ u8 last_act ‖
//! u8 direction_changes ‖ opt(active) ‖ u8 count ‖ count × inactive`, each
//! session `state ‖ opt(pending)` with the state in `dr::codec`'s form and
//! `pending = identity(32) ‖ u32 signed_prekey ‖ opt(u32 one_time) ‖
//! u8 pq_kind ‖ u32 pq_id ‖ u16 len ‖ kem_ciphertext ‖ i64 created_ms`. The
//! bytes hold secrets, so they come back wiped on drop.

use zeroize::Zeroizing;

use super::policy::LastAct;
use super::{PendingInit, Session, SessionRecord, MAX_INACTIVE_SESSIONS};
use crate::error::CryptoError;
use crate::signal::dr::{corrupt, Reader, SessionState};
use crate::signal::pqxdh::agreement::PqPrekeyId;

/// The stored form's version: 3 adds the archive policy's state and each
/// session's pending init (plan E2.5h).
const FORMAT: u8 = 3;

impl PendingInit {
    fn stored_len(&self) -> usize {
        32 + 4
            + 1
            + self.one_time_prekey_id.map_or(0, |_| 4)
            + 1
            + 4
            + 2
            + self.kem_ciphertext.len()
            + 8
    }

    fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.identity_key);
        out.extend_from_slice(&self.signed_prekey_id.to_be_bytes());
        match self.one_time_prekey_id {
            Some(id) => {
                out.push(1);
                out.extend_from_slice(&id.to_be_bytes());
            }
            None => out.push(0),
        }
        let (kind, id) = match self.pq_prekey {
            PqPrekeyId::OneTime(id) => (0, id),
            PqPrekeyId::LastResort(id) => (1, id),
        };
        out.push(kind);
        out.extend_from_slice(&id.to_be_bytes());
        let len = u16::try_from(self.kem_ciphertext.len()).expect("a KEM ciphertext fits u16");
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(&self.kem_ciphertext);
        out.extend_from_slice(&self.created_ms.to_be_bytes());
    }

    fn read(reader: &mut Reader<'_>) -> Result<Self, CryptoError> {
        let identity_key = reader.array()?;
        let signed_prekey_id = reader.u32()?;
        let one_time_prekey_id = reader.flag()?.then(|| reader.u32()).transpose()?;
        let pq_prekey = match (reader.u8()?, reader.u32()?) {
            (0, id) => PqPrekeyId::OneTime(id),
            (1, id) => PqPrekeyId::LastResort(id),
            _ => return Err(corrupt("bad ML-KEM prekey kind")),
        };
        let len = usize::from(reader.u16()?);
        let kem_ciphertext = (0..len)
            .map(|_| reader.u8())
            .collect::<Result<Vec<u8>, _>>()?;
        let created_ms = i64::from_be_bytes(reader.array()?);
        Ok(Self {
            identity_key,
            signed_prekey_id,
            one_time_prekey_id,
            pq_prekey,
            kem_ciphertext,
            created_ms,
        })
    }
}

impl Session {
    fn stored_len(&self) -> usize {
        self.state.stored_len() + 1 + self.pending.as_ref().map_or(0, PendingInit::stored_len)
    }

    fn write(&self, out: &mut Vec<u8>) {
        self.state.write(out);
        match &self.pending {
            Some(pending) => {
                out.push(1);
                pending.write(out);
            }
            None => out.push(0),
        }
    }

    fn read(reader: &mut Reader<'_>) -> Result<Self, CryptoError> {
        let state = SessionState::read(reader)?;
        let pending = reader
            .flag()?
            .then(|| PendingInit::read(reader))
            .transpose()?;
        Ok(Self { state, pending })
    }
}

impl SessionRecord {
    /// The record's stored form.
    ///
    /// The buffer is sized once from every session's stored length, so it
    /// never reallocates (a reallocation would free an unwiped copy).
    #[must_use]
    pub fn to_bytes(&self) -> Zeroizing<Vec<u8>> {
        let len = 1
            + 32
            + 2
            + 1
            + self.active.as_ref().map_or(0, Session::stored_len)
            + 1
            + self.inactive.iter().map(Session::stored_len).sum::<usize>();
        let mut out = Zeroizing::new(Vec::with_capacity(len));
        out.push(FORMAT);
        out.extend_from_slice(&self.peer);
        out.push(self.last_act.to_byte());
        out.push(self.direction_changes);
        match &self.active {
            Some(session) => {
                out.push(1);
                session.write(&mut out);
            }
            None => out.push(0),
        }
        out.push(u8::try_from(self.inactive.len()).expect("inactive sessions are bounded"));
        for session in &self.inactive {
            session.write(&mut out);
        }
        debug_assert_eq!(out.len(), len, "the stored length is exact");
        out
    }

    /// Parse [`Self::to_bytes`]' form; anything else, including trailing
    /// bytes or more than [`MAX_INACTIVE_SESSIONS`] inactive sessions, is
    /// an error.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CryptoError> {
        let mut reader = Reader::new(bytes);
        if reader.u8()? != FORMAT {
            return Err(corrupt("unknown format"));
        }
        let peer = reader.array()?;
        let last_act = LastAct::from_byte(reader.u8()?).ok_or_else(|| corrupt("bad last act"))?;
        let direction_changes = reader.u8()?;
        let active = reader
            .flag()?
            .then(|| Session::read(&mut reader))
            .transpose()?;
        let count = usize::from(reader.u8()?);
        if count > MAX_INACTIVE_SESSIONS {
            return Err(corrupt("too many inactive sessions"));
        }
        let inactive = (0..count)
            .map(|_| Session::read(&mut reader))
            .collect::<Result<_, _>>()?;
        if !reader.is_empty() {
            return Err(corrupt("trailing bytes"));
        }
        Ok(Self {
            peer,
            active,
            inactive,
            last_act,
            direction_changes,
        })
    }
}
