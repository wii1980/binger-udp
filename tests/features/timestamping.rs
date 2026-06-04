// ---------------------------------------------------------------------------
// Timestamping feature tests (SO_TIMESTAMPNS + SCM_TIMESTAMPNS)
//
// These tests only compile and run when:
//   - target_os = "linux"
//   - feature = "timestamping" is enabled
//
// Kernel timestamping requires CAP_NET_ADMIN or a recent kernel.
// If enable_timestamping fails, we skip gracefully.
// ---------------------------------------------------------------------------

#[cfg(all(target_os = "linux", feature = "timestamping"))]
mod timestamping_tests {
    use std::net::UdpSocket;

    use binger_udp::{platform_capabilities, BingerUdp, Config, RecvBatch, SendBatch};

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    /// Create a (sender, receiver, `receiver_addr`) triplet.
    fn make_pair() -> std::io::Result<(BingerUdp, BingerUdp, std::net::SocketAddr)> {
        let recv = BingerUdp::from_std(UdpSocket::bind("127.0.0.1:0")?, Config::default())?;
        let recv_addr = recv.local_addr()?;
        let send = BingerUdp::from_std(UdpSocket::bind("127.0.0.1:0")?, Config::default())?;
        Ok((send, recv, recv_addr))
    }

    // -----------------------------------------------------------------------
    // 1. Platform capability reports timestamping support
    // -----------------------------------------------------------------------

    #[test]
    fn test_timestamping_platform_capability() {
        let caps = platform_capabilities();
        assert!(
            caps.supports_timestamping,
            "supports_timestamping should be true when timestamping feature is enabled on Linux"
        );
    }

    // -----------------------------------------------------------------------
    // 2. enable_timestamping(true) succeeds (or skips)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_enable_timestamping() -> TestResult {
        let recv = BingerUdp::from_std(UdpSocket::bind("127.0.0.1:0")?, Config::default())?;

        // Kernel timestamping requires socket option support. Skip on failure.
        if recv.enable_timestamping(true).is_err() {
            return Ok(());
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // 3. Receive packets with timestamps attached
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_receive_with_timestamp() -> TestResult {
        let (send, recv, recv_addr) = make_pair()?;

        // If timestamping is not available, skip.
        if recv.enable_timestamping(true).is_err() {
            return Ok(());
        }

        // Send a few packets.
        let mut sb = SendBatch::<4>::new();
        sb.push(b"ts-1", recv_addr)?;
        sb.push(b"ts-2", recv_addr)?;
        sb.push(b"ts-3", recv_addr)?;
        sb.push(b"ts-4", recv_addr)?;
        let n = send.send_batch(&mut sb).await?;
        assert_eq!(n, 4, "should send 4 packets for timestamp test");

        // Receive and verify timestamps (may arrive across multiple recv_batch calls).
        let mut rb = RecvBatch::<4>::new(2048);
        let mut total = 0usize;
        while total < 4 {
            let n = recv.recv_batch(&mut rb).await?;
            for i in 0..n {
                let ts = rb.timestamp(i);
                assert!(
                    ts.is_some(),
                    "packet {} should have a timestamp when timestamping is enabled",
                    total + i
                );

                let ts = ts.expect("timestamp should be present");
                assert!(
                    ts.tv_sec > 0 || ts.tv_nsec > 0,
                    "timestamp should be non-zero (tv_sec={}, tv_nsec={})",
                    ts.tv_sec,
                    ts.tv_nsec
                );

                let dur = ts.as_duration();
                assert!(
                    dur.as_secs() > 0 || dur.subsec_nanos() > 0,
                    "as_duration() should be non-zero"
                );
                assert!(
                    dur.as_secs() < 2_500_000_000,
                    "as_duration() should be a reasonable Unix timestamp, got {}",
                    dur.as_secs()
                );
            }
            total += n;
            rb.clear();
        }
        assert_eq!(total, 4, "should receive 4 packets with timestamps");

        Ok(())
    }
}
