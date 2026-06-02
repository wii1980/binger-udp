# udp-binger 架构设计

## 1. 定位与差异化

### 1.1 生态空白

当前 Rust 生态中，UDP 批量 I/O 的格局存在断层：

```
太底层                    QUIC 专用                  通用 █ 空白
nix / libc               quinn-udp                   
(手动 mmsghdr)            (ECN/GSO, 非通用)            ← binger 填补
                         
已死/半死                 大厂自用                    系统抽象
fastudp / mmsg-rs        Solana streamer              socket2  
(2022 起无维护)            (不可独立复用)               (PR#583 挂一年)
```

**binger 的回答**：我要最快的平台原生 UDP 批量 I/O，不管底层怎么做到的，给我一个干净的批量 API。

### 1.2 与竞品的核心差异

| 维度 | quinn-udp | nix | socket2 | **binger** |
|------|-----------|-----|---------|------------|
| API 设计理念 | QUIC 专用 | 原始 syscall | 通用 socket | **批量优先** |
| 零分配热路径 | 部分 | ⚠️ UB 历史 | N/A | **保证** |
| 跨平台自动选择 | ✅ (GSO) | ❌ Linux only | N/A | **5 平台** |
| Async 集成 | try_io 胶水 | ❌ sync | ❌ sync | **poll 原生** |
| 内置可观测性 | ❌ | ❌ | ❌ | **✅ feature-gated** |
| 自适应批量 | ❌ | ❌ | ❌ | **✅ 背压感知** |
| 能力检测 | ❌ | ❌ | ❌ | **✅ 编译+运行时** |

### 1.3 设计哲学

1. **批量是第一公民**：`send_batch` / `recv_batch` 是主 API，单包 `send_to` / `recv_from` 只是便利包装
2. **平台是编译期问题**：用户不该关心 `#[cfg(target_os = "linux")]` — crate 替你选了最优 syscall
3. **分配是瓶颈，不是便利**：热路径零分配，buffer pool 预分配，`IoSlice` 指向预分配内存
4. **async 是 poll 问题，不是 try_io**：与 Tokio reactor 深度集成，通过 `Registration` 注册 waker
5. **可观测性不是事后总结**：metrics 是 feature flag，关闭时零开销，开启时给你完整画面

---

## 2. 平台策略

### 2.1 五层优先级

```
Layer 0: 编译期能力检测 (cfg gate)
    ├── Linux → Layer 1
    ├── macOS → Layer 3
    ├── Windows → Layer 4
    └── 其他 → Layer 5 (fallback)

Layer 1: Linux 优化路径
    ├── connected + GSO → sendmsg w/ GSO (单目标最快)
    ├── multi-dest        → sendmmsg (多目标批量)
    └── recv              → recvmmsg + GRO

Layer 2: 运行时降级 (Linux)
    ├── sendmmsg → ENOSYS → 逐个 sendto
    └── recvmmsg → ENOSYS → 逐个 recvfrom

Layer 3: macOS
    ├── send → sendmsg_x (private API, 已验证)
    └── recv → recvmsg_x (private API, 已验证)

Layer 4: Windows
    ├── send → WSASendMsg (overlapped I/O)
    └── recv → WSARecvMsg (overlapped I/O)

Layer 5: 通用 fallback
    └── 逐个 sendto/recvfrom 循环
```

### 2.2 GSO vs sendmmsg 选择逻辑

```
if connected + kernel >= 4.18 + feature "gso":
    使用 GSO (一次 syscall 发一个超大 datagram，内核分片)
elif multi-destination:
    使用 sendmmsg (一次 syscall 发多个独立 datagram)
else:
    使用逐个 sendto
```

**GSO 优势**：单 socket 单目标场景（KCP 客户端、QUIC 连接），GSO 比 sendmmsg 更快，因为内核层面的 datagram 分片比用户态组装多个 `mmsghdr` 更高效。

**sendmmsg 优势**：多目标场景（KCP 服务端、DNS 服务器），可以一次向不同 peer 发送不同数据。

