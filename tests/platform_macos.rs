#[cfg(target_os = "macos")]
mod macos {
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
    // 1. macOS platform capabilities: sendmsg_x/recvmsg_x, max_batch_size = 32
    // ---------------------------------------------------------------------------

    #[test]
    fn test_macos_platform_capabilities() {
        let caps = platform_capabilities();

        assert!(
            caps.supports_sendmsg_x,
            "sendmsg_x should be supported on macOS"
        );
        assert!(
            caps.supports_recvmsg_x,
            "recvmsg_x should be supported on macOS"
        );
        assert!(
            !caps.supports_sendmmsg,
            "sendmmsg should NOT be supported on macOS"
        );
        assert_eq!(caps.max_batch_size, 32, "macOS max_batch_size should be 32");
        assert_eq!(
            caps.backend_name, "sendmsg_x/recvmsg_x (macOS)",
            "backend_name should identify the macOS backend"
        );
    }

    // ---------------------------------------------------------------------------
    // 2. Batch send + recv with N = 16 on macOS sendmsg_x/recvmsg_x path
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn test_macos_batch_send_recv() -> TestResult {
        let (send, recv, recv_addr) = make_pair()?;

        let msgs: Vec<String> = (0..16).map(|i| format!("macos-{i}")).collect();
        let mut sb = SendBatch::<16>::new();
        for msg in &msgs {
            sb.push(msg.as_bytes(), recv_addr)?;
        }
        let n = send.send_batch(&mut sb).await?;
        assert_eq!(
            n, 16,
            "send_batch should send all 16 packets on macOS backend"
        );

        let mut rb = RecvBatch::<16>::new(2048);
        let mut received = Vec::new();
        while received.len() < 16 {
            let n = recv.recv_batch(&mut rb).await?;
            for i in 0..n {
                received.push(rb.data(i).to_vec());
            }
            rb.clear();
        }
        assert_eq!(
            received.len(),
            16,
            "recv_batch should receive all 16 packets on macOS backend"
        );

        for (i, data) in received.iter().enumerate() {
            let expected = format!("macos-{i}");
            assert_eq!(
                data.as_slice(),
                expected.as_bytes(),
                "macos packet {i} data should match"
            );
        }

        Ok(())
    }

    // ---------------------------------------------------------------------------
    // 3. Multi-destination batch on macOS: send to 2 different receivers
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn test_macos_multi_dest_batch() -> TestResult {
        let recv_a = BingerUdp::from_std(UdpSocket::bind("127.0.0.1:0")?, Config::default())?;
        let addr_a = recv_a.local_addr()?;

        let recv_b = BingerUdp::from_std(UdpSocket::bind("127.0.0.1:0")?, Config::default())?;
        let addr_b = recv_b.local_addr()?;

        let send = BingerUdp::from_std(UdpSocket::bind("127.0.0.1:0")?, Config::default())?;

        let mut sb = SendBatch::<2>::new();
        sb.push(b"macos-a", addr_a)?;
        sb.push(b"macos-b", addr_b)?;
        let n = send.send_batch(&mut sb).await?;
        assert_eq!(n, 2, "macOS send_batch to 2 destinations should send both");

        let mut rb_a = RecvBatch::<1>::new(2048);
        let n = recv_a.recv_batch(&mut rb_a).await?;
        assert_eq!(n, 1, "macOS receiver A should get 1 packet");
        assert_eq!(
            rb_a.data(0),
            b"macos-a",
            "macOS receiver A should get correct data"
        );

        let mut rb_b = RecvBatch::<1>::new(2048);
        let n = recv_b.recv_batch(&mut rb_b).await?;
        assert_eq!(n, 1, "macOS receiver B should get 1 packet");
        assert_eq!(
            rb_b.data(0),
            b"macos-b",
            "macOS receiver B should get correct data"
        );

        Ok(())
    }

    // ---------------------------------------------------------------------------
    // 4. Connected mode on macOS: connect() + push_connected
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn test_macos_connected_mode() -> TestResult {
        let (send, recv, recv_addr) = make_pair()?;
        let send_addr = send.local_addr()?;

        send.connect(recv_addr)?;

        // Single packet via connected socket
        send.try_send_to(b"conn-single", recv_addr)?;

        // Batch via push_connected
        let mut sb = SendBatch::<2>::new();
        sb.push_connected(b"conn-batch-1")?;
        sb.push_connected(b"conn-batch-2")?;
        let n = send.try_send_batch(&mut sb)?;
        assert_eq!(
            n, 2,
            "macOS try_send_batch via connected path should send 2"
        );

        // Receive all 3 packets (may arrive across multiple recv_batch calls)
        let mut items: Vec<(Vec<u8>, SocketAddr)> = Vec::new();
        let mut rb = RecvBatch::<3>::new(2048);
        while items.len() < 3 {
            let n = recv.recv_batch(&mut rb).await?;
            for i in 0..n {
                items.push((rb.data(i).to_vec(), rb.addr(i)));
            }
            rb.clear();
        }
        assert_eq!(
            items.len(),
            3,
            "macOS should receive all 3 connected packets"
        );
        assert!(
            items.iter().any(|(d, _)| *d == b"conn-single"),
            "macOS connected: should contain conn-single"
        );
        assert!(
            items.iter().any(|(d, _)| *d == b"conn-batch-1"),
            "macOS connected: should contain conn-batch-1"
        );
        assert!(
            items.iter().any(|(d, _)| *d == b"conn-batch-2"),
            "macOS connected: should contain conn-batch-2"
        );

        for (_, addr) in &items {
            assert_eq!(
                *addr, send_addr,
                "macOS all connected packets should come from sender"
            );
        }

        Ok(())
    }
}
