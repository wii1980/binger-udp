#[cfg(target_os = "windows")]
mod windows {
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
    // 1. Windows platform capabilities: WSASendMsg/WSARecvMsg, max_batch_size = 32
    // ---------------------------------------------------------------------------

    #[test]
    fn test_windows_platform_capabilities() {
        let caps = platform_capabilities();

        assert!(
            caps.supports_wsa_send_msg,
            "WSASendMsg should be supported on Windows"
        );
        assert!(
            caps.supports_wsa_recv_msg,
            "WSARecvMsg should be supported on Windows"
        );
        assert!(
            !caps.supports_sendmmsg,
            "sendmmsg should NOT be supported on Windows"
        );
        assert_eq!(
            caps.max_batch_size, 32,
            "Windows max_batch_size should be 32"
        );
        assert_eq!(
            caps.backend_name,
            "WSASendMsg/WSARecvMsg (Windows)",
            "backend_name should identify the Windows backend"
        );
    }

    // ---------------------------------------------------------------------------
    // 2. Batch send + recv with N = 16 on Windows WSASendMsg/WSARecvMsg path
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn test_windows_batch_send_recv() -> TestResult {
        let (send, recv, recv_addr) = make_pair()?;

        let msgs: Vec<String> = (0..16).map(|i| format!("win-{i}")).collect();
        let mut sb = SendBatch::<16>::new();
        for msg in &msgs {
            sb.push(msg.as_bytes(), recv_addr)?;
        }
        let n = send.send_batch(&mut sb).await?;
        assert_eq!(
            n, 16,
            "send_batch should send all 16 packets on Windows backend"
        );

        let mut rb = RecvBatch::<16>::new(2048);
        let n = recv.recv_batch(&mut rb).await?;
        assert_eq!(
            n, 16,
            "recv_batch should receive all 16 packets on Windows backend"
        );

        for i in 0..16 {
            let expected = format!("win-{i}");
            assert_eq!(
                rb.data(i),
                expected.as_bytes(),
                "windows packet {i} data should match"
            );
        }

        Ok(())
    }

    // ---------------------------------------------------------------------------
    // 3. Multi-destination batch on Windows: send to 2 different receivers
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn test_windows_multi_dest_batch() -> TestResult {
        let recv_a = BingerUdp::from_std(UdpSocket::bind("127.0.0.1:0")?, Config::default())?;
        let addr_a = recv_a.local_addr()?;

        let recv_b = BingerUdp::from_std(UdpSocket::bind("127.0.0.1:0")?, Config::default())?;
        let addr_b = recv_b.local_addr()?;

        let send = BingerUdp::from_std(UdpSocket::bind("127.0.0.1:0")?, Config::default())?;

        let mut sb = SendBatch::<2>::new();
        sb.push(b"win-a", addr_a)?;
        sb.push(b"win-b", addr_b)?;
        let n = send.send_batch(&mut sb).await?;
        assert_eq!(
            n, 2,
            "Windows send_batch to 2 destinations should send both"
        );

        let mut rb_a = RecvBatch::<1>::new(2048);
        let n = recv_a.recv_batch(&mut rb_a).await?;
        assert_eq!(n, 1, "Windows receiver A should get 1 packet");
        assert_eq!(
            rb_a.data(0),
            b"win-a",
            "Windows receiver A should get correct data"
        );

        let mut rb_b = RecvBatch::<1>::new(2048);
        let n = recv_b.recv_batch(&mut rb_b).await?;
        assert_eq!(n, 1, "Windows receiver B should get 1 packet");
        assert_eq!(
            rb_b.data(0),
            b"win-b",
            "Windows receiver B should get correct data"
        );

        Ok(())
    }

    // ---------------------------------------------------------------------------
    // 4. Connected mode on Windows: connect() + push_connected
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn test_windows_connected_mode() -> TestResult {
        let (send, recv, recv_addr) = make_pair()?;
        let send_addr = send.local_addr()?;

        send.connect(recv_addr)?;

        // Single packet via connected socket
        send.try_send_to(b"w-conn-single", recv_addr)?;

        // Batch via push_connected
        let mut sb = SendBatch::<2>::new();
        sb.push_connected(b"w-conn-batch-1")?;
        sb.push_connected(b"w-conn-batch-2")?;
        let n = send.try_send_batch(&mut sb)?;
        assert_eq!(
            n, 2,
            "Windows try_send_batch via connected path should send 2"
        );

        // Receive all 3 packets
        let mut rb = RecvBatch::<3>::new(2048);
        let n = recv.recv_batch(&mut rb).await?;
        assert_eq!(n, 3, "Windows should receive all 3 connected packets");

        let items: Vec<(&[u8], SocketAddr)> = rb.iter().collect();
        assert!(
            items.iter().any(|(d, _)| *d == b"w-conn-single"),
            "Windows connected: should contain w-conn-single"
        );
        assert!(
            items.iter().any(|(d, _)| *d == b"w-conn-batch-1"),
            "Windows connected: should contain w-conn-batch-1"
        );
        assert!(
            items.iter().any(|(d, _)| *d == b"w-conn-batch-2"),
            "Windows connected: should contain w-conn-batch-2"
        );

        for (_, addr) in &items {
            assert_eq!(
                *addr, send_addr,
                "Windows all connected packets should come from sender"
            );
        }

        Ok(())
    }

    // ---------------------------------------------------------------------------
    // 5. Single packet send_to + recv_from on Windows
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn test_windows_single_packet() -> TestResult {
        let (send, recv, recv_addr) = make_pair()?;
        let send_addr = send.local_addr()?;

        let payload: &[u8] = b"windows-single";
        let n = send.send_to(payload, recv_addr).await?;
        assert_eq!(n, payload.len(), "send_to should return payload length");

        let mut buf = vec![0u8; 2048];
        let (n, src) = recv.recv_from(&mut buf).await?;
        assert_eq!(n, payload.len(), "recv_from should return received length");
        assert_eq!(
            &buf[..n],
            payload,
            "received data should match sent payload"
        );
        assert_eq!(
            src, send_addr,
            "source address should match sender on Windows"
        );

        Ok(())
    }
}
