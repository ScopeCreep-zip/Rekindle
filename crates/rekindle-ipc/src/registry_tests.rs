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

#[test]
fn an_agent_is_scoped_by_its_capabilities_and_others_are_not() {
    use crate::protocol::AgentCapability;
    let mut reg = ClearanceRegistry::new();
    reg.register(
        "bot".into(),
        [1; 32],
        SecurityLevel::Agent,
        AgentType::Bot,
        vec![AgentCapability::ChatWrite],
    );
    reg.register(
        "client-x".into(),
        [2; 32],
        SecurityLevel::Admin,
        AgentType::Human,
        Vec::new(),
    );
    let bot = reg.lookup(&[1; 32]).unwrap();
    assert!(bot.permits(Some(AgentCapability::ChatWrite)));
    assert!(!bot.permits(Some(AgentCapability::ImHistory)));
    assert!(!bot.permits(None));
    assert!(reg.lookup(&[2; 32]).unwrap().permits(None));
}
