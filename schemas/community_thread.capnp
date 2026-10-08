# Architecture §22 — thread metadata broadcast over the
# `ThreadCreated` control envelope.
@0xb9566278fb1a259a;

struct ThreadInfo @0xa1d4eb16e9bf1c2e {
    # 16-byte UUID hex (`thr_<32 hex>`).
    id                  @0 :Text;
    # 16-byte UUID hex of the parent channel.
    channelId           @1 :Text;
    name                @2 :Text;
    # Architecture §22 line 2670 — the originating message ID.
    starterMessageId    @3 :Text;
    # Hex-encoded creator pseudonym.
    creatorPseudonym    @4 :Text;
    # Forum-channel tag this thread is filed under, when applicable.
    # Empty string = no tag.
    forumTag            @5 :Text;
    createdAt           @6 :UInt64;
    archived            @7 :Bool;
    # Architecture §22 line 2675 — auto-archive timeout in seconds.
    autoArchiveSeconds  @8 :UInt32;
    lastMessageAt       @9 :UInt64;
    messageCount        @10 :UInt32;
}
