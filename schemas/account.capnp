@0xd30521ff511c9ab5;

# The owner-encrypted header in subkey 0 of the account DHT record.
#
# This used to also carry pointers and owner keypairs for three child
# DHTShortArrays — a contact list, a chat list and an invitation list,
# the classic-Xfire account model. Nothing ever wrote to or read from
# them: friends live in their own friend-list record, conversations in
# ConversationRecord, and friend requests in the friend inbox. Creating
# an account allocated three DHT records that stayed empty for the
# account's whole life, and every login reopened all three.
struct AccountHeader {
    displayName @0 :Text;
    statusMessage @1 :Text;
    avatarHash @2 :Data;
    createdAt @3 :UInt64;
    updatedAt @4 :UInt64;
}
