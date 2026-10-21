@0xd76315b87ba233f9;

# The append-only chain (plan E2.5, primitive N2): a participant record's
# HEAD names the newest page, each page names the one before it, and every
# page is a sealed DFLT(1) record written once. Entries are opaque
# ciphertexts; sequence numbers start at 1 and run without gaps.

# Where a page sits: its record key and the sequence numbers it holds.
struct PagePointer {
    pageKey @0 :Text;
    firstSeq @1 :UInt64;
    lastSeq @2 :UInt64;
}

# Participant record subkey 0 (dm_participant::HEAD): the newest page.
struct ChainHead {
    newest @0 :PagePointer;
}

# Participant record subkey 1 (dm_participant::RETIRE_THROUGH): every entry
# at or below `throughSeq` is retired and no longer fetched.
struct ChainRetire {
    throughSeq @0 :UInt64;
}

# A page's only subkey: its entries, from `firstSeq` up, and the page
# before it (unset on the first page).
struct ChainPage {
    prev @0 :PagePointer;
    firstSeq @1 :UInt64;
    entries @2 :List(Data);
}
