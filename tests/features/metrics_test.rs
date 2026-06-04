// ---------------------------------------------------------------------------
// Metrics feature tests
//
// These tests only compile and run when feature = "metrics" is enabled.
// Metrics are cross-platform (no Linux requirement).
//
// We verify that counters increment correctly during actual send/recv
// operations, that snapshot() returns consistent values, and that reset()
// clears everything to zero.
// ---------------------------------------------------------------------------

#[cfg(feature = "metrics")]
mod metrics_tests {
    use std::net::UdpSocket;

    use binger_udp::{BingerUdp, Config, RecvBatch, SendBatch};

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    /// Helper: create a pair with metrics enabled on the receiver.
    fn make_pair_metrics() -> std::io::Result<(BingerUdp, BingerUdp, std::net::SocketAddr)> {
        let recv = BingerUdp::from_std(
            UdpSocket::bind("127.0.0.1:0")?,
            Config::new().with_metrics(true),
        )?;
        let recv_addr = recv.local_addr()?;
        let send = BingerUdp::from_std(UdpSocket::bind("127.0.0.1:0")?, Config::default())?;
        Ok((send, recv, recv_addr))
    }

    // -----------------------------------------------------------------------
    // 1. Metrics capability — no platform check needed, metrics are additive
    // -----------------------------------------------------------------------

    // -----------------------------------------------------------------------
    // 2. With metrics enabled, socket.metrics() returns Some
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_metrics_enabled() -> TestResult {
        let sock = BingerUdp::from_std(
            UdpSocket::bind("127.0.0.1:0")?,
            Config::new().with_metrics(true),
        )?;

        assert!(
            sock.metrics().is_some(),
            "metrics() should return Some when with_metrics(true) is set"
        );

        Ok(())
    }

    // -----------------------------------------------------------------------
    // 3. With default config, socket.metrics() returns None
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_metrics_disabled() -> TestResult {
        let sock = BingerUdp::from_std(UdpSocket::bind("127.0.0.1:0")?, Config::default())?;

        assert!(
            sock.metrics().is_none(),
            "metrics() should return None with default config"
        );

        Ok(())
    }

    // -----------------------------------------------------------------------
    // 4. Counters increment after send/recv operations
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_metrics_counters_increment() -> TestResult {
        let (send, recv, recv_addr) = make_pair_metrics()?;

        // Send a batch of 8 packets.
        let mut sb = SendBatch::<8>::new();
        for i in 0..8 {
            sb.push(format!("pkt-{i}").as_bytes(), recv_addr)?;
        }
        let n = send.send_batch(&mut sb).await?;
        assert_eq!(n, 8, "should send all 8 packets");

        // Receive them on the metrics-enabled socket.
        let mut rb = RecvBatch::<8>::new(2048);
        let n = recv.recv_batch(&mut rb).await?;
        assert_eq!(n, 8, "should receive all 8 packets");

        let m = recv
            .metrics()
            .expect("metrics should be enabled on receiver");

        // The receiver should have recorded the packets.
        assert!(
            m.packets_received() >= 8,
            "packets_received should be >= 8, got {}",
            m.packets_received()
        );
        assert!(
            m.batches_received() >= 1,
            "batches_received should be >= 1, got {}",
            m.batches_received()
        );
        assert!(
            m.recv_syscalls() >= 1,
            "recv_syscalls should be >= 1, got {}",
            m.recv_syscalls()
        );

        Ok(())
    }

    // -----------------------------------------------------------------------
    // 5. snapshot() returns values consistent with individual counters
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_metrics_snapshot() -> TestResult {
        let (send, recv, recv_addr) = make_pair_metrics()?;

        // Send and receive a batch.
        let mut sb = SendBatch::<4>::new();
        for i in 0..4 {
            sb.push(format!("s-{i}").as_bytes(), recv_addr)?;
        }
        send.send_batch(&mut sb).await?;

        let mut rb = RecvBatch::<4>::new(2048);
        let _n = recv.recv_batch(&mut rb).await?;

        let m = recv
            .metrics()
            .expect("metrics should be enabled on receiver");

        let snap = m.snapshot();

        // Snapshot values should match the individual counter reads.
        assert_eq!(
            snap.packets_received,
            m.packets_received(),
            "snapshot.packets_received should match"
        );
        assert_eq!(
            snap.batches_received,
            m.batches_received(),
            "snapshot.batches_received should match"
        );
        assert_eq!(
            snap.recv_syscalls,
            m.recv_syscalls(),
            "snapshot.recv_syscalls should match"
        );

        Ok(())
    }

    // -----------------------------------------------------------------------
    // 6. reset() clears all counters to zero
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_metrics_reset() -> TestResult {
        let (send, recv, recv_addr) = make_pair_metrics()?;

        // Perform some I/O to bump counters.
        let mut sb = SendBatch::<3>::new();
        sb.push(b"a", recv_addr)?;
        sb.push(b"b", recv_addr)?;
        sb.push(b"c", recv_addr)?;
        send.send_batch(&mut sb).await?;

        let mut rb = RecvBatch::<3>::new(2048);
        let _n = recv.recv_batch(&mut rb).await?;

        let m = recv
            .metrics()
            .expect("metrics should be enabled on receiver");

        // Confirm counters are non-zero before reset.
        assert!(
            m.packets_received() > 0,
            "counters should be non-zero before reset"
        );
        assert!(
            m.batches_received() > 0,
            "batches_received should be non-zero before reset"
        );

        m.reset();

        // After reset, everything should be zero.
        assert_eq!(m.packets_sent(), 0, "packets_sent should be 0 after reset");
        assert_eq!(
            m.packets_received(),
            0,
            "packets_received should be 0 after reset"
        );
        assert_eq!(m.batches_sent(), 0, "batches_sent should be 0 after reset");
        assert_eq!(
            m.batches_received(),
            0,
            "batches_received should be 0 after reset"
        );
        assert_eq!(
            m.send_syscalls(),
            0,
            "send_syscalls should be 0 after reset"
        );
        assert_eq!(
            m.recv_syscalls(),
            0,
            "recv_syscalls should be 0 after reset"
        );

        Ok(())
    }
}
