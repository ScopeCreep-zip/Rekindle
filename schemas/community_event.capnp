# Architecture §21 — scheduled-event metadata for `EventCreated` /
# `EventUpdated` envelopes plus the typed `GovernanceEntry::EventCreated`.
@0x8efb519101dd36ff;

# Architecture §21 — EventStatus lifecycle states.
enum EventStatus @0xa7d4eb16e9bf1c2e {
    scheduled @0;
    active    @1;
    completed @2;
    cancelled @3;
}

enum RecurrenceFrequency @0xa8d4eb16e9bf1c2e {
    daily   @0;
    weekly  @1;
    monthly @2;
}

enum DayOfWeek @0xa9d4eb16e9bf1c2e {
    sunday    @0;
    monday    @1;
    tuesday   @2;
    wednesday @3;
    thursday  @4;
    friday    @5;
    saturday  @6;
}

# Architecture §21 line 2628 — RFC-5545-subset recurrence rule. Either
# `until` or `count` may bound the recurrence (or neither — repeats
# forever). `daysOfWeek` is meaningful only for `weekly`. Optionality
# is signalled by `hasUntil` / `hasCount`.
struct RecurrenceRule @0xaad4eb16e9bf1c2e {
    frequency    @0 :RecurrenceFrequency;
    interval     @1 :UInt32;
    daysOfWeek   @2 :List(DayOfWeek);
    hasUntil     @3 :Bool;
    until        @4 :UInt64;
    hasCount     @5 :Bool;
    count        @6 :UInt32;
}

# Architecture §21 line 2629 — event location.
struct EventLocation @0xabd4eb16e9bf1c2e {
    union {
        # 16-byte UUID hex of the voice channel.
        voiceChannel  @0 :Text;
        # 16-byte UUID hex of the stage channel.
        stageChannel  @1 :Text;
        # External link (Twitch, YouTube, Discord, etc.).
        external      @2 :Text;
        inGame        @3 :InGameLocation;
    }
}

struct InGameLocation @0xacd4eb16e9bf1c2e {
    # Rich-presence game id.
    gameId        @0 :UInt32;
    # `host:port` string the launcher uses. Empty string = no server
    # bound (caller treats empty as absent).
    serverAddress @1 :Text;
}

# Architecture §21 — RSVP entry attached to an EventInfo.
struct EventRsvp @0xadd4eb16e9bf1c2e {
    # Hex-encoded community pseudonym.
    pseudonymKey @0 :Text;
    # `going` / `interested` / `declined`.
    status       @1 :Text;
}

# Full event document broadcast over `EventCreated` / `EventUpdated`
# envelopes.
struct EventInfo @0xaed4eb16e9bf1c2e {
    # `evt_<32 hex>` — 16-byte UUID.
    id                @0 :Text;
    title             @1 :Text;
    description       @2 :Text;
    # Hex-encoded creator pseudonym.
    creatorPseudonym  @3 :Text;
    startTime         @4 :UInt64;
    hasEndTime        @5 :Bool;
    endTime           @6 :UInt64;
    # 16-byte UUID hex of the bound channel; empty string = unbound.
    channelId         @7 :Text;
    hasMaxAttendees   @8 :Bool;
    maxAttendees      @9 :UInt32;
    createdAt         @10 :UInt64;
    # `scheduled` / `active` / `completed` / `cancelled` (string form
    # for the DTO surface; cf. EventStatus enum for the typed form).
    status            @11 :Text;
    rsvps             @12 :List(EventRsvp);
    # Architecture §21 line 2624 — peer-cached cover image content
    # hash. Empty string = no cover.
    coverImageRef     @13 :Text;
    hasRecurrence     @14 :Bool;
    recurrence        @15 :RecurrenceRule;
    hasLocation       @16 :Bool;
    location          @17 :EventLocation;
}
