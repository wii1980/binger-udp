# binger-udp

[![Rust](https://img.shields.io/badge/rust-1.75%2B-blue.svg)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/license-MIT-green.svg)](LICENSE)

**Cross-platform, batch-native UDP I/O with platform-optimal syscalls.**

Send and receive tens of thousands of small UDP packets efficiently using a unified batch API. Under the hood it picks the best syscall available: `sendmmsg`/`recvmmsg` on Linux, `sendmsg_x`/`recvmsg_x` on macOS, `WSASendMsg`/`WSARecvMsg` on Windows, with a `sendto`/`recvfrom` fallback everywhere.

## Why binger

Rust has no general-purpose, cross-platform batch UDP I/O library:

- `quinn-udp` — great but QUIC-specific, API not portable
- `nix` — has `sendmmsg` but complex API and history of zero-alloc UB
- `socket2` — `sendmmsg` PR stalled for over a year
- `fastudp` / `unix-udp-sock` — unmaintained since 2022

**binger** fills the gap: a clean batch API that automatically selects the optimal platform backend.

## Highlights

| Feature | Description |
|---------|-------------|
| **Batch-first** | `send_batch` / `recv_batch` are the primary API; single-packet is a convenience wrapper |
| **Zero-alloc hot path** | Pre-allocated buffer pool, no heap allocations in send/recv |
| **Cross-platform auto-select** | Linux→`sendmmsg`/GSO, macOS→`sendmsg_x` (dlsym), Windows→`WSASendMsg` (WSAIoctl), Generic→`sendto`/`recvfrom` |
| **Tokio native** | Poll-driven, not `try_io` glue |
| **Built-in metrics** | Optional, zero overhead, batch efficiency at a glance |
| **Adaptive batching** | Dynamically adjusts batch size based on `WouldBlock` backpressure |
| **100% safe public API** | `unsafe` confined to platform modules, Miri-testable |

## Platform backends

| Platform | Send | Recv | Extra optimizations | Status |
|----------|------|------|-------------------|--------|
| Linux (connected) | `sendmsg` w/ GSO | `recvmmsg` + GRO | `pacing`, `busy-poll` | ✅ |
| Linux (multi-dest) | `sendmmsg` | `recvmmsg` | — | ✅ |
| macOS | `sendmsg_x` | `recvmsg_x` | dlsym runtime detection | ✅ |
| Windows | `WSASendMsg` | `WSARecvMsg` | WSAIoctl runtime detection | ✅ |
| Fallback | `sendto` | `recvfrom` | — | ✅ |

## Quick start

```rust
use binger_udp::{BingerUdp, SendBatch, RecvBatch, Config};

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let socket = BingerUdp::from_std(
        std::net::UdpSocket::bind("0.0.0.0:0")?,
        Config::default(),
    )?;

    // Batch-send 32 packets in a single syscall
    let mut send = SendBatch::<32>::new();
    for i in 0..32 {
        send.push(b"hello", "192.168.1.1:8080".parse().unwrap())?;
    }
    let sent = socket.send_batch(&mut send).await?;

    // Batch-receive up to 32 packets in a single syscall
    let mut recv = RecvBatch::<32>::new(2048);
    let n = socket.recv_batch(&mut recv).await?;
    for i in 0..n {
        println!("from {}: {} bytes", recv.addr(i), recv.data(i).len());
    }

    Ok(())
}
```

## KCP integration

```rust
use binger_udp::BingerUdp;

// BingerUdp implements traits needed by KCP transports,
// enabling batch I/O for KCP sessions.
```

## Installation

```toml
[dependencies]
binger-udp = "0.1"
```

Optional features:

```toml
[dependencies]
binger-udp = { version = "0.1", features = ["metrics", "gso", "gro", "pacing", "busy-poll", "timestamping", "pktinfo"] }
```

**MSRV**: Rust 1.75+ (edition 2021)

## Use cases

| Scenario | Why binger fits |
|----------|----------------|
| KCP protocol layer | Batch flush output, `drain_output` sends multiple segments in one `sendmmsg` |
| Game servers | High packet rate, low latency, `recvmmsg` reduces kernel overhead |
| DNS servers | Batch send to multiple targets |
| Metrics collection | StatsD / Graphite batch ingest |
| RTP media streaming | High-throughput streaming with pacing and timestamps |
| Service mesh proxies | Transparent multi-connection batch forwarding |

## Performance expectations

| Scenario | Current (per-packet) | binger | syscall reduction |
|----------|---------------------|--------|-------------------|
| KCP flush 16 segments | 16 `sendto` calls | 1 `sendmmsg` | ~94% |
| Listener draining 32 packets | 32 `recvfrom` calls | 1 `recvmmsg` | ~97% |
| Game server at 10K pps | 10K syscalls/s | ~300 syscalls/s | ~97% |
| DNS 32 queries batch | 32 `sendto` calls | 1 `sendmmsg` | ~97% |

## Documentation

- [Architecture](docs/ARCHITECTURE.md)
- [API design](docs/API.md)

## Roadmap

| Version | Content | Status |
|---------|---------|--------|
| v0.1 | Linux sendmmsg/recvmmsg + fallback + Tokio + BufferPool | ✅ |
| v0.2 | macOS + Windows cross-platform | ✅ |
| v0.3 | GSO/GRO + adaptive batching + Metrics | ✅ |
| v0.4 | Pacing + busy-poll + timestamping + pktinfo | ✅ |
| v1.0 | Stable API + comprehensive docs | — |

## License

MIT
