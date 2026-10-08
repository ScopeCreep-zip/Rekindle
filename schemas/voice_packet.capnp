@0xba1432716e676983;

# One encrypted voice frame (plan step B3). Travels after the one-byte
# media tag 'V' on Veilid app_message.
struct VoicePacket {
    # Signing key of the sender (32 bytes): identity key for calls,
    # community pseudonym for channels. Selects the SFrame sender key.
    senderKey @0 :Data;
    # Per-sender frame sequence, for the jitter buffer.
    sequence @1 :UInt32;
    # Sender clock, milliseconds.
    timestamp @2 :UInt64;
    # RFC 9605 SFrame ciphertext: header || AES-256-GCM(level || opus).
    # The first plaintext byte is the VAD audio level (plan step E4; 0
    # until then). The transport-wide sequence for congestion feedback
    # is not here: it is stamped per route in the media datagram header
    # (plan E4.3.1).
    sframe @3 :Data;
    # Ed25519 signature by senderKey over rekindle-voice-packet-v3 ||
    # senderKey || sequence || timestamp || sframe.
    sig @4 :Data;
}
