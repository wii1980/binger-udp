use std::net::{SocketAddr, UdpSocket};

use binger_udp::{platform_capabilities, BingerUdp, Config, RecvBatch, SendBatch};

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

/// Create a (sender, receiver, `receiver_addr`) triplet bound to 127.0.0.1:0.
fn make_pair() -> std::io::Result<(BingerUdp, BingerUdp, SocketAddr)> {
    let recv = BingerUdp::from_std(UdpSocket::bind("127.0.0.1:0")?, Config::default())?;
    let recv_addr = recv.local_addr()?;
    let send = BingerUdp::from_std(UdpSocket::bind("127.0.0.1:0")?, Config::default())?;
    Ok((send, recv, recv_addr))
}

type TestResult = Result<(), Box<dyn std::error::Error>>;

// ---------------------------------------------------------------------------
// 1. Single packet send_to + recv_from
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_single_send_recv() -> TestResult {
    let (send, recv, recv_addr) = make_pair()?;
    let payload: &[u8] = b"hello binger";

    let n = send.send_to(payload, recv_addr).await?;
    assert_eq!(n, payload.len(), "send_to should return payload length");

    let mut buf = vec![0u8; 2048];
    let (n, src) = recv.recv_from(&mut buf).await?;
    assert_eq!(n, payload.len(), "recv_from should return received length");
    assert_eq!(&buf[..n], payload, "received data should match sent data");

    let send_addr = send.local_addr()?;
    assert_eq!(src, send_addr, "source address should match sender");

    Ok(())
}

// ---------------------------------------------------------------------------
// 2. Batch send (SendBatch<N>) + batch recv (RecvBatch<N>) with N = 1
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_batch_send_recv_n1() -> TestResult {
    let (send, recv, recv_addr) = make_pair()?;

    let mut sb = SendBatch::<1>::new();
    sb.push(b"n1 batch", recv_addr)?;
    let n = send.send_batch(&mut sb).await?;
    assert_eq!(n, 1, "send_batch should return 1 for single item");

    let mut rb = RecvBatch::<1>::new(2048);
    let n = recv.recv_batch(&mut rb).await?;
    assert_eq!(n, 1, "recv_batch should return 1 for single packet");
    assert_eq!(rb.data(0), b"n1 batch", "data should match");

    Ok(())
}

