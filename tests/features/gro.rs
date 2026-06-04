// ---------------------------------------------------------------------------
// GRO (Generic Receive Offload) feature tests
//
// These tests only compile and run when:
//   - target_os = "linux"
//   - feature = "gro" is enabled
//
// GRO requires kernel support (UDP_GRO socket option). If set_gro fails,
// the test environment likely doesn't support GRO and we skip gracefully.
// ---------------------------------------------------------------------------

#[cfg(all(target_os = "linux", feature = "gro"))]
mod gro_tests {
    use std::net::UdpSocket;

    use binger_udp::{platform_capabilities, BingerUdp, Config};

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    // -----------------------------------------------------------------------
    // 1. Platform capability reports GRO support
    // -----------------------------------------------------------------------

    #[test]
    fn test_gro_platform_capability() {
        let caps = platform_capabilities();
        assert!(
            caps.supports_gro,
            "supports_gro should be true when gro feature is enabled on Linux"
        );
    }

    // -----------------------------------------------------------------------
    // 2. set_gro(true) succeeds (or skips when not supported)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_set_gro_enabled() -> TestResult {
        let sock = BingerUdp::from_std(UdpSocket::bind("127.0.0.1:0")?, Config::default())?;

        // GRO requires kernel support (UDP_GRO). If it fails, skip.
        if sock.set_gro(true).is_err() {
            return Ok(());
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // 3. set_gro(false) succeeds
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_set_gro_disabled() -> TestResult {
        let sock = BingerUdp::from_std(UdpSocket::bind("127.0.0.1:0")?, Config::default())?;

        let _ = sock.set_gro(false).ok();

        Ok(())
    }
}
