use super::*;

#[test]
fn wire_round_trip() {
    let channel = KeyScope::Channel(ChannelId([0xab; 16]));
    assert_eq!(KeyScope::from_wire(None), Some(KeyScope::Community));
    assert_eq!(KeyScope::from_wire(Some("")), Some(KeyScope::Community));
    assert_eq!(
        KeyScope::from_wire(channel.wire_channel().as_deref()),
        Some(channel)
    );
    assert_eq!(
        KeyScope::from_wire(Some(&channel.wire_channel_str())),
        Some(channel)
    );
    assert_eq!(KeyScope::Community.wire_channel(), None);
    assert_eq!(KeyScope::Community.wire_channel_str(), "");
}

/// No other string names a scope: the old `"__community__"` slot and
/// non-hex channel names are rejected, not mapped somewhere.
#[test]
fn non_scopes_are_rejected() {
    for bad in ["__community__", "general", "abcd", &"0".repeat(33)] {
        assert_eq!(KeyScope::from_wire(Some(bad)), None, "{bad}");
    }
}
