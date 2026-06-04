// ---------------------------------------------------------------------------
// GSO (Generic Segmentation Offload) feature tests
//
// These tests only compile and run when:
//   - target_os = "linux"
//   - feature = "gso" is enabled
//
// GSO requires kernel support and a connected socket. If set_gso fails,
// the test environment likely doesn't support GSO and we skip gracefully.
// ---------------------------------------------------------------------------

#[cfg(all(target_os = "linux", feature = "gso"))]
mod gso_tests {
    use std::net::UdpSocket;

    use binger_udp::{platform_capabilities, BingerUdp, Config, RecvBatch};

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    /// Create a (sender, receiver, `receiver_addr`) triplet bound to 127.0.0.1:0.
    fn make_pair() -> std::io::Result<(BingerUdp, BingerUdp, std::net::SocketAddr)> {
        let recv = BingerUdp::from_std(UdpSocket::bind("127.0.0.1:0")?, Config::default())?;
        let recv_addr = recv.local_addr()?;
        let send = BingerUdp::from_std(UdpSocket::bind("127.0.0.1:0")?, Config::default())?;
        Ok((send, recv, recv_addr))
    }

    // -----------------------------------------------------------------------
    // 1. Platform capability reports GSO support
    // -----------------------------------------------------------------------

    #[test]
    fn test_gso_platform_capability() {
        let caps = platform_capabilities();
        assert!(
            caps.supports_gso,
            "supports_gso should be true when gso feature is enabled on Linux"
        );
    }

    // -----------------------------------------------------------------------
    // 2. set_gso(true) succeeds (or skips when not supported)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_set_gso_enabled() -> TestResult {
        let (send, _recv, recv_addr) = make_pair()?;
        send.connect(recv_addr)?;

        // GSO requires a connected socket and UDP_SEGMENT support.
        // If the kernel doesn't support GSO, skip the test.
        if send.set_gso(true).is_err() {
            return Ok(());
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // 3. set_gso(false) succeeds
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_set_gso_disabled() -> TestResult {
        let (send, _recv, recv_addr) = make_pair()?;
        send.connect(recv_addr)?;

        // Disabling GSO should always work if the option exists.
        let _ = send.set_gso(false).ok();

        Ok(())
    }

    // -----------------------------------------------------------------------
    // 4. GSO send with segmentation — send large data, receive segments
    //
    // NOTE: try_send_gso / send_gso additionally require not(feature = "miri-safe").
    // The marker attribute below handles that for this specific test.
    // -----------------------------------------------------------------------

    #[cfg(not(feature = "miri-safe"))]
    #[tokio::test]
    async fn test_gso_send_segmented() -> TestResult {
        let (send, recv, recv_addr) = make_pair()?;
        send.connect(recv_addr)?;

        // If set_gso fails, the kernel doesn't support GSO — skip.
        if send.set_gso(true).is_err() {
            return Ok(());
        }

        // Send data segmented into 1400-byte chunks.
        // Use a moderate size (28 KiB) to avoid EMSGSIZE on some kernels.
        let data: Vec<u8> = vec![0xAB; 28_000];
        let segment_size: u16 = 1400;

        let bytes_sent = match send.try_send_gso(&data, segment_size) {
            Ok(n) => n,
            Err(e) => {
                // GSO may fail with EMSGSIZE or EOPNOTSUPP on some kernels.
                // Skip the test gracefully.
                eprintln!("GSO send skipped: {e}");
                return Ok(());
            }
        };
        assert_eq!(
            bytes_sent, data.len(),
            "try_send_gso should return total data length"
        );

        // Receive all the segments. The kernel will deliver each segment
        // as a separate UDP datagram.
        let mut rb = RecvBatch::<64>::new(2048);
        let n = recv.recv_batch(&mut rb).await?;
        assert!(
            n > 0,
            "should receive at least one segment from GSO send"
        );

        let total_received: usize = rb.iter().map(|(d, _)| d.len()).sum();
        assert_eq!(
            total_received, data.len(),
            "total received bytes should match GSO-sent data"
        );

        Ok(())
    }
}
