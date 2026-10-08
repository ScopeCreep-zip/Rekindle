//! Clearance registry: maps agent names to verified identities and security
//! levels, with O(1) pubkey lookups via a reverse index.
//!
//! After Noise IK handshake, the server extracts the client's X25519 static
//! public key and resolves it here. Keys not in the registry receive
//! `SecurityLevel::Open` (ephemeral CLI clients).
//!
//! Adapted from open-sesame `core-ipc/src/registry.rs`.
//!
//! [RC-14] Every mutation method is tested for the consistency invariant.

use std::collections::HashMap;

use super::message::{AgentType, SecurityLevel};

/// An agent's verified identity, clearance, and key state.
#[derive(Debug, Clone)]
pub struct AgentIdentity {
    /// Current active X25519 static public key.
    pub current_pubkey: [u8; 32],
    /// Pending rotation pubkey (both valid during grace period).
    pub pending_pubkey: Option<[u8; 32]>,
    /// Security clearance level.
    pub security_level: SecurityLevel,
    /// Agent type classification.
    pub agent_type: AgentType,
    /// Declared capabilities (e.g., "translate", "moderate").
    pub capabilities: Vec<String>,
    /// Monotonic generation counter. Survives crash-restart.
    pub generation: u64,
}

/// Maps agent names to identities with O(1) pubkey lookups via reverse index.
#[derive(Debug, Clone, Default)]
pub struct ClearanceRegistry {
    /// Source of truth: agent_name → identity.
    identities: HashMap<String, AgentIdentity>,
    /// Derived reverse index: pubkey → agent_name.
    pubkey_index: HashMap<[u8; 32], String>,
}

impl ClearanceRegistry {
    /// Create an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an agent with initial pubkey, clearance, and type.
    /// Generation starts at 0. Overwrites any existing registration.
    pub fn register(
        &mut self,
        name: String,
        pubkey: [u8; 32],
        level: SecurityLevel,
        agent_type: AgentType,
        capabilities: Vec<String>,
    ) {
        // Remove existing pubkeys from reverse index.
        if let Some(existing) = self.identities.get(&name) {
            self.pubkey_index.remove(&existing.current_pubkey);
            if let Some(pending) = existing.pending_pubkey {
                self.pubkey_index.remove(&pending);
            }
        }

        let identity = AgentIdentity {
            current_pubkey: pubkey,
            pending_pubkey: None,
            security_level: level,
            agent_type,
            capabilities,
            generation: 0,
        };
        self.identities.insert(name.clone(), identity);
        self.pubkey_index.insert(pubkey, name);
        self.debug_assert_consistent();
    }

    /// Register with an explicit generation (used after revoke-then-reregister).
    pub fn register_with_generation(
        &mut self,
        name: String,
        pubkey: [u8; 32],
        level: SecurityLevel,
        agent_type: AgentType,
        generation: u64,
    ) {
        if let Some(existing) = self.identities.get(&name) {
            self.pubkey_index.remove(&existing.current_pubkey);
            if let Some(pending) = existing.pending_pubkey {
                self.pubkey_index.remove(&pending);
            }
        }

        let identity = AgentIdentity {
            current_pubkey: pubkey,
            pending_pubkey: None,
            security_level: level,
            agent_type,
            capabilities: Vec::new(),
            generation,
        };
        self.identities.insert(name.clone(), identity);
        self.pubkey_index.insert(pubkey, name);
        self.debug_assert_consistent();
    }

    /// Look up an agent identity by pubkey. O(1) via reverse index.
    #[must_use]
    pub fn lookup(&self, pubkey: &[u8; 32]) -> Option<&AgentIdentity> {
        let name = self.pubkey_index.get(pubkey)?;
        self.identities.get(name)
    }

    /// Look up the agent name for a pubkey. O(1).
    #[must_use]
    pub fn lookup_name(&self, pubkey: &[u8; 32]) -> Option<&str> {
        self.pubkey_index.get(pubkey).map(String::as_str)
    }

    /// Find an agent identity by name. O(1).
    #[must_use]
    pub fn find_by_name(&self, name: &str) -> Option<&AgentIdentity> {
        self.identities.get(name)
    }

    /// Register a pending rotation pubkey. Both keys valid during grace period.
    /// Returns true if the agent was found.
    pub fn register_pending(&mut self, agent_name: &str, new_pubkey: [u8; 32]) -> bool {
        let Some(identity) = self.identities.get_mut(agent_name) else {
            return false;
        };

        if let Some(old_pending) = identity.pending_pubkey {
            self.pubkey_index.remove(&old_pending);
        }

        identity.pending_pubkey = Some(new_pubkey);
        self.pubkey_index.insert(new_pubkey, agent_name.to_owned());
        self.debug_assert_consistent();
        true
    }

    /// Finalize rotation: promote pending to current, increment generation.
    pub fn finalize_rotation(&mut self, agent_name: &str) -> bool {
        let Some(identity) = self.identities.get_mut(agent_name) else {
            return false;
        };

        let Some(new_pubkey) = identity.pending_pubkey.take() else {
            return false;
        };

        self.pubkey_index.remove(&identity.current_pubkey);
        identity.current_pubkey = new_pubkey;
        identity.generation += 1;
        self.debug_assert_consistent();
        true
    }

