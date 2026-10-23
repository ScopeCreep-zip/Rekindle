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

# ── Subkey 1: the presence beat (≤ 512 B) ─────────────────────────────

enum BeatStatus {
  online @0;
  away @1;
  busy @2;
  offline @3;
}

# `sig` is the pseudonym's over `rekindle-presence-beat-v1 ‖ community ‖
# memberRecord ‖ seq ‖ status ‖ voiceChannel ‖ expiresAt ‖ extras`.
struct PresenceBeat {
  seq @0 :UInt64;
  status @1 :BeatStatus;
  voiceChannel @2 :Text;
  expiresAt @3 :UInt64;
  extras @4 :SealedExtras;
  sig @5 :Data;
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
