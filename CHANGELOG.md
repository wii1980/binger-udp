# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2025-06-03

### Added

- Cross-platform batch UDP I/O with platform-optimal syscalls
- Linux: `sendmmsg`/`recvmmsg` with zero-alloc stack arrays (<=64) and Vec fallback
- Linux: GSO (Generic Segmentation Offload) via `try_send_gso` / `send_gso`
- Linux: GRO (Generic Receive Offload) with transparent cmsg parsing
- Linux: `SO_BUSY_POLL` for ultra-low latency busy-polling
- Linux: `SO_MAX_PACING_RATE` for rate-limited sending
- Linux: `SO_TIMESTAMPNS` for kernel receive timestamps
- Linux: `IP_PKTINFO`/`IPV6_RECVPKTINFO` for destination address reception
- macOS: `sendmsg_x`/`recvmsg_x` via dlsym runtime detection with fallback
- Windows: `WSASendMsg`/`WSARecvMsg` with WSAIoctl runtime detection and recvfrom fallback
- Generic fallback: loop `sendto`/`recvfrom` (zero unsafe, Miri-compatible)
- Tokio async integration via native poll (`readable`/`writable`)
- Adaptive batching with configurable WouldBlock thresholds (30%/10%)
- Built-in metrics via atomic counters (feature-gated, zero overhead when disabled)
- `BufferPool` — lock-free pre-allocated receive buffer pool
- `SendBatch<const N>` / `RecvBatch<const N>` — compile-time fixed batch containers
- `Config` builder pattern with `with_*()` chainable methods
- `PlatformCaps` runtime capability detection
- `miri-safe` feature for Miri-compatible testing
- Comprehensive integration tests (15 tests)
- Benchmark suite (send/recv/throughput with criterion)
- Architecture documentation (ARCHITECTURE.md, API.md)
- CI pipeline (Linux/macOS/Windows + MSRV + fmt + clippy)

### Changed

- Nothing yet (initial release)

### Deprecated

- Nothing yet

### Removed

- Nothing yet

### Fixed

- Nothing yet

### Security

- Nothing yet

[0.1.0]: https://github.com/wii1980/rs-binger/releases/tag/v0.1.0