### 2.3 macOS 的两种路径

macOS 没有 `sendmmsg`/`recvmmsg`，但有未文档化的 `sendmsg_x`/`recvmsg_x`（quinn-udp 已验证可用）：

- `fast-apple-datapath` 路径：使用 `sendmsg_x`/`recvmsg_x`，类似批量
- 标准路径：使用 `sendmsg`/`recvmsg`（支持 scatter/gather，但单包）

binger 默认使用前者（已验证安全），通过 `miri-safe` feature 回退到标准路径。

### 2.4 Windows 的 Overlapped I/O 策略

Windows 不支持 `sendmmsg`，但支持 overlapped I/O 批量。策略：

1. 批量填充 `WSABUF[]` 数组
2. 单次 `WSASendMsg`（connected）或 `WSASendTo`（multi-dest）
3. 通过 IOCP 或 `WSAEventSelect` 等待完成
4. 与 Tokio Windows IOCP reactor 集成

---

## 3. 组件架构

```
┌─────────────────────────────────────────────────────────────────┐
│                        Public API                                │
│  BingerUdp  │  SendBatch<N>  │  RecvBatch<N>  │  Config       │
├─────────────────────────────────────────────────────────────────┤
│                      Async Layer (tokio)                         │
│  poll_send_batch()  │  poll_recv_batch()  │  Registration       │
├─────────────────────────────────────────────────────────────────┤
│                    Platform Dispatch                              │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌────────────────┐  │
│  │  linux   │  │  macos   │  │ windows  │  │   fallback     │  │
│  │ sendmmsg │  │sendmsg_x │  │WSASendMsg│  │  loop sendto   │  │
│  │ recvmmsg │  │recvmsg_x │  │WSARecvMsg│  │ loop recvfrom  │  │
│  │ GSO/GRO  │  │          │  │          │  │                │  │
│  └──────────┘  └──────────┘  └──────────┘  └────────────────┘  │
├─────────────────────────────────────────────────────────────────┤
│                    Buffer Pool (bufs.rs)                         │
│  ArrayQueue<BytesMut>  │  IoSlice/IoSliceMut alloc-free         │
├─────────────────────────────────────────────────────────────────┤
│                 Metrics (metrics.rs) [optional]                  │
│  packets_sent │ batches_sent │ syscalls │ batch_size histograms │
└─────────────────────────────────────────────────────────────────┘
```

### 3.1 模块职责

| 模块 | 职责 |
|------|------|
| `socket.rs` | `BingerUdp` 生命周期、from_std、async poll 集成 |
| `batch.rs` | `SendBatch<N>` / `RecvBatch<N>` — 编译期固定大小的批量容器 |
| `bufs.rs` | `BufferPool` — 预分配缓冲池，热路径零分配 |
| `platform/linux.rs` | `sendmmsg`/`recvmmsg` + GSO/GRO unsafe 封装 |
| `platform/macos.rs` | `sendmsg_x`/`recvmsg_x` unsafe 封装 |
| `platform/windows.rs` | `WSASendMsg`/`WSARecvMsg` unsafe 封装 |
| `platform/fallback.rs` | 逐个 `sendto`/`recvfrom` 循环 |
| `platform/mod.rs` | 编译期平台分发 |
| `error.rs` | `BingerError` 枚举 |
| `metrics.rs` | `BingerMetrics` — 原子计数器、可选的直方图 |

### 3.2 平台分发方式