// ---------------------------------------------------------------------------
// 3. Batch send + recv with N = 32 (many packets at once)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_batch_send_recv_n32() -> TestResult {
    let (send, recv, recv_addr) = make_pair()?;

    let msgs: Vec<String> = (0..32).map(|i| format!("packet-{i}")).collect();
    let mut sb = SendBatch::<32>::new();
    for msg in &msgs {
        sb.push(msg.as_bytes(), recv_addr)?;
    }
    let n = send.send_batch(&mut sb).await?;
    assert_eq!(n, 32, "should send all 32 packets");

    let mut rb = RecvBatch::<32>::new(2048);
    let mut all_data: Vec<Vec<u8>> = Vec::new();
    while all_data.len() < 32 {
        let n = recv.recv_batch(&mut rb).await?;
        for i in 0..n {
            all_data.push(rb.data(i).to_vec());
        }
        rb.clear();
    }
    assert_eq!(all_data.len(), 32, "should receive all 32 packets");

    for (i, expected) in msgs.iter().enumerate() {
        assert_eq!(
            all_data[i],
            expected.as_bytes(),
            "packet {i} data should match"
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// 4. Connected mode: connect() then try_send_to vs push_connected
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_connected_mode() -> TestResult {
    let (send, recv, recv_addr) = make_pair()?;

    send.connect(recv_addr)?;

    send.try_send_to(b"explicit-1", recv_addr)?;
    send.try_send_to(b"explicit-2", recv_addr)?;

    let mut sb = SendBatch::<2>::new();
    sb.push_connected(b"connected-1")?;
    sb.push_connected(b"connected-2")?;
    let n = send.try_send_batch(&mut sb)?;
    assert_eq!(n, 2, "should send 2 connected packets");

    let mut rb = RecvBatch::<4>::new(2048);
    let mut total_received = 0usize;
    let mut all_data: Vec<Vec<u8>> = Vec::new();
    while total_received < 4 {
        let n = recv.recv_batch(&mut rb).await?;
        for i in 0..n {
            all_data.push(rb.data(i).to_vec());
        }
        total_received += n;
        rb.clear();
    }
    assert_eq!(total_received, 4, "should receive all 4 packets");
    assert!(
        all_data.iter().any(|d| d == b"explicit-1"),
        "must contain explicit-1"
    );
    assert!(
        all_data.iter().any(|d| d == b"explicit-2"),
        "must contain explicit-2"
    );
    assert!(
        all_data.iter().any(|d| d == b"connected-1"),
        "must contain connected-1"
    );
    assert!(
        all_data.iter().any(|d| d == b"connected-2"),
        "must contain connected-2"
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// 5. send_batch returns correct count of sent packets
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_send_batch_returns_count() -> TestResult {
    let (send, recv, recv_addr) = make_pair()?;

    for count in [0usize, 1, 3, 7] {
        let mut sb = SendBatch::<8>::new();
        for _ in 0..count {
            sb.push(b"x", recv_addr)?;
        }
        let n = send.send_batch(&mut sb).await?;
        assert_eq!(
            n, count,
            "send_batch with {count} items should return {count}"
        );

        if count > 0 {
            let mut rb = RecvBatch::<8>::new(2048);
            let mut got = 0usize;
            while got < count {
                got += recv.recv_batch(&mut rb).await?;
                rb.clear();
            }
            assert_eq!(got, count, "drain should match item count");
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// 6. recv_batch returns correct count and data/addr for each packet
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_recv_batch_count_data_addr() -> TestResult {
    let (send, recv, recv_addr) = make_pair()?;
    let _send_addr = send.local_addr()?;

    let payloads: [&[u8]; 5] = [b"one", b"two", b"three", b"four", b"five"];
    let mut sb = SendBatch::<5>::new();
    for &p in &payloads {
        sb.push(p, recv_addr)?;
    }
    let n = send.send_batch(&mut sb).await?;
    assert_eq!(n, 5, "should send 5 packets");

    let mut rb = RecvBatch::<5>::new(2048);
    let mut all_data: Vec<Vec<u8>> = Vec::new();
    while all_data.len() < 5 {
        let n = recv.recv_batch(&mut rb).await?;
        for i in 0..n {
            all_data.push(rb.data(i).to_vec());
        }
        rb.clear();
    }
    assert_eq!(all_data.len(), 5, "should receive 5 packets");

    for (i, payload) in payloads.iter().enumerate() {
        assert_eq!(
            all_data[i], *payload,
            "packet {i} data should match sent payload"
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// 7. Data integrity: verify sent bytes == received bytes for each packet
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_data_integrity() -> TestResult {
    let (send, recv, recv_addr) = make_pair()?;

    let payloads: Vec<Vec<u8>> = vec![
        b"".to_vec(),
        b"a".to_vec(),
        b"hello world".to_vec(),
        vec![0u8; 64],
        vec![0xFF; 128],
        (0..255).map(|i| i as u8).collect(),
    ];

    let mut sb = SendBatch::<8>::new();
    for p in &payloads {
        sb.push(p.as_slice(), recv_addr)?;
    }
    let n = send.send_batch(&mut sb).await?;
    assert_eq!(n, payloads.len(), "should send all payloads");

    let mut rb = RecvBatch::<8>::new(2048);
    let mut all_data: Vec<Vec<u8>> = Vec::new();
    while all_data.len() < payloads.len() {
        let n = recv.recv_batch(&mut rb).await?;
        for i in 0..n {
            all_data.push(rb.data(i).to_vec());
        }
        rb.clear();
    }
    assert_eq!(
        all_data.len(),
        payloads.len(),
        "should receive all payloads"
    );

    for (i, (payload, received)) in payloads.iter().zip(all_data.iter()).enumerate() {
        assert_eq!(
            received.as_slice(),
            payload.as_slice(),
            "payload {i} integrity: sent {} bytes, got {} bytes",
            payload.len(),
            received.len(),
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// 8. Large batch (N = 64) with different payload sizes (10 B, 500 B, 1400 B)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_large_batch_different_sizes() -> TestResult {
    let (send, recv, recv_addr) = make_pair()?;

    let small = vec![b's'; 10];
    let medium = vec![b'm'; 500];
    let large = vec![b'L'; 1400];

    let mut sb = SendBatch::<64>::new();
    for _ in 0..20 {
        sb.push(&small, recv_addr)?;
    }
    for _ in 0..20 {
        sb.push(&medium, recv_addr)?;
    }
    for _ in 0..24 {
        sb.push(&large, recv_addr)?;
    }
    let n = send.send_batch(&mut sb).await?;
    assert_eq!(n, 64, "should send all 64 packets");

    let mut rb = RecvBatch::<64>::new(2048);
    let mut all_sizes: Vec<usize> = Vec::new();
    while all_sizes.len() < 64 {
        let n = recv.recv_batch(&mut rb).await?;
        for i in 0..n {
            all_sizes.push(rb.data(i).len());
        }
        rb.clear();
    }
    assert_eq!(all_sizes.len(), 64, "should receive all 64 packets");

    let small_count = all_sizes.iter().filter(|&&s| s == 10).count();
    let med_count = all_sizes.iter().filter(|&&s| s == 500).count();
    let large_count = all_sizes.iter().filter(|&&s| s == 1400).count();

    assert_eq!(small_count, 20, "should have 20 small payloads (10 B)");
    assert_eq!(med_count, 20, "should have 20 medium payloads (500 B)");
    assert_eq!(large_count, 24, "should have 24 large payloads (1400 B)");

    Ok(())
}

// ---------------------------------------------------------------------------
// 9. Multiple destinations in one batch (send to different ports)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_multiple_destinations() -> TestResult {
    let recv_a = BingerUdp::from_std(UdpSocket::bind("127.0.0.1:0")?, Config::default())?;
    let addr_a = recv_a.local_addr()?;

    let recv_b = BingerUdp::from_std(UdpSocket::bind("127.0.0.1:0")?, Config::default())?;
    let addr_b = recv_b.local_addr()?;

    let send = BingerUdp::from_std(UdpSocket::bind("127.0.0.1:0")?, Config::default())?;

    let mut sb = SendBatch::<2>::new();
    sb.push(b"to-a", addr_a)?;
    sb.push(b"to-b", addr_b)?;
    let n = send.send_batch(&mut sb).await?;
    assert_eq!(n, 2, "should send 2 packets to different destinations");

    let mut rb = RecvBatch::<1>::new(2048);
    let n = recv_a.recv_batch(&mut rb).await?;
    assert_eq!(n, 1, "receiver A should get 1 packet");
    assert_eq!(rb.data(0), b"to-a", "receiver A should get correct data");

    let mut rb = RecvBatch::<1>::new(2048);
    let n = recv_b.recv_batch(&mut rb).await?;
    assert_eq!(n, 1, "receiver B should get 1 packet");
    assert_eq!(rb.data(0), b"to-b", "receiver B should get correct data");

    Ok(())
}

// ---------------------------------------------------------------------------
// 10. Config builder: Config::new().with_batch_size(16)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_config_builder() -> TestResult {
    let config = Config::new().with_batch_size(16);
    let recv = BingerUdp::from_std(UdpSocket::bind("127.0.0.1:0")?, config)?;
    let recv_addr = recv.local_addr()?;

    let send = BingerUdp::from_std(
        UdpSocket::bind("127.0.0.1:0")?,
        Config::new().with_batch_size(8).with_send_buf_size(65536),
    )?;

    let mut sb = SendBatch::<1>::new();
    sb.push(b"config-builder", recv_addr)?;
    send.send_batch(&mut sb).await?;

    let mut rb = RecvBatch::<1>::new(2048);
    recv.recv_batch(&mut rb).await?;
    assert_eq!(
        rb.data(0),
        b"config-builder",
        "data should survive config builder path"
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// 11. platform_capabilities() returns valid caps on Linux
// ---------------------------------------------------------------------------

#[test]
fn test_platform_capabilities() {
    let caps = platform_capabilities();

    assert!(caps.max_batch_size > 0, "max_batch_size must be > 0");
    assert!(
        !caps.backend_name.is_empty(),
        "backend_name must not be empty"
    );

    #[cfg(target_os = "linux")]
    {
        assert!(
            caps.supports_sendmmsg,
            "sendmmsg should be supported on Linux"
        );
        assert!(
            caps.supports_recvmmsg,
            "recvmmsg should be supported on Linux"
        );
        assert_eq!(
            caps.max_batch_size, 1024,
            "Linux max_batch_size should be 1024"
        );
        assert_eq!(caps.backend_name, "sendmmsg/recvmmsg (Linux)");
    }
}

// ---------------------------------------------------------------------------
// 12. local_addr() returns the bound address
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_local_addr() -> TestResult {
    let sock = BingerUdp::from_std(UdpSocket::bind("127.0.0.1:0")?, Config::default())?;
    let addr = sock.local_addr()?;

    assert_eq!(
        addr.ip(),
        std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
        "should bind to 127.0.0.1"
    );
    assert!(addr.port() > 0, "should have a non-zero OS-assigned port");

    Ok(())
}

// ---------------------------------------------------------------------------
// 13. ttl() / set_ttl() round-trip
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_ttl_roundtrip() -> TestResult {
    let sock = BingerUdp::from_std(UdpSocket::bind("127.0.0.1:0")?, Config::default())?;

    let default = sock.ttl()?;
    assert!(default > 0, "default TTL should be > 0, got {default}");

    sock.set_ttl(128)?;
    assert_eq!(sock.ttl()?, 128, "TTL should round-trip as 128");

    sock.set_ttl(64)?;
    assert_eq!(sock.ttl()?, 64, "TTL should round-trip as 64");

    Ok(())
}

// ---------------------------------------------------------------------------
// 14. clear() on SendBatch / RecvBatch
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_clear_send_batch() -> TestResult {
    let (send, recv, recv_addr) = make_pair()?;

    let mut sb = SendBatch::<5>::new();

    assert_eq!(sb.len(), 0);
    assert!(sb.is_empty());

    sb.push(b"first", recv_addr)?;
    sb.push(b"second", recv_addr)?;
    assert_eq!(sb.len(), 2);
    assert!(!sb.is_empty());

    sb.clear();
    assert_eq!(sb.len(), 0, "len should be 0 after clear");
    assert!(sb.is_empty(), "should be empty after clear");

    sb.push(b"after-clear", recv_addr)?;
    assert_eq!(sb.len(), 1);
    send.send_batch(&mut sb).await?;

    let mut rb = RecvBatch::<1>::new(2048);
    recv.recv_batch(&mut rb).await?;
    assert_eq!(
        rb.data(0),
        b"after-clear",
        "after-clear data should arrive intact"
    );

    Ok(())
}

#[tokio::test]
async fn test_clear_recv_batch() -> TestResult {
    let (send, recv, recv_addr) = make_pair()?;

    let mut sb = SendBatch::<2>::new();
    sb.push(b"first-A", recv_addr)?;
    sb.push(b"first-B", recv_addr)?;
    send.send_batch(&mut sb).await?;

    let mut rb = RecvBatch::<4>::new(2048);
    let mut first_count = 0usize;
    while first_count < 2 {
        first_count += recv.recv_batch(&mut rb).await?;
        rb.clear();
    }
    assert_eq!(first_count, 2, "should receive first wave");

    rb.clear();
    assert_eq!(rb.len(), 0, "len should be 0 after clear");

    let mut sb = SendBatch::<1>::new();
    sb.push(b"second", recv_addr)?;
    send.send_batch(&mut sb).await?;

    let n = recv.recv_batch(&mut rb).await?;
    assert_eq!(n, 1, "should receive second wave");
    assert_eq!(rb.data(0), b"second", "second-wave data should be correct");

    Ok(())
}

// ---------------------------------------------------------------------------
// 15. iter() on RecvBatch returns correct (data, addr) pairs
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_recv_batch_iter() -> TestResult {
    let (send, recv, recv_addr) = make_pair()?;
    let _send_addr = send.local_addr()?;

    let mut sb = SendBatch::<3>::new();
    sb.push(b"alpha", recv_addr)?;
    sb.push(b"beta", recv_addr)?;
    sb.push(b"gamma", recv_addr)?;
    send.send_batch(&mut sb).await?;

    let mut rb = RecvBatch::<3>::new(2048);
    let mut all_data: Vec<Vec<u8>> = Vec::new();
    while all_data.len() < 3 {
        let n = recv.recv_batch(&mut rb).await?;
        for i in 0..n {
            all_data.push(rb.data(i).to_vec());
        }
        rb.clear();
    }
    assert_eq!(all_data.len(), 3, "should receive 3 packets");

    assert_eq!(all_data[0], b"alpha", "first iter item");
    assert_eq!(all_data[1], b"beta", "second iter item");
    assert_eq!(all_data[2], b"gamma", "third iter item");

    Ok(())
}
