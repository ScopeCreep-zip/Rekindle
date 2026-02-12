@0xd30521ff511c9ab5;

struct AccountHeader {
    contactListKey @0 :Text;
    chatListKey @1 :Text;
    invitationListKey @2 :Text;
    displayName @3 :Text;
    statusMessage @4 :Text;
    avatarHash @5 :Data;
    createdAt @6 :UInt64;
    updatedAt @7 :UInt64;
    contactListKeypair @8 :Text;
    chatListKeypair @9 :Text;
    invitationListKeypair @10 :Text;
}

struct ContactEntry {
    publicKey @0 :Data;
    displayName @1 :Text;
    nickname @2 :Text;
    group @3 :Text;
    localConversationKey @4 :Text;
    remoteConversationKey @5 :Text;
    addedAt @6 :UInt64;
    updatedAt @7 :UInt64;
}

struct ChatEntry {
    contactPublicKey @0 :Data;
    localConversationKey @1 :Text;
    lastMessageTimestamp @2 :UInt64;
    unreadCount @3 :UInt32;
    isPinned @4 :Bool;
    isMuted @5 :Bool;
}
