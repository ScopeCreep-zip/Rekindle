//! Asking a game server what it is running (plan E7.2 G4, decision 49):
//! Valve's A2S_INFO over UDP, for the Source and GoldSrc titles the game
//! database marks `"server_query": "a2s"`.
//!
//! The wire format is Valve's "Server queries" page
//! (<https://developer.valvesoftware.com/wiki/Server_queries>, read from
//! its 2024-06-02 Wayback copy): every packet starts with the simple header
//! `-1` (`FF FF FF FF`; `-2` marks a split reply), all integers are little
//! endian and strings are UTF-8 ended by `0x00`. The request is `0x54`
//! `"Source Engine Query\0"`; since December 2020 a server may answer
//! `S2C_CHALLENGE` (`0x41` and a 4-byte challenge) instead, and the client
//! repeats the request with that challenge appended. The answer is `0x49`:
//! protocol, name, map, folder, game, app id (short), players, max players,
//! bots, then fields this view does not read. Some older titles also send
//! the obsolete GoldSrc `0x6D` reply; it is skipped for the `0x49` one.
//!
//! A query shows our IP address to the server (threat model: metadata that
//! leaves the app), so the Host sends one only when the user asks
//! (`ServersRefresh`); this module never starts one itself. It holds the
//! codec and the round, over [`QueryChannel`], so the Host's tests drive
//! it without a network.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Which query protocol a game's servers answer (`GameEntry::server_query`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServerQueryProtocol {
    /// Valve's A2S_INFO (Source and GoldSrc engines).
    A2s,
}

const SIMPLE: [u8; 4] = [0xFF; 4];
const SPLIT: [u8; 4] = [0xFE, 0xFF, 0xFF, 0xFF];
const A2S_INFO: u8 = 0x54;
const S2C_CHALLENGE: u8 = 0x41;
const INFO_REPLY: u8 = 0x49;
const GOLDSRC_INFO_REPLY: u8 = 0x6D;
const PAYLOAD: &[u8] = b"Source Engine Query\0";
/// Datagrams read for one round: the challenge, an obsolete GoldSrc reply
/// and the answer, with one to spare.
const MAX_DATAGRAMS: usize = 4;

/// What a server says about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct A2sInfo {
    pub name: String,
    pub map: String,
    pub players: u8,
    pub max_players: u8,
}

/// One parsed reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    /// `S2C_CHALLENGE`: repeat the request with this challenge.
    Challenge([u8; 4]),
    Info(A2sInfo),
    /// The obsolete GoldSrc reply some older titles send first.
    GoldSrcInfo,
}

/// Why a query got no answer.
#[derive(Debug, thiserror::Error)]
pub enum QueryError {
    #[error("the query could not be sent: {0}")]
    Io(#[from] std::io::Error),
    #[error("the reply is split into packets")]
    Split,
    #[error("the reply is malformed: {0}")]
    Malformed(&'static str),
    #[error("the server answered without its information")]
    NoAnswer,
}

/// The A2S_INFO request, with the server's challenge when it sent one.
#[must_use]
pub fn info_request(challenge: Option<[u8; 4]>) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + 1 + PAYLOAD.len() + 4);
    out.extend_from_slice(&SIMPLE);
    out.push(A2S_INFO);
    out.extend_from_slice(PAYLOAD);
    if let Some(challenge) = challenge {
        out.extend_from_slice(&challenge);
    }
    out
}

