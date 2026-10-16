# 0016 — Call media is keyed by sender-owned keys (RFC 9605 §5.1)

- **Status:** Accepted (owner ruling Q4, 2026-10-15)
- **Date:** 2026-10-15
- **Plan steps:** C7.20 (built 2026-10-07), C7.22 (delivery), C7.29 (the four fixes below), E4.2 (group calls
  adopt the keyring) in `.claude/plans/standards-remediation/00-integration-plan.md` (v2.2)
- **Research:** `.claude/plans/standards-remediation/realignment/r8-media-keys.md`. Primary sources are saved in
  `.claude/plans/standards-remediation/evidence/sources/realignment/media-keys/` (`SOURCES.txt` lists URLs and
  commits).
- **Related:** [0012](0012-community-keys-stay-on-mek.md) governs persistent channel **text** (MEK). This ADR
  governs call **media**, which is not under a MEK. [0015](0015-media-engine.md) is the media engine;
  [0018](0018-mek-rotation-ballots.md) is the MEK rotation agreement, which does not apply to media.

## Context and problem statement

Community voice used one shared key per voice channel, re-minted by an elected member on every join and
leave. Two members sometimes minted at the same moment. Each side then held a different key under the same
generation, and every frame failed AEAD (the C7.15 run: up to 210 `Rejected` drops per 5 s). A shared key
needs an agreement step, and agreement among peers with no server is exactly what the network cannot provide
cheaply (r8 §2.8; DAVE and WhatsApp converge only because a central server breaks ties).

C7.20 (2026-10-07) replaced it with sender-owned keys: each participant makes its own random key, sends it
pairwise to each listener, acknowledged and retried, and makes a fresh key when someone leaves. That stopped
the dropouts. The plan recorded Q4 as "resolved by research", and D1, D8, D20, E3.5 and E4.2 still described
the channel-MEK call key. The owner ruled on 2026-10-15, after r8.

## Evidence

All six systems studied give each sender its own key (r8 §2):

- **RFC 9605 (SFrame) §5.1** (`rfc9605.txt`): per-sender keys, and a forward ratchet
  `base_key[i+1] = HKDF-Expand(HKDF-Extract("", base_key[i]), "SFrame 1.0 Ratchet", Nh)` (L786-790).
- **Signal RingRTC** (`ringrtc_group_call.rs`, `ringrtc_frame_crypto.rs`, commit `331d6018`): per-sender keys,
  ratchet on join, new key on leave after a delay, one pending rotation then rotate again (L3630-3700),
  `MAX_RECEIVER_STATES_TO_RETAIN = 5`, `MAX_OOO_RATCHETS = 5` (`ringrtc_frame_crypto.rs:26,31`).
- **MatrixRTC MSC4143** (`msc4143.md`) and its reference `RTCEncryptionManager.ts`; **Jitsi**
  (`jitsi_ManagedKeyHandler.js`, `jitsi_OlmAdapter.js`); **Discord DAVE** (`dave_protocol.md`); **WhatsApp**
  (`whatsapp_security_whitepaper.txt` pp. 21-22).
- **Key transport.** None of the six seals media keys to a long-term static key; all use a forward-secret
  pairwise channel. RFC 9180 §9.1 (`rfc9180.txt` L1315-1318): "HPKE does not provide forward secrecy with
  respect to recipient compromise. In the Base and Auth modes, the secrecy properties are only expected to hold
  if the recipient private key skR is not compromised at any point in time."

## Decision outcome

**Call media is keyed by sender-owned keys, per RFC 9605 §5.1, for every scope:** a random per-sender secret
for community voice channels and group calls, and ephemeral ECDH for DM calls (`rekindle-calls`, already
forward-secret). C7.20 is the design of record. The KID is `generation ‖ ratchet step`: a leave bumps the
generation (new random key), a join bumps the ratchet step.

C7.20 as built has four gaps, fixed in plan step **C7.29** before E4.2:

| Gap | As built (code at `27f19f1d`) | Fix |
|---|---|---|
| **G1** A joiner can read earlier media | `keyring.rs:125-136` `on_join` shares the current key if younger than `SHARE_GRACE` (10 s); `media_keys.rs` answers `VoiceMediaKeyRequest` with `shareable()` = current + pending, so a joiner gets the key in use since the last rotation | Ratchet forward on join (RFC 9605 §5.1, RingRTC, Jitsi; intent T6). The new state goes only to the joiner; a request is answered with the current state only. `SHARE_GRACE` and `JoinShare` are deleted. |
| **G2** Key transport is not forward-secret | `media_sender_key/mod.rs` seals with HPKE Auth mode to the member's long-lived community pseudonym (`derive.rs:18`) | Each participant makes an X25519 keypair per channel session, signed by its pseudonym and carried on the join handshake; `VoiceMediaKey` is HPKE Base to that key plus a pseudonym signature; the private half is zeroized at session end. Pre-D1 format change. |
| **G3** Back-to-back leaves postpone rotation and reuse an index | A leave during a pending rotation replaces the pending key at the same index and restarts the 5 s delay | RingRTC's rule: at most one pending rotation, applied on schedule, then another; every rotation takes a new generation. |
| **G4** Stage channels never rotate | `on_peer_removed` returns before rotating when `is_stage` (`media_keys.rs`) | A community departure (kick, ban, leave) always rotates; only a voluntary stage-listener leave skips it. |

Group calls (`rekindle-calls/src/group.rs`, initiator key wrapped per participant) adopt the same keyring in
E4.2, and `wrap_call_key` and `GroupCallState.call_key` are deleted there.

**Rejected:** a channel-MEK-derived call key (returns to the agreement problem, and D20's ballot would put a
multi-second DHT agreement in front of every join and leave); keeping share-or-rotate (MatrixRTC's grace,
which its own reference implementation marks temporary).

## Consequences

- **Promises to users, after C7.29** (r8 §1): a leaver, kicked or banned member cannot read media sent more
  than ~10 s after leaving (RingRTC's documented bound under churn); a joiner cannot read media from before
  joining; a device or recovery phrase seized later does not open recorded calls, because the session key that
  sealed the media keys was erased.
- **Today (C7.20 as built), honestly:** the leave property holds except for kicked stage listeners; the join
  and seizure properties do **not** hold. The threat model and `privacy-properties.md` say so until C7.29 lands.
- No in-session post-compromise security and no periodic rotation (DAVE declines it for short-lived media;
  RingRTC rotates only on membership change). A > 4 MCU host is a participant and hears plaintext until E4.4's
  forwarding-only relay.
- ControlPayload @71/@72/@73 (`voiceMediaKey`, `voiceMediaKeyRequest`, `voiceMediaKeyAck`) stay allocated
  (`schemas/community_envelope.capnp:773-775`). C7.29 adds one field per join payload and a signature on
  `VoiceMediaKeyPayload`.
- D1, D8, D20, E3.5 and E4.2 drop "voice MEK" text: there is no voice MEK.

## More information

- [RFC 9605](https://www.rfc-editor.org/rfc/rfc9605), [RFC 9180](https://www.rfc-editor.org/rfc/rfc9180),
  [Signal: group calls](https://signal.org/blog/how-to-build-encrypted-group-calls/),
  [DAVE](https://daveprotocol.com/), [MSC4143](https://github.com/matrix-org/matrix-spec-proposals/pull/4143)
- [`../architecture/voice.md`](../architecture/voice.md), [`../security/threat-model.md`](../security/threat-model.md)
