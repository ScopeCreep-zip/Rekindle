@0xdfc34e082362ae4b;

using Identity = import "identity.capnp";

struct ConversationHeader {
    identityPublicKey @0 :Data;
    profile @1 :Identity.UserProfile;
    messageLogKey @2 :Text;
    routeBlob @3 :Data;
    preKeyBundle @4 :Identity.PreKeyBundle;
    createdAt @5 :UInt64;
    updatedAt @6 :UInt64;
}
