//! The vault header `{vault}.hdr` (plan D1): the Argon2id parameters and
//! salt a passphrase is stretched with, readable before the vault itself
//! (they are needed to derive its key). 128 bytes, plaintext: neither the
//! parameters nor the salts are secret, only unique per vault.
//!
//! Layout (little-endian): `b"RKVH"`, version `2`, 3 zero bytes,
//! `m_cost` KiB, `t_cost`, `p_cost`, output length (4 × u32), the 32-byte
//! salt, the 32-byte duress salt, the 32-byte duress tag, 8 zero bytes.
//!
//! Version 2 (plan E7.1, quick wipe) adds the duress pair
//! ([`crate::duress`]). An unarmed header holds random bytes in both, so a
//! header does not show whether a duress passphrase exists. A version-1
//! header is refused: vaults reset with the schema (pre-release, no
//! legacy path).

use std::path::{Path, PathBuf};

use rand::RngCore;

use crate::error::VaultError;

const MAGIC: &[u8; 4] = b"RKVH";
const HEADER_VERSION: u8 = 2;
/// The header's size on disk.
#[cfg(test)]
thread_local! {
    /// The Argon2id derivations this thread ran (the duress timing-parity
    /// test, `crate::duress`).
    pub(crate) static STRETCHES: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

pub const VAULT_HEADER_LEN: usize = 128;

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
    /// The salt of the duress derivation (random while unarmed).
    pub duress_salt: [u8; 32],
    /// `Argon2id(duress passphrase, duress_salt, these params)[..32]`, or
    /// random bytes while unarmed.
    pub duress_tag: [u8; 32],
}

impl VaultHeader {
    /// The header for a new vault: default parameters, a fresh salt.
    #[must_use]
    pub fn fresh() -> Self {
        let mut salt = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut salt);
        let mut header = Self {
            m_cost_kib: DEFAULT_M_COST_KIB,
            t_cost: DEFAULT_T_COST,
            p_cost: DEFAULT_P_COST,
            output_len: MASTER_LEN,
            salt,
            duress_salt: [0u8; 32],
            duress_tag: [0u8; 32],
        };
        header.randomize_duress();
        header
    }

    /// Fill the duress pair with random bytes: the unarmed state.
    pub fn randomize_duress(&mut self) {
        rand::rngs::OsRng.fill_bytes(&mut self.duress_salt);
        rand::rngs::OsRng.fill_bytes(&mut self.duress_tag);
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

    /// `Argon2id(passphrase, salt, these params)`, `output_len` bytes: the
    /// vault master under [`Self::salt`], the duress tag's source under
    /// [`Self::duress_salt`].
    ///
    /// # Errors
    /// Parameters `argon2` refuses (a corrupt header).
    pub fn stretch(
        &self,
        passphrase: &str,
        salt: &[u8; 32],
    ) -> Result<zeroize::Zeroizing<Vec<u8>>, VaultError> {
        let argon = argon2::Argon2::new(
            argon2::Algorithm::Argon2id,
            argon2::Version::V0x13,
            self.params()?,
        );
        let mut out = zeroize::Zeroizing::new(vec![0u8; self.output_len as usize]);
        #[cfg(test)]
        STRETCHES.with(|n| n.set(n.get() + 1));
        argon
            .hash_password_into(passphrase.as_bytes(), salt, &mut out)
            .map_err(|e| VaultError::Kdf(e.to_string()))?;
        Ok(out)
    }

    #[must_use]
    pub fn encode(&self) -> [u8; VAULT_HEADER_LEN] {
        let mut out = [0u8; VAULT_HEADER_LEN];
        out[..4].copy_from_slice(MAGIC);
        out[4] = HEADER_VERSION;
        out[8..12].copy_from_slice(&self.m_cost_kib.to_le_bytes());
        out[12..16].copy_from_slice(&self.t_cost.to_le_bytes());
        out[16..20].copy_from_slice(&self.p_cost.to_le_bytes());
        out[20..24].copy_from_slice(&self.output_len.to_le_bytes());
        out[24..56].copy_from_slice(&self.salt);
        out[56..88].copy_from_slice(&self.duress_salt);
        out[88..120].copy_from_slice(&self.duress_tag);
        out
    }

    /// Parse a header.
    ///
    /// # Errors
    /// Wrong length, magic or version.
    pub fn decode(bytes: &[u8]) -> Result<Self, VaultError> {
        if bytes.len() != VAULT_HEADER_LEN || &bytes[..4] != MAGIC || bytes[4] != HEADER_VERSION {
            return Err(VaultError::Schema("not a Rekindle vault header".into()));
        }
        let u32_at = |i: usize| {
            let mut b = [0u8; 4];
            b.copy_from_slice(&bytes[i..i + 4]);
            u32::from_le_bytes(b)
        };
        let bytes32 = |i: usize| {
            let mut b = [0u8; 32];
            b.copy_from_slice(&bytes[i..i + 32]);
            b
        };
        Ok(Self {
            m_cost_kib: u32_at(8),
            t_cost: u32_at(12),
            p_cost: u32_at(16),
            output_len: u32_at(20),
            salt: bytes32(24),
            duress_salt: bytes32(56),
            duress_tag: bytes32(88),
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
    write_file(&header_path(vault_path), header)
}

/// Write `header` to a new owner-only file at `path`, synced.
fn write_file(path: &Path, header: &VaultHeader) -> Result<(), VaultError> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::AlreadyExists => VaultError::AlreadyExists,
        _ => e.into(),
    })?;
    file.write_all(&header.encode())?;
    file.sync_all()?;
    Ok(())
}

/// Replace an existing vault's header atomically: `{vault}.hdr.new`,
/// synced, renamed over it, the directory synced (plan E7.1, arming and
/// disarming the duress passphrase).
///
/// # Errors
/// [`VaultError::HeaderMissing`] when there is none to replace; I/O.
pub fn rewrite(vault_path: &Path, header: &VaultHeader) -> Result<(), VaultError> {
    let path = header_path(vault_path);
    if !path.exists() {
        return Err(VaultError::HeaderMissing);
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".new");
    let tmp = PathBuf::from(tmp);
    match std::fs::remove_file(&tmp) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    write_file(&tmp, header)?;
    std::fs::rename(&tmp, &path)?;
    if let Some(dir) = path.parent() {
        std::fs::File::open(dir)?.sync_all()?;
    }
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