/// Parse one datagram a server sent back.
///
/// # Errors
/// A split reply, or one that is not a challenge or an info reply.
pub fn parse_reply(datagram: &[u8]) -> Result<Reply, QueryError> {
    let mut r = Reader(datagram);
    let header = r.take(4)?;
    if header == SPLIT {
        return Err(QueryError::Split);
    }
    if header != SIMPLE {
        return Err(QueryError::Malformed("no packet header"));
    }
    match r.byte()? {
        S2C_CHALLENGE => {
            let c = r.take(4)?;
            Ok(Reply::Challenge([c[0], c[1], c[2], c[3]]))
        }
        INFO_REPLY => {
            let _protocol = r.byte()?;
            let name = r.string()?;
            let map = r.string()?;
            let _folder = r.string()?;
            let _game = r.string()?;
            let _app_id = r.take(2)?;
            let players = r.byte()?;
            let max_players = r.byte()?;
            Ok(Reply::Info(A2sInfo {
                name,
                map,
                players,
                max_players,
            }))
        }
        GOLDSRC_INFO_REPLY => Ok(Reply::GoldSrcInfo),
        _ => Err(QueryError::Malformed("not an info reply")),
    }
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], QueryError> {
        if self.0.len() < n {
            return Err(QueryError::Malformed("truncated"));
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Ok(head)
    }

    fn byte(&mut self) -> Result<u8, QueryError> {
        Ok(self.take(1)?[0])
    }

    fn string(&mut self) -> Result<String, QueryError> {
        let end = self
            .0
            .iter()
            .position(|b| *b == 0)
            .ok_or(QueryError::Malformed("unterminated string"))?;
        let text = String::from_utf8_lossy(&self.0[..end]).into_owned();
        self.0 = &self.0[end + 1..];
        Ok(text)
    }
}

/// One server's datagram channel: what the Host's port opens.
#[async_trait]
pub trait QueryChannel: Send {
    async fn send(&mut self, datagram: &[u8]) -> std::io::Result<()>;
    /// The next datagram the server sends.
    async fn recv(&mut self) -> std::io::Result<Vec<u8>>;
}

/// Opens a [`QueryChannel`] to a server (the Host's port: UDP in the app,
/// a fake in tests).
#[async_trait]
pub trait ServerQuery: Send + Sync {
    /// `server` is `host:port`.
    async fn open(&self, server: &str) -> std::io::Result<Box<dyn QueryChannel>>;
}

/// Run one A2S_INFO round: the request, the challenge round when the server
/// asks for it, the answer. Returns it with the round trip of the request
/// it answered. The caller bounds the time.
///
/// # Errors
/// The channel fails, or the server's replies are not an answer.
pub async fn query_info(channel: &mut dyn QueryChannel) -> Result<(A2sInfo, Duration), QueryError> {
    let mut sent = tokio::time::Instant::now();
    channel.send(&info_request(None)).await?;
    let mut challenged = false;
    for _ in 0..MAX_DATAGRAMS {
        let datagram = channel.recv().await?;
        match parse_reply(&datagram)? {
            Reply::Info(info) => return Ok((info, sent.elapsed())),
            Reply::GoldSrcInfo => {}
            Reply::Challenge(_) if challenged => {
                return Err(QueryError::Malformed("a second challenge"));
            }
            Reply::Challenge(challenge) => {
                challenged = true;
                sent = tokio::time::Instant::now();
                channel.send(&info_request(Some(challenge))).await?;
            }
        }
    }
    Err(QueryError::NoAnswer)
}

/// The app's port: a UDP socket per server, connected so only that
/// server's datagrams are read.
#[derive(Debug, Clone, Copy, Default)]
pub struct UdpServerQuery;

struct UdpChannel(tokio::net::UdpSocket);

#[async_trait]
impl QueryChannel for UdpChannel {
    async fn send(&mut self, datagram: &[u8]) -> std::io::Result<()> {
        self.0.send(datagram).await.map(|_| ())
    }

    async fn recv(&mut self) -> std::io::Result<Vec<u8>> {
        // Steam's packets are at most 1400 bytes; Rust's servers have been
        // seen sending more (Valve's page), so the buffer holds a datagram.
        let mut buf = vec![0u8; 65_536];
        let n = self.0.recv(&mut buf).await?;
        buf.truncate(n);
        Ok(buf)
    }
}

#[async_trait]
impl ServerQuery for UdpServerQuery {
    async fn open(&self, server: &str) -> std::io::Result<Box<dyn QueryChannel>> {
        let target = tokio::net::lookup_host(server)
            .await?
            .next()
            .ok_or_else(|| std::io::Error::other("the address resolves to nothing"))?;
        let local: std::net::SocketAddr = if target.is_ipv4() {
            (std::net::Ipv4Addr::UNSPECIFIED, 0).into()
        } else {
            (std::net::Ipv6Addr::UNSPECIFIED, 0).into()
        };
        let socket = tokio::net::UdpSocket::bind(local).await?;
        socket.connect(target).await?;
        Ok(Box::new(UdpChannel(socket)))
    }
}

#[cfg(test)]
#[path = "server_query_tests.rs"]
mod tests;
