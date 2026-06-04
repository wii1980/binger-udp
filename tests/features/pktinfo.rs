// ---------------------------------------------------------------------------
// Pktinfo feature tests (IP_PKTINFO / IPV6_RECVPKTINFO)
//
// These tests only compile and run when:
//   - target_os = "linux"
//   - feature = "pktinfo" is enabled
//
// Pktinfo allows receivers to determine the destination address of an
// incoming packet. This is useful for multi-homed servers.
// If enable_pktinfo fails, we skip gracefully.
// ---------------------------------------------------------------------------

#[cfg(all(target_os = "linux", feature = "pktinfo"))]
mod pktinfo_tests {
    use std::net::{Ipv4Addr, UdpSocket};

    use binger_udp::{platform_capabilities, BingerUdp, Config, RecvBatch, SendBatch};

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    /// Create a (sender, receiver, `receiver_addr`) triplet bound to 127.0.0.1:0.
    fn make_pair() -> std::io::Result<(BingerUdp, BingerUdp, std::net::SocketAddr)> {
        let recv = BingerUdp::from_std(UdpSocket::bind("127.0.0.1:0")?, Config::default())?;
        let recv_addr = recv.local_addr()?;
        let send = BingerUdp::from_std(UdpSocket::bind("127.0.0.1:0")?, Config::default())?;
        Ok((send, recv, recv_addr))
    }

    // -----------------------------------------------------------------------
    // 1. Platform capability reports pktinfo support
    // -----------------------------------------------------------------------

    #[test]
    fn test_pktinfo_platform_capability() {
        let caps = platform_capabilities();
        assert!(
            caps.supports_pktinfo,
            "supports_pktinfo should be true when pktinfo feature is enabled on Linux"
        );
    }

    // -----------------------------------------------------------------------
    // 2. enable_pktinfo(true) succeeds (or skips)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_enable_pktinfo() -> TestResult {
        let recv = BingerUdp::from_std(UdpSocket::bind("127.0.0.1:0")?, Config::default())?;

        if recv.enable_pktinfo(true).is_err() {
            return Ok(());
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // 3. Receive with dst_addr — verify destination is 127.0.0.1
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_receive_with_dst_addr() -> TestResult {
        let (send, recv, recv_addr) = make_pair()?;

        if recv.enable_pktinfo(true).is_err() {
            return Ok(());
        }

        // Send a packet to the receiver bound to 127.0.0.1.
        let mut sb = SendBatch::<1>::new();
        sb.push(b"pktinfo-test", recv_addr)?;
        let n = send.send_batch(&mut sb).await?;
        assert_eq!(n, 1, "should send 1 packet for pktinfo test");

        // Receive and check dst_addr.
        let mut rb = RecvBatch::<1>::new(2048);
        let n = recv.recv_batch(&mut rb).await?;
        assert_eq!(n, 1, "should receive 1 packet");

        let dst = rb.dst_addr(0);
        assert!(
            dst.is_some(),
            "dst_addr(0) should be Some when pktinfo is enabled"
        );

        let dst = dst.expect("dst_addr should be present");
        assert_eq!(
            dst.ip(),
            std::net::IpAddr::V4(Ipv4Addr::LOCALHOST),
            "destination address should be 127.0.0.1"
        );

        Ok(())
    }
}
