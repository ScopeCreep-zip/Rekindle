@0x9822355b3b39e806;

# Plan E4.3.2: transport feedback for the per-route bandwidth estimator
# (evidence/e4-3-bandwidth-owner-design.md #4). RFC 8888-shaped: the
# receiver reports, for every transport_seq from beginSeq, whether the
# media datagram arrived and when, relative to reportTimeMs. Send times
# stay with the sender, as in RFC 8888 and libwebrtc transport-cc.
# Travels after the one-byte media tag 'T' on the peer's media route.
struct TransportFeedback {
    # Signing key of the reporting receiver (32 bytes): the key its own
    # media is signed with. Feedback is accepted only from a receiver
    # about its own reception (r6 R-BW6).
    reporterKey @0 :Data;
    # First transport_seq this report covers.
    beginSeq @1 :UInt32;
    # Receiver clock, milliseconds, when the report was built.
    reportTimeMs @2 :UInt64;
    # One entry per transport_seq from beginSeq: 0xFFFF = not received;
    # otherwise how long before reportTimeMs it arrived, in 1/1024 s
    # (RFC 8888 arrival time offset), capped at 0xFFFE.
    arrivals @3 :List(UInt16);
    # Ed25519 signature by reporterKey over rekindle-transport-feedback-v1
    # || reporterKey || beginSeq || reportTimeMs || arrivals.
    sig @4 :Data;
}
