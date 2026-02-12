@0xff610fc1912eb1ec;

struct VoiceSignaling {
    type @0 :SignalType;
    channelId @1 :Text;
    senderKey @2 :Data;
    payload @3 :Data;

    enum SignalType {
        join @0;
        leave @1;
        offer @2;
        answer @3;
        iceCandidate @4;
    }
}