```rust
// platform/mod.rs
#[cfg(all(target_os = "linux", not(feature = "miri-safe")))]
mod linux;
#[cfg(all(target_os = "linux", not(feature = "miri-safe")))]
pub(crate) use linux as imp;

#[cfg(all(target_os = "macos", not(feature = "miri-safe")))]
mod macos;
#[cfg(all(target_os = "macos", not(feature = "miri-safe")))]
pub(crate) use macos as imp;

#[cfg(all(target_os = "windows", not(feature = "miri-safe")))]
mod windows;
#[cfg(all(target_os = "windows", not(feature = "miri-safe")))]
pub(crate) use windows as imp;

#[cfg(any(
    feature = "miri-safe",
    not(any(target_os = "linux", target_os = "macos", target_os = "windows"))
))]
mod fallback;
#[cfg(any(
    feature = "miri-safe",
    not(any(target_os = "linux", target_os = "macos", target_os = "windows"))
))]
pub(crate) use fallback as imp;
```

---

## 4. 缓冲池设计

### 4.1 设计原则

- **预分配**：构造时分配全部缓冲区，send/recv 热路径不碰堆
- **无锁复用**：`crossbeam::ArrayQueue` 或 `std::sync::mpsc` 做池
- **编译期批量大小**：`SendBatch<const N: usize>` 泛型确定数组大小
- **零拷贝接收**：数据直接写入 `RecvBatch` 的预分配 buffer，不复制

### 4.2 BufferPool

```
BufferPool {
    pool: ArrayQueue<BytesMut>,  // 无锁队列
    buf_size: usize,              // 每个 buffer 的大小
    capacity: usize,              // 池容量
}

get():
    if pool.pop() == Some(buf):
        buf.clear()
        return buf
    else:
        return BytesMut::zeroed(buf_size)  // 懒分配

put(buf):
    let _ = pool.push(buf)  // 满则丢弃（GC 处理）
```

### 4.3 SendBatch 内部布局

```rust
struct SendBatch<const N: usize> {
    // 预分配的头部数组 — 与平台相关
    headers: [SendHeader; N],       // Linux: mmsghdr, macOS: 自定义
    // IoSlice 数组 — 指向用户传入的 &[u8]
    slices: [IoSlice<'static>; N],  // unsafe: 生命周期由调用者保证
    // 地址数组
    addrs: [SocketAddr; N],
    // 有效条目数
    len: usize,
}
```

### 4.4 RecvBatch 内部布局

```rust
struct RecvBatch<const N: usize> {
    // 预分配接收缓冲区
    buf: Box<[u8; N * MAX_BUF_SIZE]>,
    // 元数据数组 — 填充后记录每条消息的 [offset, len, addr]
    meta: [RecvMeta; N],
    // 接收到的有效条目数
    len: usize,
}
```

---

## 5. 异步集成设计

### 5.1 Tokio Poll 模型

```
BingerUdp {
    fd: RawFd,
    tokio_sock: UdpSocket,     // Tokio UdpSocket（用于 poll）
    metrics: Option<Metrics>,  // 可选指标
}

impl BingerUdp {
    async fn send_batch(&self, batch: &mut SendBatch) -> io::Result<usize> {
        loop {
            match self.inner.try_send_batch(batch) {
                Ok(n) => return Ok(n),
                Err(ref e) if e.kind() == WouldBlock => {
                    self.io.writable().await?;  // 等待 socket 可写
                }
                Err(e) => return Err(e),
            }
        }
    }

    async fn recv_batch(&self, batch: &mut RecvBatch) -> io::Result<usize> {
        loop {
            match self.inner.try_recv_batch(batch) {
                Ok(n) => return Ok(n),
                Err(ref e) if e.kind() == WouldBlock => {
                    self.io.readable().await?;  // 等待 socket 可读
                }
                Err(e) => return Err(e),
            }
        }
    }
}
```

### 5.2 平台适配

- **Linux**：标准 fd → `tokio::io::Registration::new()` 注册到 epoll
- **macOS**：标准 fd → kqueue 注册（Tokio 已处理）
- **Windows**：SOCKET → IOCP 注册（Tokio 已处理）

所有平台通过 Tokio 的 `Registration` 统一抽象，内部平台代码仅提供同步的 `try_send_batch` / `try_recv_batch`。

---

## 6. 可观测性设计

### 6.1 指标层次

