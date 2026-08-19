// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Tests for length-prefixed CBOR framing.

use tokio::io;

use super::*;

#[tokio::test]
async fn send_then_receive_round_trips() {
    let (mut a, mut b) = io::duplex(64 * 1024);
    send(&mut a, &"hello".to_owned()).await.expect("send");
    let received: String = receive(&mut b).await.expect("receive");
    assert_eq!(received, "hello");
}

#[tokio::test]
async fn oversized_frame_is_rejected_before_reading_the_payload() {
    let (mut client, mut server) = io::duplex(64 * 1024);

    // Bypass `send` to model an untrusted length prefix.
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
    let (mut a, mut b) = io::duplex(64 * 1024);

    // A byte string whose CBOR encoding is guaranteed to exceed the cap.
    let oversized = vec![0_u8; MAX_FRAME_LEN as usize + 1];
    match send(&mut a, &oversized).await {
        Err(FramingError::FrameTooLarge { len, max }) => {
            assert!(len > MAX_FRAME_LEN);
            assert_eq!(max, MAX_FRAME_LEN);
        }
        other => panic!("expected FrameTooLarge, got {other:?}"),
    }

    drop(a);
    let mut byte = [0_u8; 1];
    let read = b.read(&mut byte).await.expect("read from closed peer");
    assert_eq!(read, 0, "no bytes should have been sent");
}

#[tokio::test]
async fn frame_timeout_does_not_expire_while_connection_is_idle() {
    let (mut client, mut server) = io::duplex(64 * 1024);
    let receive = tokio::spawn(async move {
        receive_with_frame_timeout::<String>(&mut server, Duration::from_millis(20)).await
    });

    time::sleep(Duration::from_millis(60)).await;
    send(&mut client, &"after idle".to_owned())
        .await
        .expect("send after idle period");

    assert_eq!(
        receive.await.expect("receive task").expect("receive"),
        "after idle"
    );
}

#[tokio::test]
async fn frame_timeout_expires_after_a_partial_prefix() {
    let (mut client, mut server) = io::duplex(64 * 1024);
    client.write_u8(0).await.expect("write first prefix byte");

    let result = receive_with_frame_timeout::<String>(&mut server, Duration::from_millis(20)).await;
    assert!(matches!(result, Err(FramingError::Timeout(_))));
}
