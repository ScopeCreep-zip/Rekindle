//! The vault header `{vault}.hdr` (plan D1): the Argon2id parameters and
//! salt a passphrase is stretched with, readable before the vault itself
//! (they are needed to derive its key). 64 bytes, plaintext: neither the
//! parameters nor the salt is secret, only unique per vault.
//!
//! Layout (little-endian): `b"RKVH"`, version `1`, 3 zero bytes,
//! `m_cost` KiB, `t_cost`, `p_cost`, output length (4 × u32), the 32-byte
//! salt, 8 zero bytes.

use std::path::{Path, PathBuf};

use rand::RngCore;

use crate::error::VaultError;

const MAGIC: &[u8; 4] = b"RKVH";
const VERSION: u8 = 1;
/// The header's size on disk.
pub const VAULT_HEADER_LEN: usize = 64;

/// Argon2id parameters for a new vault: RFC 9106 §4's second recommended
/// option (64 MiB, 3 passes, 4 lanes), with a 64-byte master output.
pub const DEFAULT_M_COST_KIB: u32 = 65_536;
pub const DEFAULT_T_COST: u32 = 3;
pub const DEFAULT_P_COST: u32 = 4;
pub const MASTER_LEN: u32 = 64;

/// A vault's KDF parameters and salt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultHeader {
    pub m_cost_kib: u32,
    pub t_cost: u32,
    pub p_cost: u32,
    pub output_len: u32,
    pub salt: [u8; 32],
}

impl VaultHeader {
    /// The header for a new vault: default parameters, a fresh salt.
    #[must_use]
    pub fn fresh() -> Self {
        let mut salt = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut salt);
        Self {
            m_cost_kib: DEFAULT_M_COST_KIB,
            t_cost: DEFAULT_T_COST,
            p_cost: DEFAULT_P_COST,
            output_len: MASTER_LEN,
            salt,
        }
    }

    /// The Argon2id parameters this header names.
    ///
    /// # Errors
    /// Parameters `argon2` refuses (a corrupt header).
    pub fn params(&self) -> Result<argon2::Params, VaultError> {
        argon2::Params::new(
            self.m_cost_kib,
            self.t_cost,
            self.p_cost,
            Some(self.output_len as usize),
        )
        .map_err(|e| VaultError::Kdf(e.to_string()))
    }

    #[must_use]
    pub fn encode(&self) -> [u8; VAULT_HEADER_LEN] {
        let mut out = [0u8; VAULT_HEADER_LEN];
        out[..4].copy_from_slice(MAGIC);
        out[4] = VERSION;
        out[8..12].copy_from_slice(&self.m_cost_kib.to_le_bytes());
        out[12..16].copy_from_slice(&self.t_cost.to_le_bytes());
        out[16..20].copy_from_slice(&self.p_cost.to_le_bytes());
        out[20..24].copy_from_slice(&self.output_len.to_le_bytes());
        out[24..56].copy_from_slice(&self.salt);
        out
    }

    /// Parse a header.
    ///
    /// # Errors
    /// Wrong length, magic or version.
    pub fn decode(bytes: &[u8]) -> Result<Self, VaultError> {
        if bytes.len() != VAULT_HEADER_LEN || &bytes[..4] != MAGIC || bytes[4] != VERSION {
            return Err(VaultError::Schema("not a Rekindle vault header".into()));
        }
        let u32_at = |i: usize| {
            let mut b = [0u8; 4];
            b.copy_from_slice(&bytes[i..i + 4]);
            u32::from_le_bytes(b)
        };
        let mut salt = [0u8; 32];
        salt.copy_from_slice(&bytes[24..56]);
        Ok(Self {
            m_cost_kib: u32_at(8),
            t_cost: u32_at(12),
            p_cost: u32_at(16),
            output_len: u32_at(20),
            salt,
        })
    }
}

/// `{vault}.hdr`.
#[must_use]
pub fn header_path(vault_path: &Path) -> PathBuf {
    let mut p = vault_path.as_os_str().to_owned();
    p.push(".hdr");
    PathBuf::from(p)
}

/// Read a vault's header.
///
/// # Errors
/// [`VaultError::HeaderMissing`] when there is none; I/O or a malformed header.
pub fn read(vault_path: &Path) -> Result<VaultHeader, VaultError> {
    let path = header_path(vault_path);
    match std::fs::read(&path) {
        Ok(bytes) => VaultHeader::decode(&bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(VaultError::HeaderMissing),
        Err(e) => Err(e.into()),
    }
}

/// Write a new vault's header, owner-only. Refuses to replace one.
///
/// # Errors
/// [`VaultError::AlreadyExists`] or I/O.
pub fn write_new(vault_path: &Path, header: &VaultHeader) -> Result<(), VaultError> {
    use std::io::Write;
    let path = header_path(vault_path);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path).map_err(|e| match e.kind() {
        std::io::ErrorKind::AlreadyExists => VaultError::AlreadyExists,
        _ => e.into(),
    })?;
    file.write_all(&header.encode())?;
    file.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_header_round_trips_and_garbage_is_refused() {
        let h = VaultHeader::fresh();
        assert_eq!(VaultHeader::decode(&h.encode()).unwrap(), h);
        assert!(h.params().is_ok());
        assert!(VaultHeader::decode(&[0u8; VAULT_HEADER_LEN]).is_err());
        assert!(VaultHeader::decode(&h.encode()[..10]).is_err());
    }

    #[test]
    fn a_header_is_never_replaced() {
        let dir = tempfile::TempDir::new().unwrap();
        let vault = dir.path().join("a.vault");
        assert!(matches!(read(&vault), Err(VaultError::HeaderMissing)));
        write_new(&vault, &VaultHeader::fresh()).unwrap();
        assert!(matches!(
            write_new(&vault, &VaultHeader::fresh()),
            Err(VaultError::AlreadyExists)
        ));
    }
}
