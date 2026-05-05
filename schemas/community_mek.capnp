# Architecture §6 — channel MEK delivery shape used inside
# `BootstrapResponse.channelMeks`. The MEK itself is wrapped (sealed
# to the recipient's pseudonym key) so this struct carries opaque
# ciphertext, not raw key material.
@0xc98813ffab0673f8;

struct ChannelMekDelivery @0xa3d4eb16e9bf1c2e {
    # 16-byte UUID hex of the channel this MEK belongs to. Empty string
    # when the delivery is the community-wide MEK (architecture §6.4).
    channelId       @0 :Text;
    # Architecture §6.7 — monotonically increasing generation counter
    # the rotator bumps on every MEK rotation.
    generation      @1 :UInt64;
    # Wrapped MEK ciphertext (sealed to the recipient's pseudonym
    # public key via the deterministic ECDH wrapper).
    wrappedMek      @2 :Data;
}
