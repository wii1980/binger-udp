use std::net::{SocketAddr, UdpSocket};
use std::time::Duration;

use binger_udp::{BingerError, BingerUdp, Config, RecvBatch, SendBatch};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Create a (sender, receiver, `receiver_addr`) triplet bound to 127.0.0.1:0.
fn make_pair() -> std::io::Result<(BingerUdp, BingerUdp, SocketAddr)> {
    let recv = BingerUdp::from_std(UdpSocket::bind("127.0.0.1:0")?, Config::default())?;
    let recv_addr = recv.local_addr()?;
    let send = BingerUdp::from_std(UdpSocket::bind("127.0.0.1:0")?, Config::default())?;
    Ok((send, recv, recv_addr))
}

// ---------------------------------------------------------------------------
// 1. Empty payload (0-byte datagram)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_empty_payload() -> TestResult {
    let (send, recv, recv_addr) = make_pair()?;

    let mut sb = SendBatch::<1>::new();
    sb.push(b"", recv_addr)?;
    let n = send.send_batch(&mut sb).await?;
    assert_eq!(n, 1, "should send 1 empty packet");

    let mut rb = RecvBatch::<1>::new(2048);
    let n = recv.recv_batch(&mut rb).await?;
    assert_eq!(n, 1, "should receive 1 empty packet");
    assert_eq!(
        rb.data(0).len(),
        0,
        "received empty payload should be 0 bytes"
    );
    assert!(rb.data(0).is_empty(), "data(0) should be an empty slice");

    Ok(())
}

// ---------------------------------------------------------------------------
// 2. Near-maximum UDP payload (1400 bytes, safe for standard MTU)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_max_udp_payload() -> TestResult {
    let (send, recv, recv_addr) = make_pair()?;

    let payload = vec![0xABu8; 1400];
    let mut sb = SendBatch::<1>::new();
    sb.push(&payload, recv_addr)?;
    let n = send.send_batch(&mut sb).await?;
    assert_eq!(n, 1, "should send 1 large packet");

    let mut rb = RecvBatch::<1>::new(2048);
    let n = recv.recv_batch(&mut rb).await?;
    assert_eq!(n, 1, "should receive 1 large packet");
    assert_eq!(
        rb.data(0).len(),
        1400,
        "received payload should be 1400 bytes"
    );
    assert_eq!(rb.data(0), payload.as_slice(), "large payload must match");

    Ok(())
}

// ---------------------------------------------------------------------------
// 3. SendBatch::push beyond capacity returns BatchFull
// ---------------------------------------------------------------------------

