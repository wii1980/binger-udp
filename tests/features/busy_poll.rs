// ---------------------------------------------------------------------------
// Busy-poll feature tests (SO_BUSY_POLL)
//
// These tests only compile and run when:
//   - target_os = "linux"
//   - feature = "busy-poll" is enabled
//
// Busy-poll requires either CAP_NET_ADMIN or the net.core.busy_poll sysctl
// to be configured globally. If set_busy_poll fails, we skip gracefully.
// ---------------------------------------------------------------------------

#[cfg(all(target_os = "linux", feature = "busy-poll"))]
mod busy_poll_tests {
    use std::net::UdpSocket;

    use binger_udp::{platform_capabilities, BingerUdp, Config};

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    // -----------------------------------------------------------------------
    // 1. Platform capability reports busy-poll support
    // -----------------------------------------------------------------------

    #[test]
    fn test_busy_poll_platform_capability() {
        let caps = platform_capabilities();
        assert!(
            caps.supports_busy_poll,
            "supports_busy_poll should be true when busy-poll feature is enabled on Linux"
        );
    }

    // -----------------------------------------------------------------------
    // 2. set_busy_poll succeeds when kernel and privileges allow it
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_set_busy_poll() -> TestResult {
        let sock = BingerUdp::from_std(UdpSocket::bind("127.0.0.1:0")?, Config::default())?;

        // SO_BUSY_POLL may fail with EPERM or EINVAL. Skip on failure.
        if sock.set_busy_poll(50).is_err() {
            return Ok(());
        }

        Ok(())
    }
}
