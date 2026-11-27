use super::*;

#[test]
fn administrator_bypasses_all() {
    let perms = Permissions(ADMINISTRATOR);
    assert!(perms.has(SEND_MESSAGES));
    assert!(perms.has(BAN_MEMBERS));
    assert!(perms.has(MANAGE_CHANNELS));
}

#[test]
fn specific_permission_check() {
    let perms = Permissions(SEND_MESSAGES | ADD_REACTIONS);
    assert!(perms.has(SEND_MESSAGES));
    assert!(perms.has(ADD_REACTIONS));
    assert!(!perms.has(BAN_MEMBERS));
}

#[test]
fn default_everyone_includes_basics() {
    let perms = Permissions(DEFAULT_EVERYONE);
    assert!(perms.has(VIEW_CHANNELS));
    assert!(perms.has(SEND_MESSAGES));
    assert!(perms.has(READ_HISTORY));
    assert!(perms.has(CONNECT));
    assert!(perms.has(SPEAK));
    assert!(!perms.has(BAN_MEMBERS));
    assert!(!perms.has(ADMINISTRATOR));
}

#[test]
fn bits_dont_overlap() {
    // Verify no two constants share a bit
    let all_consts = [
        VIEW_CHANNELS,
        MANAGE_CHANNELS,
        MANAGE_ROLES,
        MANAGE_COMMUNITY,
        CREATE_INVITES,
        KICK_MEMBERS,
        BAN_MEMBERS,
        TIMEOUT_MEMBERS,
        MANAGE_NICKNAMES,
        MANAGE_EXPRESSIONS,
        VIEW_AUDIT_LOG,
        VIEW_INSIGHTS,
        CREATE_EXPRESSIONS,
        SEND_MESSAGES,
        EMBED_LINKS,
        ATTACH_FILES,
        ADD_REACTIONS,
        MENTION_EVERYONE,
        MANAGE_MESSAGES,
        READ_HISTORY,
        SEND_TTS,
        USE_EXTERNAL_EMOJIS,
        USE_EXTERNAL_STICKERS,
        PIN_MESSAGES,
        SEND_VOICE_MESSAGES,
        SEND_POLLS,
        BYPASS_SLOWMODE,
        CONNECT,
        SPEAK,
        MUTE_MEMBERS,
        DEAFEN_MEMBERS,
        MOVE_MEMBERS,
        USE_VOICE_ACTIVITY,
        PRIORITY_SPEAKER,
        USE_SOUNDBOARD,
        USE_EXTERNAL_SOUNDS,
        REQUEST_TO_SPEAK,
        STREAM,
        MANAGE_THREADS,
        CREATE_PUBLIC_THREADS,
        CREATE_PRIVATE_THREADS,
        SEND_IN_THREADS,
        MANAGE_EVENTS,
        CREATE_EVENTS,
        ADMINISTRATOR,
    ];
    for (i, a) in all_consts.iter().enumerate() {
        for b in &all_consts[i + 1..] {
            assert_eq!(a & b, 0, "Permission bits overlap: {a:#x} and {b:#x}");
        }
    }
}
