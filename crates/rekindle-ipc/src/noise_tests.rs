//! Noise IK handshake and transport tests.

use super::super::noise_keys::generate_keypair;
use super::*;

const BUS: &str = "/run/user/1000/rekindle/daemon.sock";

fn peer(local_uid: u32, remote_uid: u32) -> HandshakePeer<'static> {
    HandshakePeer {
        local_uid,
        remote_uid,
        socket_path: Path::new(BUS),
    }
}

#[test]
fn prologue_v2_is_canonical_and_pid_free() {
    let a = build_prologue(1000, 1001, Path::new(BUS));
    assert_eq!(a, build_prologue(1001, 1000, Path::new(BUS)));
    let mut expected = b"REKINDLE-IPC-v3".to_vec();
    expected.extend_from_slice(&1000u32.to_be_bytes());
    expected.extend_from_slice(&1001u32.to_be_bytes());
    expected.extend_from_slice(BUS.as_bytes());
    assert_eq!(a, expected);
}

/// Two transports over an in-memory duplex: (client, server, client
/// stream halves, server stream halves).
async fn pair(
    client_peer: HandshakePeer<'_>,
    server_peer: HandshakePeer<'_>,
) -> (
    Result<NoiseTransport>,
    Result<NoiseTransport>,
    (
        tokio::io::ReadHalf<tokio::io::DuplexStream>,
        tokio::io::WriteHalf<tokio::io::DuplexStream>,
    ),
    (
        tokio::io::ReadHalf<tokio::io::DuplexStream>,
        tokio::io::WriteHalf<tokio::io::DuplexStream>,
    ),
) {
    let server_kp = generate_keypair().unwrap();
    let client_kp = generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
    let (cs, ss) = tokio::io::duplex(1024 * 1024);
    let (mut cr, mut cw) = tokio::io::split(cs);
    let (mut sr, mut sw) = tokio::io::split(ss);
    let (client, server) = tokio::join!(
        client_handshake(
            &mut cr,
            &mut cw,
            &server_pub,
            client_kp.as_inner(),
            client_peer
        ),
        server_handshake(&mut sr, &mut sw, server_kp.as_inner(), server_peer),
    );
    (client, server, (cr, cw), (sr, sw))
}

/// Read one frame the way the I/O loops do: whole messages in, frame out.
async fn read_frame<R: AsyncRead + Unpin>(t: &mut NoiseTransport, r: &mut R) -> Vec<u8> {
    loop {
        let message = read_noise_message(r).await.unwrap();
        if let Some(frame) = t.read_message(&message).unwrap() {
            return frame;
        }
    }
}

#[tokio::test]
async fn handshake_and_transport_roundtrip() {
    let (client, server, (mut cr, mut cw), (mut sr, mut sw)) =
        pair(peer(1000, 1000), peer(1000, 1000)).await;
    let (mut client, mut server) = (client.unwrap(), server.unwrap());

    client
        .write_frame(&mut cw, b"hello encrypted world")
        .await
        .unwrap();
    assert_eq!(
        read_frame(&mut server, &mut sr).await,
        b"hello encrypted world"
    );
    server.write_frame(&mut sw, b"acknowledged").await.unwrap();
    assert_eq!(read_frame(&mut client, &mut cr).await, b"acknowledged");
}

#[tokio::test]
async fn large_and_empty_frames() {
    let (client, server, (_cr, mut cw), (mut sr, _sw)) =
        pair(peer(1000, 1000), peer(1000, 1000)).await;
    let (mut client, mut server) = (client.unwrap(), server.unwrap());

    let large = vec![0xABu8; 200 * 1024];
    let messages = client.encrypt_frame(&large).unwrap();
    assert_eq!(messages.len(), 4);
    for message in &messages {
        write_noise_message(&mut cw, message).await.unwrap();
    }
    cw.flush().await.unwrap();
    assert_eq!(read_frame(&mut server, &mut sr).await, large);

    client.write_frame(&mut cw, b"").await.unwrap();
    assert!(read_frame(&mut server, &mut sr).await.is_empty());
}

