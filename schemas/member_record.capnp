@0x856a8fa1b28e4564;

# A member's own DFLT(8) MemberRecord `M` (ADR 0011, plan E3.2, D15
# subkey map): the formats of subkey 0 (card), 1 (beat) and 5 (history
# ad). E3.3 and E3.4 extend this file; a retired field keeps its ordinal
# as `obsolete…`.

# ── Subkey 0: the member card ─────────────────────────────────────────

# The card as written: `body` is a packed `MemberCardBody` message and
# `sig` is the member's pseudonym signature over
# `rekindle-member-card-v1 ‖ community ‖ memberRecord ‖ body`, so a reader
# verifies the exact bytes it decodes.
struct MemberCard {
  body @0 :Data;
  sig @1 :Data;
}

# The per-community profile.
struct MemberCardBody {
  displayName @0 :Text;
  bio @1 :Text;
  avatarRef @2 :Text;
  bannerRef @3 :Text;
  pronouns @4 :Text;
  hasThemeColor @5 :Bool;
  themeColor @6 :UInt32;       # ARGB
  badges @7 :List(Text);
  customStatus @8 :Text;
  game @9 :CardGame;
  hpkeKey @10 :Data;           # X25519 public key, 32 bytes
  hpkeSig @11 :Data;           # pseudonym sig: rekindle-card-hpke-v1 ‖ community ‖ hpkeKey
}

struct CardGame {
  gameName @0 :Text;
  gameId @1 :Text;
  hasElapsedSeconds @2 :Bool;
  elapsedSeconds @3 :UInt64;
  serverAddress @4 :Text;
}

# ── Subkey 1: the presence beat (exactly 512 B) ───────────────────────

enum BeatStatus {
  online @0;
  away @1;
  busy @2;
  offline @3;
  departed @4;                 # the member left the community
}

# `sig` is the pseudonym's over `rekindle-presence-beat-v2 ‖ community ‖
# memberRecord ‖ seq ‖ status ‖ voiceChannel ‖ beatNumber ‖ extras ‖
# padding`. `seq` counts the member's beats (persisted, never reused);
# `beatNumber` is `floor(writer's clock ms / 300 000)`, the 5-minute
# period the beat was written in. `padding` is random bytes sized after
# packing so the packed beat is exactly 512 B, its size leaking nothing
# (plan E3.4 Q2); it is allocated last, so it ends the message.
struct PresenceBeat {
  seq @0 :UInt64;
  status @1 :BeatStatus;
  voiceChannel @2 :Text;
  beatNumber @3 :UInt64;
  extras @4 :SealedExtras;
  sig @5 :Data;
  padding @6 :Data;
}

# A `SessionExtrasBody` sealed under the community text key of
# `mekGeneration`, AAD = community ‖ memberRecord.
struct SealedExtras {
  mekGeneration @0 :UInt64;
  sealed @1 :Data;
}

struct SessionExtrasBody {
  location :union {
    none @0 :Void;
    text @1 :Text;             # channel id
    voice @2 :Text;            # channel id
  }
  activity @3 :Text;
}

# ── Subkey 5: the history ad (≤ 4 KiB) ────────────────────────────────

# `sealed` is the community text key's AEAD (generation `mekGeneration`,
# AAD = community ‖ memberRecord) over a packed `HistoryRanges`; `sig` is
# the pseudonym's over `rekindle-history-ad-v1 ‖ community ‖ memberRecord
# ‖ mekGeneration ‖ sealed`.
struct HistoryAdRecord {
  mekGeneration @0 :UInt64;
  sealed @1 :Data;
  sig @2 :Data;
}

struct HistoryRanges {
  ranges @0 :List(HistoryRange);
}

struct HistoryRange {
  channelId @0 :Data;          # 16 bytes
  oldestLamport @1 :UInt64;
  newestLamport @2 :UInt64;
}

# ── Subkey 6: the member's routes (plan E3.2 S7c.8) ─────────────────

# The general private route blob every app_call to the member uses,
# rewritten each time the member's route is replaced. `issuedAt` is the
# writer's clock (ms); `sig` is the pseudonym's over
# `rekindle-member-routes-v1 ‖ community ‖ memberRecord ‖ issuedAt ‖
# general`.
struct MemberRoutes {
  general @0 :Data;
  issuedAt @1 :UInt64;
  sig @2 :Data;
}

# ── The ring hint (plan E3.2 S7, evidence §4.3) ──────────────────────

# A ring watcher saw `author`'s member record `memberRecord` advance
# `subkey` (1, 2, 3 or 4; 6 is reserved for E3.4's MemberRoutes) to `seq`.
# No content: the recipient reads the writer-signed value. `sig` is the
# relaying member's pseudonym over `rekindle-head-advanced-v1 ‖ community ‖
# author ‖ memberRecord ‖ subkey ‖ seq`. Carried in `headAdvanced @76`.
struct HeadAdvanced {
  author @0 :Data;
  memberRecord @1 :Text;
  subkey @2 :UInt32;
  seq @3 :UInt64;
  sig @4 :Data;
}

# ── Subkey 4: the channel outbox (plan E3.3) ─────────────────────────

# `M`/4 holds the head of the member's channel outbox chain (the N2 chain,
# `PagePointer` to the newest sealed page, as `M`/2 holds the op log);
# each chain entry is one `OutboxEntry`. Its chain position is the N2 seq,
# so no index is stored. `prev` is BLAKE3 of the previous entry's bytes in
# the author's chain (empty for the first), so a reader checks continuity
# over ciphertext. `channelTag` is `HKDF(text key of generation,
# rekindle-channel-tag-v1 ‖ channelId)[..16]`; `streamSeq` is the
# per-(author, tag) counter frozen at creation, the body AAD position and
# the dedup key. `sealed` is the kind's entry (a JSON `ChannelRecordEntry`)
# whose body is sealed at `rekindle-channel-body-aad-v1 ‖ anchor ‖
# channel_id ‖ author ‖ stream_seq_le64`. `sig` is the author pseudonym's
# over `rekindle-outbox-entry-v1 ‖ anchor ‖ chain seq ‖` every field above.
struct OutboxEntry {
  prev @0 :Data;
  channelTag @1 :Data;         # 16 bytes
  streamSeq @2 :UInt64;
  generation @3 :UInt64;
  committedAt @4 :UInt64;      # ms, the author's clock at commit
  kind @5 :OutboxEntryKind;
  sealed @6 :Data;
  sig @7 :Data;
}

enum OutboxEntryKind {
  message @0;
  forward @1;
  attachmentCached @2;
  reaction @3;
  pollCreate @4;
  pollVote @5;
  pollClose @6;
  handRaise @7;
  edit @8;
  delete @9;
}
