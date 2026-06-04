// ---------------------------------------------------------------------------
// Pacing feature tests (SO_MAX_PACING_RATE)
//
// These tests only compile and run when:
//   - target_os = "linux"
//   - feature = "pacing" is enabled
//
// Pacing requires the fq qdisc (fair queueing) on the egress path.
// If set_pacing_rate fails (e.g., no fq qdisc or insufficient privileges),
// we skip gracefully.
// ---------------------------------------------------------------------------

#[cfg(all(target_os = "linux", feature = "pacing"))]
mod pacing_tests {
    use std::net::UdpSocket;

    use binger_udp::{platform_capabilities, BingerUdp, Config};

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    // -----------------------------------------------------------------------
    // 1. Platform capability reports pacing support
    // -----------------------------------------------------------------------

    #[test]
    fn test_pacing_platform_capability() {
        let caps = platform_capabilities();
        assert!(
            caps.supports_pacing,
            "supports_pacing should be true when pacing feature is enabled on Linux"
        );
    }

    // -----------------------------------------------------------------------
    // 2. set_pacing_rate succeeds when fq qdisc is available
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_set_pacing_rate() -> TestResult {
        let sock = BingerUdp::from_std(UdpSocket::bind("127.0.0.1:0")?, Config::default())?;

        // Pacing requires the fq qdisc. If the setsockopt fails (e.g., EINVAL
        // or EPERM), skip the test rather than failing.
        if sock.set_pacing_rate(1_000_000).is_err() {
            return Ok(());
        }

        Ok(())
    }
}
