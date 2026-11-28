use std::collections::VecDeque;

use super::*;

/// Valve's example A2S_INFO reply for Counter-Strike: Source, byte for byte
/// from the "Server queries" page.
const CSS_REPLY: &[u8] = &[
    0xFF, 0xFF, 0xFF, 0xFF, 0x49, 0x02, 0x67, 0x61, 0x6D, 0x65, 0x32, 0x78, 0x73, 0x2E, 0x63, 0x6F,
    0x6D, 0x20, 0x43, 0x6F, 0x75, 0x6E, 0x74, 0x65, 0x72, 0x2D, 0x53, 0x74, 0x72, 0x69, 0x6B, 0x65,
    0x20, 0x53, 0x6F, 0x75, 0x72, 0x63, 0x65, 0x20, 0x23, 0x31, 0x00, 0x64, 0x65, 0x5F, 0x64, 0x75,
    0x73, 0x74, 0x00, 0x63, 0x73, 0x74, 0x72, 0x69, 0x6B, 0x65, 0x00, 0x43, 0x6F, 0x75, 0x6E, 0x74,
    0x65, 0x72, 0x2D, 0x53, 0x74, 0x72, 0x69, 0x6B, 0x65, 0x3A, 0x20, 0x53, 0x6F, 0x75, 0x72, 0x63,
    0x65, 0x00, 0xF0, 0x00, 0x05, 0x10, 0x04, 0x64, 0x6C, 0x00, 0x00, 0x31, 0x2E, 0x30, 0x2E, 0x30,
    0x2E, 0x32, 0x32, 0x00,
];

/// Valve's challenge example: the server's reply, and the request to send.
const CHALLENGE: &[u8] = &[0xFF, 0xFF, 0xFF, 0xFF, 0x41, 0x0A, 0x08, 0x5E, 0xEA];
const CHALLENGED_REQUEST: &[u8] = &[
    0xFF, 0xFF, 0xFF, 0xFF, 0x54, 0x53, 0x6F, 0x75, 0x72, 0x63, 0x65, 0x20, 0x45, 0x6E, 0x67, 0x69,
    0x6E, 0x65, 0x20, 0x51, 0x75, 0x65, 0x72, 0x79, 0x00, 0x0A, 0x08, 0x5E, 0xEA,
];

fn css() -> A2sInfo {
    A2sInfo {
        name: "game2xs.com Counter-Strike Source #1".into(),
        map: "de_dust".into(),
        players: 5,
        max_players: 16,
    }
}

#[test]
fn a_recorded_info_reply_parses() {
    assert_eq!(parse_reply(CSS_REPLY).unwrap(), Reply::Info(css()));
    assert!(matches!(
        parse_reply(&CSS_REPLY[..20]),
        Err(QueryError::Malformed(_))
    ));
    assert!(matches!(
        parse_reply(&[0xFE, 0xFF, 0xFF, 0xFF, 0x49]),
        Err(QueryError::Split)
    ));
}

/// A server that answers each request from a script and records them.
struct Scripted {
    replies: VecDeque<Vec<u8>>,
    sent: Vec<Vec<u8>>,
}

#[async_trait]
impl QueryChannel for Scripted {
    async fn send(&mut self, datagram: &[u8]) -> std::io::Result<()> {
        self.sent.push(datagram.to_vec());
        Ok(())
    }

    async fn recv(&mut self) -> std::io::Result<Vec<u8>> {
        self.replies
            .pop_front()
            .ok_or_else(|| std::io::Error::other("no more replies"))
    }
}

#[tokio::test]
async fn the_challenge_round_repeats_the_request_with_the_challenge() {
    let mut server = Scripted {
        replies: VecDeque::from([CHALLENGE.to_vec(), CSS_REPLY.to_vec()]),
        sent: Vec::new(),
    };
    let (info, _) = query_info(&mut server).await.unwrap();
    assert_eq!(info, css());
    assert_eq!(
        server.sent,
        vec![info_request(None), CHALLENGED_REQUEST.to_vec()]
    );

    let mut twice = Scripted {
        replies: VecDeque::from([CHALLENGE.to_vec(), CHALLENGE.to_vec()]),
        sent: Vec::new(),
    };
    assert!(matches!(
        query_info(&mut twice).await,
        Err(QueryError::Malformed("a second challenge"))
    ));
}
