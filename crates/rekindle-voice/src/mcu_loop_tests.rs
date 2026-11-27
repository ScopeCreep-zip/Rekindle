use super::select_recipients;

#[test]
fn select_recipients_excludes_self() {
    let me = vec![0u8; 32];
    let me_hex = hex::encode(&me);
    let peer = vec![1u8; 32];
    let peer_hex = hex::encode(&peer);
    let recipients = select_recipients(&[me_hex, peer_hex], &me);
    assert_eq!(recipients, vec![peer]);
}

#[test]
fn select_recipients_includes_silent_audience() {
    let alice = vec![1u8; 32];
    let bob = vec![2u8; 32];
    let carol = vec![3u8; 32];
    let dave = vec![4u8; 32];
    let peers = vec![
        hex::encode(&alice),
        hex::encode(&bob),
        hex::encode(&carol),
        hex::encode(&dave),
    ];
    let recipients = select_recipients(&peers, &alice);
    assert_eq!(recipients.len(), 3);
    assert!(recipients.contains(&bob));
    assert!(recipients.contains(&carol));
    assert!(recipients.contains(&dave));
}

#[test]
fn select_recipients_drops_invalid_hex() {
    let me = vec![0u8; 32];
    let recipients = select_recipients(&["not-hex".to_string(), hex::encode(&me)], &me);
    assert!(recipients.is_empty(), "self filtered + invalid dropped");
}
