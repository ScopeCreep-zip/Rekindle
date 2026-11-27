use super::*;
use rekindle_types::channel_keys::Zeroizing;
use rekindle_types::id::ChannelId;

/// Generations 1 and 2 of the community key.
struct Keys;

impl ChannelKeyProvider for Keys {
    fn current_epoch(&self, _: &str, _: KeyScope) -> Option<KeyEpoch> {
        Some(KeyEpoch(2))
    }
    fn key(&self, _: &str, _: KeyScope, epoch: KeyEpoch) -> Option<Zeroizing<[u8; 32]>> {
        let byte = u8::try_from(epoch.0).ok().filter(|g| (1..=2).contains(g))?;
        Some(Zeroizing::new([byte; 32]))
    }
    fn scope_for_text(&self, _: &str, _: ChannelId) -> KeyScope {
        KeyScope::Community
    }
}

const AUTHOR: [u8; 32] = [4; 32];

fn at(channel_id: &str, stream_seq: u64) -> BodyPosition<'_> {
    BodyPosition {
        anchor: "VLD0:anchor",
        channel_id,
        author: &AUTHOR,
        stream_seq,
    }
}

/// A reader finds the entry's channel by its tag and opens the body at
/// the entry's position; at another position it does not open.
#[test]
fn a_sealed_body_names_its_channel_and_opens_only_in_place() {
    let sealed = seal_entry_body(&Keys, "c", KeyScope::Community, at("chan-b", 7), b"hi").unwrap();
    assert_eq!(sealed.generation, 2);
    let streams = [
        ("chan-a", KeyScope::Community),
        ("chan-b", KeyScope::Community),
    ];
    let found = stream_of_tag(&Keys, "c", streams, sealed.generation, &sealed.channel_tag);
    assert_eq!(found, Some("chan-b"));
    assert_eq!(
        stream_of_tag(&Keys, "c", streams, 1, &sealed.channel_tag),
        None
    );
    let open = |at| open_entry_body(&Keys, "c", KeyScope::Community, 2, at, &sealed.ciphertext);
    assert_eq!(open(at("chan-b", 7)).as_deref(), Some(&b"hi"[..]));
    assert_eq!(open(at("chan-b", 8)), None);
    assert_eq!(open(at("chan-a", 7)), None);
}
