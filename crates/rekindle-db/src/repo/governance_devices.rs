//! `governance_devices` (ADR 0022 rules 2, 6, 7; plan E3.2 S7d-keys.3):
//! each member's device-key set as the last merge judged it, one row per
//! endorsed `(device_key, incarnation)`. The op set stays the source of
//! truth (the endorsements travel in ops); this is the set a merge derives
//! from it, written after each rebuild like `governance_checkpoints`, so the
//! unlock reads our own keys' standing before the network answers.

use rusqlite::{params, Connection, Row};

use crate::repo::governance_ops::{hash32, int};
use crate::row::{get, get_enum, get_u64};

/// A device key's standing in the last merge (ADR 0022).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceState {
    /// The endorsement's extended set is not fully held yet (rule 2).
    Pending,
    /// Endorsed, within the cap and admitted by rule 7.
    Effective,
    /// Above the cap of 6 live keys (rule 6); never re-carried into effect.
    Inert,
    /// A `DeviceRetired` or fork evidence names it; permanent (rule 2).
    Retired,
}

impl DeviceState {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Effective => "effective",
            Self::Inert => "inert",
            Self::Retired => "retired",
        }
    }
}

impl std::str::FromStr for DeviceState {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(Self::Pending),
            "effective" => Ok(Self::Effective),
            "inert" => Ok(Self::Inert),
            "retired" => Ok(Self::Retired),
            other => Err(format!("unknown governance device state {other:?}")),
        }
    }
}

/// One element of a member's device-key set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRow {
    /// The member's pseudonym, hex.
    pub member: String,
    /// The device key, hex.
    pub device_key: String,
    pub incarnation: u64,
    /// The device's member record `M(p,d)` (rule 3).
    pub device_record: String,
    pub state: DeviceState,
    /// The fork-evidence hash the endorsement answers (rule 10), if any.
    pub fork_evidence: Option<[u8; 32]>,
}

fn device_row(row: &Row<'_>) -> rusqlite::Result<DeviceRow> {
    let fork: Option<Vec<u8>> = get(row, "fork_evidence")?;
    Ok(DeviceRow {
        member: get(row, "member")?,
        device_key: get(row, "device_key")?,
        incarnation: get_u64(row, 2)?,
        device_record: get(row, "device_record")?,
        state: get_enum(row, "state")?,
        fork_evidence: match fork {
            None => None,
            Some(_) => Some(hash32(row, "fork_evidence")?),
        },
    })
}

/// Replace `community_id`'s device set with `rows`, the set the last merge
/// derived (the caller holds the transaction).
///
/// # Errors
/// The write failed.
pub fn replace(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    rows: &[DeviceRow],
) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM governance_devices WHERE owner_key = ?1 AND community_id = ?2",
        params![owner_key, community_id],
    )?;
    let mut stmt = conn.prepare(
        "INSERT INTO governance_devices (owner_key, community_id, member, device_key, \
         incarnation, device_record, state, fork_evidence) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )?;
    for row in rows {
        stmt.execute(params![
            owner_key,
            community_id,
            row.member,
            row.device_key,
            int(row.incarnation)?,
            row.device_record,
            row.state.as_str(),
            row.fork_evidence.as_ref().map(<[u8; 32]>::as_slice),
        ])?;
    }
    Ok(())
}

/// `community_id`'s device set, `member` only when given, by member then
/// `(incarnation, device_key)` (rule 6's order).
///
/// # Errors
/// The query failed or a row is corrupt.
pub fn list(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    member: Option<&str>,
) -> rusqlite::Result<Vec<DeviceRow>> {
    let mut stmt = conn.prepare(
        "SELECT member, device_key, incarnation, device_record, state, fork_evidence \
         FROM governance_devices \
         WHERE owner_key = ?1 AND community_id = ?2 AND (?3 IS NULL OR member = ?3) \
         ORDER BY member, incarnation, device_key",
    )?;
    let rows = stmt.query_map(params![owner_key, community_id, member], device_row)?;
    rows.collect()
}

#[cfg(test)]
#[path = "governance_devices_tests.rs"]
mod tests;