/// Both ends run the I/O loops' shape — a reader task feeding a
/// `select!` that also writes — and exchange 10 000 frames each way
/// concurrently, varied in size so frames span several messages. Every
/// frame arrives intact and in order: no select ever loses part of a
/// message or desynchronises a nonce (WS12.1).
#[tokio::test]
async fn duplex_interleave_soak() {
    const FRAMES: usize = 10_000;
    let (client, server, (cr, cw), (sr, sw)) = pair(peer(1000, 1000), peer(1000, 1000)).await;

    fn frame(i: usize) -> Vec<u8> {
        let len = if i.is_multiple_of(97) {
            70_000
        } else {
            i % 300
        };
        let mut f = u32::try_from(i).unwrap().to_be_bytes().to_vec();
        f.resize(4 + len, u8::try_from(i % 251).unwrap());
        f
    }

    async fn run<R, W>(mut t: NoiseTransport, reader: R, mut writer: W) -> usize
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin,
    {
        let (mut inbound, reader_task) = crate::framing::spawn_reader(reader);
        let (mut sent, mut received) = (0usize, 0usize);
        while received < FRAMES || sent < FRAMES {
            tokio::select! {
                message = inbound.recv(), if received < FRAMES => {
                    if let Some(f) = t.read_message(&message.unwrap()).unwrap() {
                        assert_eq!(f, frame(received), "frame {received}");
                        received += 1;
                    }
                }
                () = std::future::ready(()), if sent < FRAMES => {
                    t.write_frame(&mut writer, &frame(sent)).await.unwrap();
                    sent += 1;
                }
            }
        }
        reader_task.abort();
        received
    }

    let (a, b) = tokio::join!(run(client.unwrap(), cr, cw), run(server.unwrap(), sr, sw));
    assert_eq!((a, b), (FRAMES, FRAMES));
}

/// A PID-less peer handshakes: the prologue carries no PID.
#[tokio::test]
async fn handshake_needs_no_pid() {
    let (client, server, _, _) = pair(peer(1000, 1000), peer(1000, 1000)).await;
    assert!(client.is_ok() && server.is_ok());
}

/// [RC-4] A different UID or bus path on either end fails the handshake.
#[tokio::test]
async fn prologue_mismatch_fails_handshake() {
    let (client, server, _, _) = pair(peer(1000, 1000), peer(1000, 9999)).await;
    assert!(client.is_err() || server.is_err(), "uid mismatch");

    let other = HandshakePeer {
        socket_path: Path::new("/tmp/other.sock"),
        ..peer(1000, 1000)
    };
    let (client, server, _, _) = pair(peer(1000, 1000), other).await;
    assert!(client.is_err() || server.is_err(), "path mismatch");
}

/// Ends built from different protocol sources refuse each other with a
/// typed error naming both builds, on both sides.
#[tokio::test]
async fn a_build_mismatch_is_a_typed_error_on_both_ends() {
    let server_kp = generate_keypair().unwrap();
    let client_kp = generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
    let (cs, ss) = tokio::io::duplex(1024 * 1024);
    let (mut cr, mut cw) = tokio::io::split(cs);
    let (mut sr, mut sw) = tokio::io::split(ss);
    let (client, server) = tokio::join!(
        client_handshake_as(
            &mut cr,
            &mut cw,
            &server_pub,
            client_kp.as_inner(),
            peer(1000, 1000),
            "client-build",
        ),
        server_handshake_as(
            &mut sr,
            &mut sw,
            server_kp.as_inner(),
            peer(1000, 1000),
            "host-build"
        ),
    );
    for result in [client, server] {
        match result {
            Err(IpcError::BuildMismatch { client, host }) => {
                assert_eq!(
                    (client.as_str(), host.as_str()),
                    ("client-build", "host-build")
                );
            }
            Err(other) => panic!("expected BuildMismatch, got {other}"),
            Ok(_) => panic!("expected BuildMismatch, got a transport"),
        }
    }
}

/// The total length is authenticated: a frame whose messages run past
/// it is refused, and a length over the cap is refused before any
/// allocation for the frame.
#[test]
fn assembler_enforces_the_authenticated_length() {
    let mut assembler = FrameAssembler::default();
    let mut first = 3u32.to_be_bytes().to_vec();
    first.extend_from_slice(b"ab");
    assert_eq!(assembler.push(&first).unwrap(), None);
    assert!(matches!(
        assembler.push(b"cd"),
        Err(IpcError::FrameLengthMismatch { .. })
    ));

    let mut assembler = FrameAssembler::default();
    assert!(matches!(
        assembler.push(&(MAX_APP_FRAME + 1).to_be_bytes()),
        Err(IpcError::FrameTooLarge { .. })
    ));
    assert!(matches!(
        FrameAssembler::default().push(b"ab"),
        Err(IpcError::InvalidChunkHeader { got: 2 })
    ));
}
