//! Noise IK prologue construction, keypair generation, and builder helpers.

use crate::v4::wire::constants::PROLOGUE_PREFIX;
use super::resolver::noise_builder;

/// Noise IK protocol parameters — AES-256-GCM with SHA-256 via aws-lc-rs resolver.
pub const NOISE_PARAMS: &str = "Noise_IK_25519_AESGCM_SHA256";

#[derive(Debug)]
pub enum PrologueError {
    PidZero { which: &'static str },
}

#[derive(Debug)]
pub enum NoiseError {
    KeygenFailed(String),
    BuilderFailed(String),
}

/// Build the canonical UCred prologue for the Noise IK handshake.
///
/// Format: `"RTI-IPC-v1:{lower_pid}:{lower_uid}:{higher_pid}:{higher_uid}"`
/// PIDs are sorted lower-first for canonical ordering regardless of caller role.
/// PID 0 is rejected unconditionally.
pub fn build_prologue(
    local_pid: u32,
    local_uid: u32,
    remote_pid: u32,
    remote_uid: u32,
) -> Result<Vec<u8>, PrologueError> {
    if local_pid == 0 {
        return Err(PrologueError::PidZero { which: "local" });
    }
    if remote_pid == 0 {
        return Err(PrologueError::PidZero { which: "remote" });
    }

    let (lower_pid, lower_uid, higher_pid, higher_uid) = if local_pid <= remote_pid {
        (local_pid, local_uid, remote_pid, remote_uid)
    } else {
        (remote_pid, remote_uid, local_pid, local_uid)
    };

    let s = format!(
        "{PROLOGUE_PREFIX}{lower_pid}:{lower_uid}:{higher_pid}:{higher_uid}"
    );
    Ok(s.into_bytes())
}

/// Generate a Noise IK X25519 static keypair using the aws-lc-rs resolver.
pub fn generate_keypair() -> Result<snow::Keypair, NoiseError> {
    noise_builder(NOISE_PARAMS)
        .generate_keypair()
        .map_err(|e| NoiseError::KeygenFailed(format!("{e}")))
}

/// Build a Noise IK initiator (dialler) handshake state.
///
/// The initiator knows the responder's static public key in advance.
pub fn build_initiator(
    local_private_key: &[u8],
    remote_public_key: &[u8],
    prologue: &[u8],
) -> Result<snow::HandshakeState, NoiseError> {
    noise_builder(NOISE_PARAMS)
        .local_private_key(local_private_key)
        .map_err(|e| NoiseError::BuilderFailed(format!("local key: {e}")))?
        .remote_public_key(remote_public_key)
        .map_err(|e| NoiseError::BuilderFailed(format!("remote key: {e}")))?
        .prologue(prologue)
        .map_err(|e| NoiseError::BuilderFailed(format!("prologue: {e}")))?
        .build_initiator()
        .map_err(|e| NoiseError::BuilderFailed(format!("build initiator: {e}")))
}

/// Build a Noise IK responder (listener) handshake state.
pub fn build_responder(
    local_private_key: &[u8],
    prologue: &[u8],
) -> Result<snow::HandshakeState, NoiseError> {
    noise_builder(NOISE_PARAMS)
        .local_private_key(local_private_key)
        .map_err(|e| NoiseError::BuilderFailed(format!("local key: {e}")))?
        .prologue(prologue)
        .map_err(|e| NoiseError::BuilderFailed(format!("prologue: {e}")))?
        .build_responder()
        .map_err(|e| NoiseError::BuilderFailed(format!("build responder: {e}")))
}
