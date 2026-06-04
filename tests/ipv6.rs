// ---------------------------------------------------------------------------
// IPv6 cross-platform tests
//
// All tests attempt to bind to "[::1]:0" (IPv6 loopback). If IPv6 is not
// available on the test machine (common in some CI environments), the test
// returns Ok(()) to skip gracefully.
//
// Tests cover single send/recv, batch send/recv, address correctness,
// multi-destination sends, and connected mode.
// ---------------------------------------------------------------------------

mod ipv6_tests {
    use std::net::{Ipv6Addr, SocketAddr, UdpSocket};

    use binger_udp::{BingerUdp, Config, RecvBatch, SendBatch};

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    /// Create an IPv6 socket bound to `[::1]:0`. Returns an error if IPv6
    /// is not available on this system.
    fn bind_ipv6() -> std::io::Result<BingerUdp> {
        BingerUdp::from_std(UdpSocket::bind("[::1]:0")?, Config::default())
    }

    /// Create a (sender, receiver, `receiver_addr`) triplet using IPv6 loopback.
    /// Returns an error if IPv6 is not available.
    fn make_ipv6_pair() -> std::io::Result<(BingerUdp, BingerUdp, SocketAddr)> {
        let recv = bind_ipv6()?;
        let recv_addr = recv.local_addr()?;
        let send = bind_ipv6()?;
        Ok((send, recv, recv_addr))
    }

    // -----------------------------------------------------------------------
    // 1. Single packet send_to + recv_from over IPv6
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_ipv6_single_send_recv() -> TestResult {
        let Ok((send, recv, recv_addr)) = make_ipv6_pair() else {
            return Ok(()); // IPv6 not available, skip
        };

        let payload: &[u8] = b"hello ipv6";
        let n = send.send_to(payload, recv_addr).await?;
        assert_eq!(n, payload.len(), "send_to should return payload length");

        let mut buf = vec![0u8; 2048];
        let (n, src) = recv.recv_from(&mut buf).await?;
        assert_eq!(n, payload.len(), "recv_from should return received length");
        assert_eq!(&buf[..n], payload, "received data should match sent data");

        let send_addr = send.local_addr()?;
        assert_eq!(src, send_addr, "source should match sender IPv6 address");

        Ok(())
    }

    // -----------------------------------------------------------------------
    // 2. IPv6 batch send (N=16) + batch recv (N=16)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_ipv6_batch_send_recv() -> TestResult {
        let Ok((send, recv, recv_addr)) = make_ipv6_pair() else {
            return Ok(()); // IPv6 not available, skip
        };

        let msgs: Vec<String> = (0..16).map(|i| format!("ipv6-pkt-{i}")).collect();

        let mut sb = SendBatch::<16>::new();
        for msg in &msgs {
            sb.push(msg.as_bytes(), recv_addr)?;
        }
        let n = send.send_batch(&mut sb).await?;
        assert_eq!(n, 16, "send_batch should send all 16 IPv6 packets");

        let mut rb = RecvBatch::<16>::new(2048);
        let mut received: Vec<Vec<u8>> = Vec::new();
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
            "recv_batch should receive all 16 IPv6 packets"
        );

        for (i, data) in received.iter().enumerate() {
            let expected = format!("ipv6-pkt-{i}");
            assert_eq!(
                data.as_slice(),
                expected.as_bytes(),
                "IPv6 packet {i} data should match"
            );
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // 3. IPv6 address correctness — local_addr() returns IPv6
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_ipv6_addr_correctness() -> TestResult {
        let Ok(sock) = bind_ipv6() else {
            return Ok(()); // IPv6 not available, skip
        };

        let addr = sock.local_addr()?;
        assert!(
            addr.is_ipv6(),
            "local_addr() should return an IPv6 address, got {addr}"
        );

        let SocketAddr::V6(ipv6) = addr else {
            return Err("expected V6 address".into());
        };

        assert_eq!(ipv6.ip(), &Ipv6Addr::LOCALHOST, "should be bound to ::1");
        assert!(ipv6.port() > 0, "should have a non-zero port");

        Ok(())
    }

    // -----------------------------------------------------------------------
    // 4. IPv6 multi-destination: send to two different IPv6 receivers
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_ipv6_multi_dest() -> TestResult {
        let Ok(recv_a) = bind_ipv6() else {
            return Ok(()); // IPv6 not available, skip
        };
        let addr_a = recv_a.local_addr()?;

        let Ok(recv_b) = bind_ipv6() else {
            return Ok(()); // IPv6 not available, skip
        };
        let addr_b = recv_b.local_addr()?;

        let Ok(send) = bind_ipv6() else {
            return Ok(()); // IPv6 not available, skip
        };

        let mut sb = SendBatch::<2>::new();
        sb.push(b"to-a-v6", addr_a)?;
        sb.push(b"to-b-v6", addr_b)?;
        let n = send.send_batch(&mut sb).await?;
        assert_eq!(n, 2, "should send 2 packets over IPv6");

        let mut rb = RecvBatch::<1>::new(2048);
        let n = recv_a.recv_batch(&mut rb).await?;
        assert_eq!(n, 1, "receiver A should get 1 packet over IPv6");
        assert_eq!(rb.data(0), b"to-a-v6", "receiver A data");

        let mut rb = RecvBatch::<1>::new(2048);
        let n = recv_b.recv_batch(&mut rb).await?;
        assert_eq!(n, 1, "receiver B should get 1 packet over IPv6");
        assert_eq!(rb.data(0), b"to-b-v6", "receiver B data");

        Ok(())
    }

    // -----------------------------------------------------------------------
    // 5. IPv6 connected mode: connect() + push_connected
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_ipv6_connected_mode() -> TestResult {
        let Ok((send, recv, recv_addr)) = make_ipv6_pair() else {
            return Ok(()); // IPv6 not available, skip
        };

        let send_addr = send.local_addr()?;

        // Connect the sender.
        send.connect(recv_addr)?;

        // Send individual packet after connect.
        send.send_to(b"v6-connected-1", recv_addr).await?;

        // Send a batch using push_connected (no address per-packet).
        let mut sb = SendBatch::<3>::new();
        sb.push_connected(b"v6-batch-a")?;
        sb.push_connected(b"v6-batch-b")?;
        sb.push_connected(b"v6-batch-c")?;
        let n = send.send_batch(&mut sb).await?;
        assert_eq!(n, 3, "IPv6 connected batch should send 3 packets");

        // Receive all 4 packets (may arrive across multiple recv_batch calls).
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
            "should receive all 4 IPv6 connected-mode packets"
        );
        assert!(
            items.iter().any(|(d, _)| *d == b"v6-connected-1"),
            "should contain v6-connected-1"
        );
        assert!(
            items.iter().any(|(d, _)| *d == b"v6-batch-a"),
            "should contain v6-batch-a"
        );
        assert!(
            items.iter().any(|(d, _)| *d == b"v6-batch-b"),
            "should contain v6-batch-b"
        );
        assert!(
            items.iter().any(|(d, _)| *d == b"v6-batch-c"),
            "should contain v6-batch-c"
        );

        for (_, addr) in &items {
            assert_eq!(
                *addr, send_addr,
                "all packets should come from the connected sender"
            );
        }

        Ok(())
    }
}