#[test]
fn test_send_batch_full_returns_error() {
    let mut sb = SendBatch::<2>::new();
    let addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();

    assert!(sb.push(b"first", addr).is_ok(), "first push should succeed");
    assert!(
        sb.push(b"second", addr).is_ok(),
        "second push should succeed"
    );
    assert_eq!(sb.len(), 2, "batch len should be 2");

    let err = sb.push(b"third", addr).unwrap_err();
    assert!(
        matches!(err, BingerError::BatchFull { capacity: 2 }),
        "expected BatchFull with capacity 2, got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// 4. RecvBatch smaller than the number of arrived packets
// ---------------------------------------------------------------------------
//
// Send 8 packets, then receive with RecvBatch::<4>. Since recv_batch is
// async and retries on WouldBlock, the first call should return up to 4,
// and the second call should drain the remaining 4.

#[tokio::test]
async fn test_recv_batch_smaller_than_arrived() -> TestResult {
    let (send, recv, recv_addr) = make_pair()?;

    // Send 8 packets
    let mut sb = SendBatch::<8>::new();
    for i in 0..8 {
        let payload = [i; 4];
        sb.push(&payload, recv_addr)?;
    }
    let n = send.send_batch(&mut sb).await?;
    assert_eq!(n, 8, "should send 8 packets");

    // Receive with capacity 4 — first call gets up to 4
    let mut rb = RecvBatch::<4>::new(2048);
    let first = recv.recv_batch(&mut rb).await?;
    assert!(
        (1..=4).contains(&first),
        "first recv_batch should return 1-4, got {first}"
    );

    // Receive remaining
    let mut total = first;
    while total < 8 {
        total += recv.recv_batch(&mut rb).await?;
    }
    assert_eq!(total, 8, "should receive all 8 packets across batches");

    Ok(())
}

// ---------------------------------------------------------------------------
// 5. Zero-item send batch
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_zero_item_send_batch() -> TestResult {
    let (send, _recv, _recv_addr) = make_pair()?;

    let mut sb = SendBatch::<4>::new();
    assert!(sb.is_empty(), "batch should be empty initially");
    assert_eq!(sb.len(), 0, "batch len should be 0");

    let n = send.send_batch(&mut sb).await?;
    assert_eq!(n, 0, "send_batch with 0 items should return 0");

    Ok(())
}

// ---------------------------------------------------------------------------
// 6. Large number of small packets (10 × 32 = 320 total)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_large_number_of_small_packets() -> TestResult {
    const BATCHES: usize = 10;
    const PER_BATCH: usize = 32;
    const TOTAL: usize = BATCHES * PER_BATCH;

    let (send, recv, recv_addr) = make_pair()?;

    // Send 10 batches.  Interleave receive after each batch so the kernel
    // send buffer never fills up.
    let mut rb = RecvBatch::<PER_BATCH>::new(2048);
    let mut received = 0usize;

    for b in 0..BATCHES {
        let mut sb = SendBatch::<PER_BATCH>::new();
        let payload = [b as u8; 4];
        for _ in 0..PER_BATCH {
            sb.push(&payload, recv_addr)?;
        }
        let n = send.send_batch(&mut sb).await?;
        assert_eq!(n, PER_BATCH, "batch {b}: should send {PER_BATCH} packets");

        // Drain until we've caught up (or at least got something)
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while received < (b + 1) * PER_BATCH {
            assert!(
                tokio::time::Instant::now() < deadline,
                "timeout after batch {b}: got {received}/{TOTAL}",
            );
            let n = recv.recv_batch(&mut rb).await?;
            assert!(n > 0, "recv_batch returned 0 before all packets received");
            received += n;
        }
    }

    assert_eq!(received, TOTAL, "should receive all {TOTAL} packets");

    Ok(())
}

// ---------------------------------------------------------------------------
// 7. Mixed payload sizes in a single batch (0, 1, 100, 500, 1400 bytes)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_mixed_payload_sizes_batch() -> TestResult {
    let (send, recv, recv_addr) = make_pair()?;

    let payloads: [&[u8]; 5] = [
        b"",           // 0 bytes
        b"X",          // 1 byte
        &[b'A'; 100],  // 100 bytes
        &[b'B'; 500],  // 500 bytes
        &[b'C'; 1400], // 1400 bytes
    ];

    let mut sb = SendBatch::<5>::new();
    for p in &payloads {
        sb.push(p, recv_addr)?;
    }
    let n = send.send_batch(&mut sb).await?;
    assert_eq!(n, 5, "should send 5 packets with mixed sizes");

    let mut all_data: Vec<Vec<u8>> = Vec::new();
    let mut rb = RecvBatch::<5>::new(2048);
    while all_data.len() < 5 {
        let n = recv.recv_batch(&mut rb).await?;
        for i in 0..n {
            all_data.push(rb.data(i).to_vec());
        }
        rb.clear();
    }
    assert_eq!(all_data.len(), 5, "should receive 5 packets with mixed sizes");

    let sizes: Vec<usize> = all_data.iter().map(|d| d.len()).collect();
    assert!(sizes.contains(&0), "batch should contain 0-byte payload");
    assert!(sizes.contains(&1), "batch should contain 1-byte payload");
    assert!(
        sizes.contains(&100),
        "batch should contain 100-byte payload"
    );
    assert!(
        sizes.contains(&500),
        "batch should contain 500-byte payload"
    );
    assert!(
        sizes.contains(&1400),
        "batch should contain 1400-byte payload"
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// 8. SendBatch clear + refill between sends
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_send_batch_clear_and_reuse() -> TestResult {
    let (send, recv, recv_addr) = make_pair()?;

    let mut sb = SendBatch::<8>::new();

    // First wave: 4 packets
    for i in 0..4 {
        sb.push(format!("wave1-{i}").as_bytes(), recv_addr)?;
    }
    let n = send.send_batch(&mut sb).await?;
    assert_eq!(n, 4, "wave 1: should send 4 packets");

    sb.clear();
    assert!(sb.is_empty(), "should be empty after clear");

    // Second wave: 8 packets (fill to capacity)
    for i in 0..8 {
        sb.push(format!("wave2-{i}").as_bytes(), recv_addr)?;
    }
    assert_eq!(sb.len(), 8, "should have 8 items after refill");
    let n = send.send_batch(&mut sb).await?;
    assert_eq!(n, 8, "wave 2: should send 8 packets");

    // Receive all 12 packets
    let mut rb = RecvBatch::<4>::new(2048);
    let mut received = 0usize;
    while received < 12 {
        received += recv.recv_batch(&mut rb).await?;
    }
    assert_eq!(received, 12, "should receive all 12 packets");

    Ok(())
}

// ---------------------------------------------------------------------------
// 9. RecvBatch clear + reuse after first receive
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_recv_batch_clear_and_reuse() -> TestResult {
    let (send, recv, recv_addr) = make_pair()?;

    // First wave
    let mut sb = SendBatch::<3>::new();
    sb.push(b"first-A", recv_addr)?;
    sb.push(b"first-B", recv_addr)?;
    sb.push(b"first-C", recv_addr)?;
    send.send_batch(&mut sb).await?;

    let mut rb = RecvBatch::<4>::new(2048);
    let mut first_items: Vec<Vec<u8>> = Vec::new();
    while first_items.len() < 3 {
        let n = recv.recv_batch(&mut rb).await?;
        for i in 0..n {
            first_items.push(rb.data(i).to_vec());
        }
        rb.clear();
    }
    assert_eq!(first_items.len(), 3, "first wave should receive 3 packets");

    rb.clear();
    assert_eq!(rb.len(), 0, "recv batch should be empty after clear");
    assert!(
        rb.iter().collect::<Vec<_>>().is_empty(),
        "iter should be empty after clear"
    );

    // Second wave
    let mut sb = SendBatch::<2>::new();
    sb.push(b"second-X", recv_addr)?;
    sb.push(b"second-Y", recv_addr)?;
    send.send_batch(&mut sb).await?;

    let mut second_items: Vec<Vec<u8>> = Vec::new();
    while second_items.len() < 2 {
        let n = recv.recv_batch(&mut rb).await?;
        for i in 0..n {
            second_items.push(rb.data(i).to_vec());
        }
        rb.clear();
    }
    assert_eq!(
        second_items.len(),
        2,
        "second wave: should receive 2 packets"
    );
    assert_eq!(
        second_items[0], b"second-X",
        "second wave packet 0 mismatch"
    );
    assert_eq!(
        second_items[1], b"second-Y",
        "second wave packet 1 mismatch"
    );

    Ok(())
}
