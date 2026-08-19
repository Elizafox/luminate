// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::*;

#[test]
fn stream_keys_preserve_target_scope() {
    assert_ne!(
        stream_key(&TargetId::device("desk")),
        stream_key(&TargetId::surface("desk", "front"))
    );
    assert_eq!(
        stream_key(&TargetId::element("desk", "front", "key")),
        "desk/front/key"
    );
}

#[test]
fn segment_sizes_include_the_header_and_detect_overflow() {
    assert_eq!(segment_bytes(ShmPixelFormat::Mono8, 0), Some(32));
    assert_eq!(segment_bytes(ShmPixelFormat::Rgb8, 10), Some(62));
    assert_eq!(segment_bytes(ShmPixelFormat::Rgbw8, 10), Some(72));
    assert_eq!(segment_bytes(ShmPixelFormat::Rgbx8, 10), Some(72));
    assert_eq!(segment_bytes(ShmPixelFormat::Rgb8, u32::MAX), None);
}

#[test]
fn rejection_logging_is_rate_limited_until_the_interval_elapses() {
    let target = TargetId::device("desk");
    let mut last = Instant::now();
    log_rejected_frame(&mut last, &target, &"first");
    let first = last;
    log_rejected_frame(&mut last, &target, &"second");
    assert_eq!(last, first);
    last = last
        .checked_sub(REJECTED_FRAME_LOG_INTERVAL)
        .expect("test instant should have enough range");
    log_rejected_frame(&mut last, &target, &"third");
    assert!(last > first);
}