    /// Revoke an agent: remove identity and all pubkey index entries.
    /// Returns the removed identity for generation continuity.
    pub fn revoke_by_name(&mut self, agent_name: &str) -> Option<AgentIdentity> {
        let identity = self.identities.remove(agent_name)?;
        self.pubkey_index.remove(&identity.current_pubkey);
        if let Some(pending) = identity.pending_pubkey {
            self.pubkey_index.remove(&pending);
        }
        self.debug_assert_consistent();
        Some(identity)
    }

    /// Number of registered agents.
    #[must_use]
    pub fn len(&self) -> usize {
        self.identities.len()
    }

    /// Whether the registry is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.identities.is_empty()
    }

    /// Validate internal consistency between primary map and reverse index.
    ///
    /// Invariant: every pubkey in reverse index maps to an existing identity
    /// whose current_pubkey or pending_pubkey matches. And vice versa.
    fn debug_assert_consistent(&self) {
        #[cfg(debug_assertions)]
        {
            for (pubkey, name) in &self.pubkey_index {
                let identity = self
                    .identities
                    .get(name)
                    .unwrap_or_else(|| panic!("pubkey_index → nonexistent agent: {name}"));
                let matches = identity.current_pubkey == *pubkey
                    || identity.pending_pubkey.as_ref() == Some(pubkey);
                assert!(
                    matches,
                    "pubkey_index entry for {name} matches neither current nor pending"
                );
            }

            for (name, identity) in &self.identities {
                assert_eq!(
                    self.pubkey_index.get(&identity.current_pubkey),
                    Some(name),
                    "current_pubkey for {name} missing from pubkey_index"
                );
                if let Some(pending) = &identity.pending_pubkey {
                    assert_eq!(
                        self.pubkey_index.get(pending),
                        Some(name),
                        "pending_pubkey for {name} missing from pubkey_index"
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_lookup() {
        let mut reg = ClearanceRegistry::new();
        let key = [0xAA; 32];
        reg.register(
            "agent-a".into(),
            key,
            SecurityLevel::Agent,
            AgentType::Bot,
            vec![],
        );
        let id = reg.lookup(&key).unwrap();
        assert_eq!(id.current_pubkey, key);
        assert_eq!(id.security_level, SecurityLevel::Agent);
        assert_eq!(id.generation, 0);
    }

    #[test]
    fn lookup_name() {
        let mut reg = ClearanceRegistry::new();
        let key = [0xAA; 32];
        reg.register(
            "agent-a".into(),
            key,
            SecurityLevel::Internal,
            AgentType::System,
            vec![],
        );
        assert_eq!(reg.lookup_name(&key), Some("agent-a"));
        assert_eq!(reg.lookup_name(&[0xBB; 32]), None);
    }

    #[test]
    fn register_overwrites() {
        let mut reg = ClearanceRegistry::new();
        reg.register(
            "a".into(),
            [0xAA; 32],
            SecurityLevel::Open,
            AgentType::Human,
            vec![],
        );
        reg.register(
            "a".into(),
            [0xBB; 32],
            SecurityLevel::Agent,
            AgentType::Bot,
            vec![],
        );
        assert!(reg.lookup(&[0xAA; 32]).is_none());
        assert!(reg.lookup(&[0xBB; 32]).is_some());
    }

    #[test]
    fn pending_allows_dual_key_lookup() {
        let mut reg = ClearanceRegistry::new();
        reg.register(
            "a".into(),
            [0xAA; 32],
            SecurityLevel::Agent,
            AgentType::Bot,
            vec![],
        );
        assert!(reg.register_pending("a", [0xBB; 32]));
        assert!(reg.lookup(&[0xAA; 32]).is_some());
        assert!(reg.lookup(&[0xBB; 32]).is_some());
    }

    #[test]
    fn finalize_rotation() {
        let mut reg = ClearanceRegistry::new();
        reg.register(
            "a".into(),
            [0xAA; 32],
            SecurityLevel::Agent,
            AgentType::Bot,
            vec![],
        );
        reg.register_pending("a", [0xBB; 32]);
        assert!(reg.finalize_rotation("a"));
        assert!(reg.lookup(&[0xAA; 32]).is_none());
        assert!(reg.lookup(&[0xBB; 32]).is_some());
        assert_eq!(reg.find_by_name("a").unwrap().generation, 1);
    }

    #[test]
    fn revoke_removes_both_keys() {
        let mut reg = ClearanceRegistry::new();
        reg.register(
            "a".into(),
            [0xAA; 32],
            SecurityLevel::Agent,
            AgentType::Bot,
            vec![],
        );
        reg.register_pending("a", [0xBB; 32]);
        let revoked = reg.revoke_by_name("a").unwrap();
        assert_eq!(revoked.generation, 0);
        assert!(reg.lookup(&[0xAA; 32]).is_none());
        assert!(reg.lookup(&[0xBB; 32]).is_none());
    }

    #[test]
    fn revoke_then_reregister_preserves_generation() {
        let mut reg = ClearanceRegistry::new();
        reg.register(
            "a".into(),
            [0xAA; 32],
            SecurityLevel::Agent,
            AgentType::Bot,
            vec![],
        );
        reg.register_pending("a", [0xBB; 32]);
        reg.finalize_rotation("a"); // gen = 1
        let revoked = reg.revoke_by_name("a").unwrap();
        assert_eq!(revoked.generation, 1);
        reg.register_with_generation(
            "a".into(),
            [0xCC; 32],
            SecurityLevel::Agent,
            AgentType::Bot,
            revoked.generation + 1,
        );
        assert_eq!(reg.find_by_name("a").unwrap().generation, 2);
    }
}
