# udp-binger

**大胃王** — 跨平台批量 UDP I/O，专吃大量小包。

[![Rust](https://img.shields.io/badge/rust-1.75%2B-blue.svg)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/license-MIT-green.svg)](LICENSE)

## 一句话描述

> Rust 生态中第一个「跨平台自适应批量 UDP I/O + 零分配 + Tokio async」的通用库。

## 为什么需要 binger

当前 Rust 中没有通用、高性能、跨平台的批量 UDP I/O 库：

- `quinn-udp` — 好，但是为 QUIC 量身定制，API 不通用
- `nix` — 有 sendmmsg，但 API 复杂，有零分配 UB 历史
- `socket2` — sendmmsg PR 挂了一年没合
- `fastudp` / `unix-udp-sock` — 2022 年后无维护

**binger** 填补了这条空白：给你一个干净的批量 API，自动选平台最优的系统调用。

## 核心卖点

| 特性 | 说明 |
|------|------|
| 🍔 **批量优先** | `send_batch` / `recv_batch` 是主 API，单包是便利包装 |
| 🏎️ **零分配热路径** | 预分配 buffer pool，send/recv 不碰堆 |
| 🌍 **跨平台自动选择** | Linux→sendmmsg/GSO, macOS→sendmsg_x (dlsym), Windows→WSASendMsg (WSAIoctl), 通用→sendto/recvfrom |
| ⚡ **Tokio 原生** | poll 驱动，不是 try_io 胶水 |
| 📊 **内置指标** | 可选，零开销，batch 效率一目了然 |
| 🔄 **自适应批量** | 根据背压自动调 batch_size |
| 🛡️ **100% safe 公共 API** | unsafe 仅在平台模块，Miri 可测 |

## 平台后端

| 平台 | 发送 | 接收 | 额外优化 | 状态 |
|------|------|------|---------|------|
| Linux (connected) | `sendmsg` w/ GSO | `recvmmsg` + GRO | `pacing`, `busy-poll` | ✅ 已实现 |
| Linux (multi-dest) | `sendmmsg` | `recvmmsg` | — | ✅ 已实现 |
| macOS | `sendmsg_x` | `recvmsg_x` | dlsym 运行时检测 | ✅ 已实现 |
| Windows | `WSASendMsg` | `WSARecvMsg` | WSAIoctl 运行时检测 | ✅ 已实现 |
| Fallback | `sendto` | `recvfrom` | — | ✅ 已实现 |

## 快速开始

```rust
use udp_binger::{BingerUdp, SendBatch, RecvBatch, Config};

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let socket = BingerUdp::from_std(
        std::net::UdpSocket::bind("0.0.0.0:0")?,
        Config::default(),
    )?;

    // 批量发送 — 32 个包，1 次 syscall
    let mut send = SendBatch::<32>::new();
    for i in 0..32 {
        send.push(b"hello", "192.168.1.1:8080".parse().unwrap())?;
    }
    let sent = socket.send_batch(&mut send).await?;

    // 批量接收 — 1 次 syscall
    let mut recv = RecvBatch::<32>::new(2048);
    let n = socket.recv_batch(&mut recv).await?;
    for i in 0..n {
        println!("from {}: {} bytes", recv.addr(i), recv.data(i).len());
    }

    Ok(())
}
```

## 与 KCP 集成

```rust
use udp_binger::BingerUdp;
use kcp2_std::transport::KcpTransport;

// binger 可以轻松实现 KcpTransport trait，
// 让 KCP 享受批量 I/O 的性能提升
```

## 安装

```toml
[dependencies]
udp-binger = "0.1"
```

可选 features：

```toml
[dependencies]
udp-binger = { version = "0.1", features = ["metrics", "gso", "gro", "pacing", "busy-poll", "timestamping", "pktinfo"] }
```

**MSRV**: Rust 1.75+ (edition 2021)

## 应用场景

| 场景 | 为什么适合 binger |
|------|------------------|
| KCP 协议层 | 批量 flush 输出，drain_output 一次 sendmmsg 处理多个 segment |
| 游戏服务器 | 高包率、低延迟，recvmmsg 减少网络栈开销 |
| DNS 服务器 | 多目标查询批量发送 |
| Metrics 采集 | StatsD / Graphite 批量 ingest |
| RTP 媒体流 | 需要 pacing + timestamp 的高吞吐流媒体 |
| 服务网格代理 | 多连接透明代理，批量转发 |

## 性能预期

| 场景 | 当前 (逐个 syscall) | binger | syscall 减少 |
|------|-------------------|--------|-------------|
| KCP flush 产 16 包 | 16 次 sendto | 1 次 sendmmsg | ~94% |
| Listener 排队 32 包 | 32 次 recvfrom | 1 次 recvmmsg | ~97% |
| 游戏 10K pps | 10K syscall/s | ~300 syscall/s | ~97% |
| DNS 32 查询批量 | 32 次 sendto | 1 次 sendmmsg | ~97% |

## 文档

- [架构设计](docs/ARCHITECTURE.md)
- [API 设计](docs/API.md)

## 版本路线

| 版本 | 内容 | 状态 |
|------|------|------|
| v0.1 | Linux sendmmsg/recvmmsg + fallback + Tokio + BufferPool | ✅ |
| v0.2 | macOS + Windows 全平台 | ✅ |
| v0.3 | GSO/GRO + 自适应批量 + Metrics | ✅ |
| v0.4 | Pacing + busy-poll + timestamping + pktinfo | ✅ |
| v1.0 | 稳定 API + 全面文档 | — |

## License

MIT