```
BingerMetrics (feature = "metrics" 开启，否则零开销编译消除)

层次 1: 基础计数器（始终可用，AtomicU64）
    ├── packets_sent      — 总发送包数
    ├── packets_received   — 总接收包数
    ├── batches_sent       — 总批次数
    ├── batches_received   — 总接收批次数
    ├── send_syscalls      — 总发送 syscall 数
    ├── recv_syscalls      —— 总接收 syscall 数
    ├── send_would_block   — WouldBlock 次数
    └── recv_would_block   — WouldBlock 次数

层次 2: 采集指标（需主动 snapshot）
    ├── avg_batch_size    — 滑动平均批量大小
    ├── throughput_tx_bps — 发送吞吐（byte/s）
    ├── throughput_rx_bps — 接收吞吐（byte/s）
    └── syscall_ratio     — 包/syscall 比（衡量批处理效率）

层次 3: 追踪 span（需 tracing feature）
    └── send_batch{size, result, duration_us}
    └── recv_batch{size, result, duration_us}
```

### 6.2 自适应批量

```
if metrics.would_block_rate > 0.3:
    减小 batch_size（socket 拥塞，减少批量避免积压）
elif metrics.avg_batch_size > batch_size * 0.8:
    增大 batch_size（吞吐未饱和，增大批量提高效率）
```

---

## 7. 应用场景与需求映射

### 7.1 场景矩阵

| 场景 | 连接模式 | 包大小 | 包速率 | 关键需求 | binger 特性 |
|------|---------|--------|--------|---------|------------|
| KCP 服务端 | 多 conn | ~1400B | 中高 | 批量发送到不同 peer | sendmmsg |
| KCP 客户端 | connected | ~1400B | 中 | 批量发送到单 peer | GSO |
| 游戏服务器 | 多 client | 50-500B | 极高 | 低延迟 + 批量接收 | recvmmsg + busy-poll |
| DNS 服务器 | connectionless | 100-512B | 极高 | 批量收发 + 多目标 | sendmmsg + recvmmsg |
| Metrics 采集 | connectionless | 200-1KB | 高 | fire-and-forget | sendmmsg |
| RTP 媒体流 | connected | ~1200B | 极高 | pacing + timestamps | pacing feature |
| 服务网格 proxy | 多 conn | 不定 | 极高 | src IP 保留 + 批量 | sendmmsg + pktinfo |
| IoT 网关 | 多 device | 50-200B | 低-中 | 小包批量 + 嵌入式 | recvmmsg + no_std |

### 7.2 版本路线图

| 版本 | 内容 | 时间 |
|------|------|------|
| v0.1 | Linux sendmmsg/recvmmsg + fallback + Tokio async + BufferPool | MVP |
| v0.2 | macOS sendmsg_x/recvmsg_x + Windows WSASendMsg/WSARecvMsg | 全平台 |
| v0.3 | GSO/GRO + 自适应批量 + Metrics | 性能特性 |
| v0.4 | Pacing + busy-poll + timestamping + pktinfo | 高级特性 |
| v0.5 | no_std + embedded (embassy-net 集成) | 嵌入式 |
| v1.0 | 稳定 API + 全面测试 + 文档 | 生产就绪 |

---

## 8. 安全边界

### 8.1 unsafe 隔离

所有 `unsafe` 代码**仅**存在于 `platform/` 子模块中：

- `platform/linux.rs`：`libc::sendmmsg()`、`libc::recvmmsg()`、`libc::setsockopt()`（GSO）
- `platform/macos.rs`：`libc::sendmsg_x()`、`libc::recvmsg_x()`
- `platform/windows.rs`：`WSASendMsg()`、`WSARecvMsg()`、IOCP 操作
- `platform/fallback.rs`：**零 unsafe** — 纯 safe 逐个 sendto/recvfrom

公共 API 面 100% safe Rust。

### 8.2 生命周期安全

`SendBatch` 中的 `IoSlice` 指向用户提供的 `&[u8]`。生命周期安全由以下保证：

