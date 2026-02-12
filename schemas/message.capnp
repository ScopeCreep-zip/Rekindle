@0xf59af4be759a7de3;

struct MessageEnvelope {
    senderKey @0 :Data;          # Ed25519 public key (32 bytes)
    timestamp @1 :UInt64;        # Unix timestamp milliseconds
    nonce @2 :Data;              # Unique message nonce
    payload @3 :Data;            # Signal-encrypted ciphertext
    signature @4 :Data;          # Ed25519 signature of (timestamp + nonce + payload)
}

struct ChatMessage {
    body @0 :Text;
    attachments @1 :List(Attachment);
    replyTo @2 :Data;            # Nonce of message being replied to
}

struct Attachment {
    name @0 :Text;
    mimeType @1 :Text;
    size @2 :UInt64;
    dhtKey @3 :Data;             # DHT record key where file data is stored
    checksum @4 :Data;           # SHA-256 of file
}
