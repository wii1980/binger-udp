#[cfg(target_os = "linux")]
mod linux {
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
    // 1. Linux platform capabilities: sendmmsg/recvmmsg, max_batch_size = 1024
    // ---------------------------------------------------------------------------

    #[test]
    fn test_linux_platform_capabilities() {
        let caps = platform_capabilities();

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
        assert_eq!(
            caps.backend_name, "sendmmsg/recvmmsg (Linux)",
            "backend_name should identify the Linux backend"
        );
    }

    // ---------------------------------------------------------------------------
    // 2. Batch send + recv with N = 32 on Linux sendmmsg/recvmmsg path
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn test_linux_batch_send_recv() -> TestResult {
        let (send, recv, recv_addr) = make_pair()?;

        let msgs: Vec<String> = (0..32).map(|i| format!("linux-{i}")).collect();
        let mut sb = SendBatch::<32>::new();
        for msg in &msgs {
            sb.push(msg.as_bytes(), recv_addr)?;
        }
        let n = send.send_batch(&mut sb).await?;
        assert_eq!(
            n, 32,
            "send_batch should send all 32 packets on Linux backend"
        );

        let mut rb = RecvBatch::<32>::new(2048);
        let mut received: Vec<Vec<u8>> = Vec::new();
        while received.len() < 32 {
            let n = recv.recv_batch(&mut rb).await?;
            for i in 0..n {
                received.push(rb.data(i).to_vec());
            }
            rb.clear();
        }
        assert_eq!(
            received.len(),
            32,
            "recv_batch should receive all 32 packets on Linux backend"
        );

        for (i, data) in received.iter().enumerate() {
            let expected = format!("linux-{i}");
            assert_eq!(
                data.as_slice(),
                expected.as_bytes(),
                "linux packet {i} data should match"
            );
        }

        Ok(())
    }

    // ---------------------------------------------------------------------------
    // 3. sendmmsg multi-destination: send to 3 different receivers
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn test_linux_sendmmsg_multi_dest() -> TestResult {
        let recv_a = BingerUdp::from_std(UdpSocket::bind("127.0.0.1:0")?, Config::default())?;
        let addr_a = recv_a.local_addr()?;

        let recv_b = BingerUdp::from_std(UdpSocket::bind("127.0.0.1:0")?, Config::default())?;
        let addr_b = recv_b.local_addr()?;

        let recv_c = BingerUdp::from_std(UdpSocket::bind("127.0.0.1:0")?, Config::default())?;
        let addr_c = recv_c.local_addr()?;

        let send = BingerUdp::from_std(UdpSocket::bind("127.0.0.1:0")?, Config::default())?;

        let mut sb = SendBatch::<3>::new();
        sb.push(b"alpha", addr_a)?;
        sb.push(b"beta", addr_b)?;
        sb.push(b"gamma", addr_c)?;
        let n = send.send_batch(&mut sb).await?;
        assert_eq!(
            n, 3,
            "send_batch to 3 destinations should send all 3 packets"
        );

        // Each receiver should only get its own packet
        let mut rb_a = RecvBatch::<1>::new(2048);
        let n = recv_a.recv_batch(&mut rb_a).await?;
        assert_eq!(n, 1, "receiver A should get 1 packet");
        assert_eq!(rb_a.data(0), b"alpha", "receiver A should get 'alpha'");

        let mut rb_b = RecvBatch::<1>::new(2048);
        let n = recv_b.recv_batch(&mut rb_b).await?;
        assert_eq!(n, 1, "receiver B should get 1 packet");
        assert_eq!(rb_b.data(0), b"beta", "receiver B should get 'beta'");

        let mut rb_c = RecvBatch::<1>::new(2048);
        let n = recv_c.recv_batch(&mut rb_c).await?;
        assert_eq!(n, 1, "receiver C should get 1 packet");
        assert_eq!(rb_c.data(0), b"gamma", "receiver C should get 'gamma'");

        Ok(())
    }

    // ---------------------------------------------------------------------------
    // 4. recvmmsg large batch: send N = 64, receive all at once
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn test_linux_recvmmsg_large_batch() -> TestResult {
        let (send, recv, recv_addr) = make_pair()?;

        let mut expected_payloads: Vec<Vec<u8>> = Vec::with_capacity(64);
        let mut sb = SendBatch::<64>::new();
        for i in 0..64 {
            let payload = vec![i as u8; 16];
            sb.push(&payload, recv_addr)?;
            expected_payloads.push(payload);
        }
        let n = send.send_batch(&mut sb).await?;
        assert_eq!(n, 64, "send_batch should send all 64 packets via sendmmsg");

        let mut rb = RecvBatch::<64>::new(2048);
        let mut received_payloads: Vec<Vec<u8>> = Vec::new();
        while received_payloads.len() < 64 {
            let n = recv.recv_batch(&mut rb).await?;
            for i in 0..n {
                received_payloads.push(rb.data(i).to_vec());
            }
            rb.clear();
        }
        assert_eq!(
            received_payloads.len(),
            64,
            "recv_batch should receive all 64 packets via recvmmsg"
        );

        // Check that every sent payload was received (UDP may reorder)
        for expected in &expected_payloads {
            assert!(
                received_payloads.iter().any(|r| r == expected.as_slice()),
                "large batch should contain payload {expected:?}",
            );
        }

        Ok(())
    }

    // ---------------------------------------------------------------------------
    // 5. Connected mode: connect() + push_connected (sendmsg path on Linux)
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn test_linux_connected_sendmsg_path() -> TestResult {
        let (send, recv, recv_addr) = make_pair()?;
        let send_addr = send.local_addr()?;

        // Connect the sender so it uses the sendmsg (connected) path
        send.connect(recv_addr)?;

        // Send individual packets after connect
        let n = send.send_to(b"connected-1", recv_addr).await?;
        assert_eq!(n, 11, "send_to via connected socket should succeed");

        // Send a batch using push_connected (no address per-packet)
        let mut sb = SendBatch::<3>::new();
        sb.push_connected(b"batch-a")?;
        sb.push_connected(b"batch-b")?;
        sb.push_connected(b"batch-c")?;
        let n = send.send_batch(&mut sb).await?;
        assert_eq!(n, 3, "send_batch via connected path should send all 3");

        // Receive all 4 packets (may arrive across multiple recv_batch calls)
        let mut items: Vec<(Vec<u8>, SocketAddr)> = Vec::new();
        let mut rb = RecvBatch::<4>::new(2048);
        while items.len() < 4 {
            let n = recv.recv_batch(&mut rb).await?;
            for i in 0..n {
                items.push((rb.data(i).to_vec(), rb.addr(i)));
            }
            rb.clear();
        }
        assert_eq!(
            items.len(),
            4,
            "should receive all 4 packets on connected path"
        );

        // Verify data and source address
        assert!(
            items.iter().any(|(d, _)| d.as_slice() == b"connected-1"),
            "should contain connected-1"
        );
        assert!(
            items.iter().any(|(d, _)| d.as_slice() == b"batch-a"),
            "should contain batch-a"
        );
        assert!(
            items.iter().any(|(d, _)| d.as_slice() == b"batch-b"),
            "should contain batch-b"
        );
        assert!(
            items.iter().any(|(d, _)| d.as_slice() == b"batch-c"),
            "should contain batch-c"
        );

        for (_, addr) in &items {
            assert_eq!(
                *addr, send_addr,
                "all packets should come from connected sender"
            );
        }

        Ok(())
    }
}
