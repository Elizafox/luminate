// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Tests for the parent module.

use tokio::io::{DuplexStream, duplex};

use super::*;

/// A connected in-memory duplex pair standing in for the real transport.
///
/// [`send`] and [`receive`] are generic over `AsyncRead`/`AsyncWrite`,
/// so the concrete stream underneath them is incidental to everything
/// these tests assert: they exercise the frame format, not a socket.
/// An in-memory pair says that plainly and, unlike the `UnixStream` pair
/// this used to build, compiles on Windows too.
///
/// The buffer is far smaller than [`MAX_FRAME_LEN`] deliberately: no
/// test below ever writes more than a length prefix and a short payload
/// (the oversized cases are all rejected before anything reaches the
/// stream), so a large buffer would only obscure that.
fn transport_pair() -> (DuplexStream, DuplexStream) {
    duplex(64 * 1024)
}

#[tokio::test]
async fn send_then_receive_round_trips() {
    let (mut a, mut b) = transport_pair();
    send(&mut a, &"hello".to_owned()).await.expect("send");
    let received: String = receive(&mut b).await.expect("receive");
    assert_eq!(received, "hello");
}

#[tokio::test]
async fn oversized_frame_is_rejected_before_reading_the_payload() {
    let (mut client, mut server) = transport_pair();

    // Write an oversized length prefix directly, bypassing `send`, since
    // this is what an untrusted/malicious peer could do. No well-behaved
    // sender would ever construct a frame this large.
    client
        .write_u32(MAX_FRAME_LEN + 1)
        .await
        .expect("write length prefix");

    match receive::<String>(&mut server).await {
        Err(FramingError::FrameTooLarge { len, max }) => {
            assert_eq!(len, MAX_FRAME_LEN + 1);
            assert_eq!(max, MAX_FRAME_LEN);
        }
        other => panic!("expected FrameTooLarge, got {other:?}"),
    }
}

#[tokio::test]
async fn send_rejects_a_frame_over_the_cap_without_writing_it() {
    let (mut a, mut b) = transport_pair();

    // A byte string whose CBOR encoding is guaranteed to exceed the cap.
    let oversized = vec![0_u8; MAX_FRAME_LEN as usize + 1];
    match send(&mut a, &oversized).await {
        Err(FramingError::FrameTooLarge { len, max }) => {
            assert!(len > MAX_FRAME_LEN);
            assert_eq!(max, MAX_FRAME_LEN);
        }
        other => panic!("expected FrameTooLarge, got {other:?}"),
    }

    // Nothing should have been written to the peer: the send bailed out
    // before touching the stream.
    drop(a);
    let mut byte = [0_u8; 1];
    let read = b.read(&mut byte).await.expect("read from closed peer");
    assert_eq!(read, 0, "no bytes should have been sent");
}