- `push()` 需要 `&[u8]`，内部持有 raw pointer，但：
  - `send_batch()` 是消耗性操作（flush 后 batch 清空）
  - 不跨 `await` 点持有 raw pointer
  - Miri 检测 use-after-free

### 8.3 `miri-safe` feature

```
cargo +nightly miri test --features miri-safe
```

此 feature 强制走 fallback 路径（纯 safe），允许 Miri 验证所有平台无关逻辑。

---

## 9. 与 KCP 的集成方案

### 9.1 作为独立 crate

binger 不依赖 kcp2，保持独立。集成通过 `kcp2-std` 侧完成：

```rust
// kcp2-std 中新增
#[cfg(feature = "binger")]
impl KcpTransport for BingerTransport {
    fn try_send(&self, buf: &[u8]) -> io::Result<usize> {
        // 单包便利包装
        self.inner.try_send_to(buf, self.peer)
    }
    fn try_send_to(&self, buf: &[u8], target: SocketAddr) -> io::Result<usize> {
        self.inner.try_send_to(buf, target)
    }
    fn recv<'a>(&'a self, buf: &'a mut [u8]) -> RecvFuture<'a> { ... }
    fn recv_from<'a>(&'a self, buf: &'a mut [u8]) -> RecvFromFuture<'a> { ... }
    fn local_addr(&self) -> io::Result<SocketAddr> { ... }
}
```

### 9.2 listener 批量优化

当前 `listener::accept()` 逐个 `recv_from()`。改进：

```rust
// 使用 RecvBatch 批量接收，一次 recvmmsg 拿多个包
let mut batch = RecvBatch::<32>::new(2048);
let n = self.socket.recv_batch(&mut batch).await?;
for i in 0..n {
    let conv = extract_conv(batch.data(i));
    let conn = self.get_or_create_conn(conv, batch.addr(i));
    conn.input_bytes(batch.data(i));
}
```

### 9.3 actor drain_output 批量优化

```rust
fn drain_output(&mut self) {
    let mut batch = SendBatch::<32>::new();
    while let Some(pkt) = self.collected.pop() {
        batch.push(&payload, self.peer);
        if batch.len() >= 32 {
            self.transport.send_batch(&batch);
            batch.clear();
        }
    }
    if !batch.is_empty() {
        self.transport.send_batch(&batch);
    }
}
```

---

## 10. 性能预期

### 理论收益

| 场景 | 当前 | binger | 收益 |
|------|------|--------|------|
| KCP flush 产 16 个包 (MTU=1400) | 16 次 sendto | 1 次 sendmmsg | ~60-80% syscall 减少 |
| KCP listener 排队 32 个入站包 | 32 次 recvfrom | 1 次 recvmmsg | ~80-90% syscall 减少 |
| 游戏服务器 10K pps | 10K syscall/s | ~300 syscall/s (batch=32) | ~97% syscall 减少 |
| DNS 递归解析器 | 每查询 1 次 sendto | 批量 32 查询 1 次 sendmmsg | 吞吐 2-3x |

### 基准测试计划

```
benches/batch_bench.rs:
    - single_vs_batch_send      — 1, 4, 8, 16, 32 包串行 vs 批量发送
    - single_vs_batch_recv      — 同上，接收
    - gso_vs_sendmmsg           — connected 模式 GSO vs sendmmsg
    - adaptive_batch_ramp       — WouldBlock 触发的自适应调整
    - mixed_destinations        — 多目标批量发送
```

---

## 参考资料

- [quinn-udp 架构](https://github.com/quinn-rs/quinn/tree/main/quinn-udp)
- [Solana recvmmsg 实现](https://github.com/solana-labs/solana/tree/master/streamer/src)
- [socket2 sendmmsg PR #583](https://github.com/rust-lang/socket2/pull/583)
- [nix sendmmsg API 讨论](https://github.com/nix-rust/nix/issues/2459)
- [Linux udp(7) man page](https://man7.org/linux/man-pages/man7/udp.7.html)
