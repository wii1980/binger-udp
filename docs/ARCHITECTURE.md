# binger-udp 架构设计

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
4. **async 是 poll 问题，不是 try_io**：与 Tokio reactor 深度集成，通过 `UdpSocket` 原生 poll 接口
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

Layer 1.5: 运行时能力检测 (macOS/Windows)
    ├── macOS → dlsym(RTLD_DEFAULT, "sendmsg_x") → 成功则用 sendmsg_x，否则 fallback sendmsg 循环
    └── Windows → WSAIoctl(SIO_GET_EXTENSION_FUNCTION_POINTER) → 成功则用 WSARecvMsg，否则 recvfrom

Layer 1: Linux 优化路径
    ├── connected + GSO → sendmsg w/ GSO cmsg (显式 API 调用)
    ├── multi-dest        → sendmmsg (多目标批量)
    ├── single-dest batch → sendmmsg (单目标但非 GSO)
    └── recv              → recvmmsg + GRO + cmsg 解析

Layer 2: 运行时降级 (Linux)
    ├── sendmmsg → ENOSYS → 逐个 sendto/send
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

### 2.2 GSO/GRO + sendmmsg 策略

binger 的 GSO 不是自动选择的批量路径，而是一个独立的显式 API。

**GSO 实现** (`try_send_gso` in `platform/linux.rs`):

```rust
pub(crate) fn try_send_gso(fd: Fd, data: &[u8], segment_size: u16) -> io::Result<usize>
```

- 构造 `msghdr`，通过 `CMSG_FIRSTHDR` + `CMSG_DATA` 写入 `UDP_SEGMENT` cmsg
- 调用 `sendmsg` 发送一个大 buffer，内核在 `segment_size` 边界分片
- 不是 setsockopt 开关，而是每次发送通过 cmsg 携带分段信息
- 需要 connected socket + `gso` feature
- 对应 `BingerUdp::try_send_gso()` 和 `BingerUdp::send_gso()`（async WouldBlock 重试）

**`set_gso()`** 函数 (`socket.rs` 第 447 行) 是一个独立的 setsockopt 开关，用于启用/禁用接口层面的 GSO 能力，不直接参与批量发送路径选择。

**GRO 实现**：集成在 `try_recv_batch()` 内部。`recvmmsg` 的每个 slot 预留 256 字节 cmsg 缓冲区。收到后解析 `UDP_GRO` cmsg 获取 `segment_size`。如果 > 0，将 coalesced 的大包拆分成多个独立的 `RecvSlot`，每个 slot 共享相同的 addr/timestamp/dst_addr。总输出数可能超过 `recvmmsg` 返回的报文数。

```
GSO 场景: connected socket, 大 buffer, 显式调用 try_send_gso()
sendmmsg 场景: 多目标或非 GSO 批量, 自动选择 sendmmsg/sendto
GRO 场景: 接收侧自动处理, 对用户透明

GSO 优势: 单目标大量小包场景, 内核分片比用户态构造多个 mmsghdr 更高效
sendmmsg 优势: 多目标场景, 可一次向不同 peer 发送不同数据
GRO 优势: 接收侧减少 syscall 数, 内核合并后拆分对用户透明
```

### 2.3 macOS 实现

macOS 没有 `sendmmsg`/`recvmmsg`，但有未文档化的 `sendmsg_x`/`recvmsg_x`（quinn-udp 已验证可用）。binger 在运行时通过 `dlsym` 动态解析这些符号：

- **运行时解析**: 使用 `dlsym(RTLD_DEFAULT, "sendmsg_x")` 和 `dlsym(RTLD_DEFAULT, "recvmsg_x")` 查找函数地址。结果缓存在 `OnceLock<usize>` 中。找到后通过 `mem::transmute` 将地址转为函数指针。
- **两阶段批量**: 先构造 msghdr_x 条目数组（包括 msg_name/msg_iov/msg_iovlen/msg_datalen），再通过单次 `sendmsg_x`/`recvmsg_x` 调用发送/接收整个数组。
- **EINTR 重试**: `recvmsg_x` 遇到 `EINTR` 时自动重试（`retry_eintr` 循环）。
- **回退路径**: 如果 `dlsym` 失败（地址为零），自动回退到逐个 `sendmsg`/`recvmsg` 循环（通过 `sockaddr::raw_sendto`/`raw_recvfrom`）。
- **没有 feature gate**: binger 没有 `fast-apple-datapath` feature — 始终优先尝试 `sendmsg_x`，失败时静默降级。

### 2.4 Windows 实现

Windows 没有原生的批量 UDP 系统调用。binger 的"批处理"实际上是对每个包循环调用 `WSASendMsg` / `WSARecvMsg`：

- **发送**: 循环调用 `WSASendMsg`，每个包构造一个 `WSAMSG` + `WSABUF` + 控制缓冲区（携带目标地址）。地址通过 `encode_addr_into` 从 `SocketAddr` 编码为 `SOCKADDR_STORAGE`。
- **接收**: 循环调用 `WSARecvMsg`，每个包准备 `WSAMSG` 接收缓冲区。收到后通过 `decode_sockaddr` 从 `SOCKADDR_STORAGE` 解析为 `SocketAddr`。
- **WSARecvMsg 加载**: 通过 `WSAIoctl(SIO_GET_EXTENSION_FUNCTION_POINTER)` 运行时获取函数指针，结果缓存在 `OnceLock<Option<WsaRecvMsgFn>>` 中。加载失败时自动回退到 `recvfrom`。
- **WSASendMsg**: 直接从 `ws2_32.dll` 导出，无需动态加载。
- **地址编组**: `encode_addr_into` / `decode_sockaddr` 在 `SocketAddr` 与 `SOCKADDR_IN`/`SOCKADDR_IN6` 之间转换。

当前实现是同步的，尚未集成 IOCP。

### 2.5 版本检测策略

binger 不使用 build.rs 进行编译期版本检测。所有平台能力检测在运行时完成：

- **macOS**: `dlsym(RTLD_DEFAULT, "sendmsg_x")` 检查内核是否支持 sendmsg_x/recvmsg_x。失败则自动回退到 sendmsg/recvmsg 循环。
- **Windows**: `WSAIoctl(SIO_GET_EXTENSION_FUNCTION_POINTER)` 加载 WSARecvMsg 扩展函数。失败则回退到 recvfrom。
- **Linux**: ENOSYS 处理 — sendmmsg/recvmmsg 返回 ENOSYS 时自动回退到 sendto/recvfrom。
- **最低 Rust 版本**: edition 2021, rust-version = "1.75"

这种纯运行时策略意味着：
1. 不需要 build dependency
2. 编译一次，运行时自动适配
3. 没有版本相关的 cfg 配置

---

## 3. 组件架构

```
┌──────────────────────────────────────────────────────────────────┐
│                         Public API                             │
│  BingerUdp  │  SendBatch<N>  │  RecvBatch<N>  │  Config       │
│  PlatformCaps │ recommended_batch_size()  │  Timestamp       │
├──────────────────────────────────────────────────────────────────┤
│                      Async Layer (tokio)                        │
│  send_batch/recv_batch  │  send_gso  │  wait_writable/readable  │
│  自适应 WouldBlock 重试 + AdaptiveState 记录                     │
├──────────────────────────────────────────────────────────────────┤
│                     Platform Dispatch                            │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌────────────────┐ │
│  │  linux   │  │  macos   │  │ windows  │  │   fallback     │ │
│  │ sendmmsg │  │sendmsg_x │  │WSASendMsg│  │  loop sendto   │ │
│  │ recvmmsg │  │recvmsg_x │  │WSARecvMsg│  │ loop recvfrom  │ │
│  │ try_send │  │          │  │          │  │                │ │
│  │  _gso 🔒 │  │          │  │          │  │                │ │
│  │ GRO +    │  │          │  │          │  │                │ │
│  │ cmsg 解析│  │          │  │          │  │                │ │
│  │ retry_   │  │          │  │          │  │                │ │
│  │ eintr    │  │          │  │          │  │                │ │
│  │ 🔒stack  │  │          │  │          │  │                │ │
│  │ arrays   │  │          │  │          │  │                │ │
│  └──────────┘  └──────────┘  └──────────┘  └────────────────┘ │
├──────────────────────────────────────────────────────────────────┤
│                    sys/ + sockaddr/                               │
│  Fd type  │  close_fd  │  SOL_SOCKET/IPPROTO_UDP 常量           │
│  SO_BUSY_POLL(75)  │  SO_MAX_PACING_RATE(79)                    │
│  raw_setsockopt_u32  │  raw_setsockopt_timeval                  │
├──────────────────────────────────────────────────────────────────┤
│                    Buffer Pool (bufs.rs)                         │
│  ArrayQueue<BytesMut>  │  IoSlice/IoSliceMut alloc-free         │
├──────────────────────────────────────────────────────────────────┤
│              AdaptiveState (socket.rs) [optional]                │
│  target_size  │  would_block_count  │  total_send_count         │
│  maybe_adjust()  │  100ms 间隔  │  30%/10% 阈值                │
├──────────────────────────────────────────────────────────────────┤
│              Metrics (metrics.rs) [optional]                     │
│  packets_sent/received  │  batches_sent/received  │  syscalls   │
└──────────────────────────────────────────────────────────────────┘
```

### 3.1 模块职责

| 模块 | 职责 | 行数 |
|------|------|------|
| `socket.rs` | `BingerUdp` 生命周期、from_std、async poll 集成、`Config`、`PlatformCaps`、`AdaptiveState`、GSO/GRO/pacing/busy-poll/timestamping/pktinfo socket 选项 | 656 |
| `batch.rs` | `SendBatch<N>`/`RecvBatch<N>` 编译期固定批量容器、`SendBatchRaw`/`RecvBatchRaw`、`Timestamp`、feature-gated 字段 | 329 |
| `bufs.rs` | `BufferPool` — 预分配缓冲池，热路径零分配 | 169 |
| `sockaddr.rs` | `encode_sockaddr`/`decode_sockaddr`、`raw_sendto/send/recvfrom`、`raw_setsockopt/getsockopt/connect/getsockname`、`raw_setsockopt_u32`/`raw_setsockopt_timeval` | 250 |
| `platform/linux.rs` | `try_send_batch` (sendmmsg + 栈数组/vec 双路径)、`try_recv_batch` (recvmmsg + GRO + cmsg 解析)、`try_send_gso`、`retry_eintr`、fallback 函数 | 652 |
| `platform/macos.rs` | `sendmsg_x`/`recvmsg_x` dlsym 动态解析 + unsafe 封装 | 211 |
| `platform/windows.rs` | `WSASendMsg`/`WSARecvMsg` + `WSAIoctl` 动态加载 | 298 |
| `platform/fallback.rs` | 逐个 `sendto`/`recvfrom` 循环（零 unsafe） | 41 |
| `platform/mod.rs` | 编译期平台分发 (cfg gate) | 27 |
| `error.rs` | `BingerError` 枚举 | 24 |
| `metrics.rs` | `BingerMetrics` — 原子计数器、`MetricsSnapshot`、syscall_efficiency_ratio | 219 |
| `lib.rs` | crate 根, re-exports, feature-gated `Timestamp` | 52 |
| `sys/mod.rs` | 编译期平台选择 (unix/windows) | 12 |
| `sys/unix.rs` | Fd=RawFd, libc 常量 (SO_BUSY_POLL=75, SO_MAX_PACING_RATE=79 等), close_fd | 37 |
| `sys/windows.rs` | Fd=usize (SOCKET), windows-sys 常量, close_fd | 40 |
| **总计** | | **3,017** |

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

`sys/` 模块的编译期选择更简洁，直接区分 unix/windows：

```rust
// sys/mod.rs
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;
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
// batch.rs — 实际实现

struct SendSlot {
    data_ptr: *const u8,          // 指向用户提供的 &[u8]
    data_len: usize,
    addr: Option<SocketAddr>,     // None = connected 模式
    _marker: std::marker::PhantomData<*const [u8]>,
}

pub struct SendBatchRaw {
    slots: Vec<SendSlot>,
    len: usize,
}

// 公开泛型包装
pub struct SendBatch<const N: usize> {
    raw: SendBatchRaw,
}
```

`SendBatch<N>` 通过 `Deref`/`DerefMut` 委托给 `SendBatchRaw`。平台代码通过 `entry(idx) -> (&[u8], Option<SocketAddr>)` 访问。

**生命周期安全**：`data_ptr` 指向用户传入的 buffer，但 `send_batch()` 是消耗性操作 — flush 后 batch 清空，不跨 `await` 点持有 raw pointer。

### 4.4 RecvBatch 内部布局

```rust
// batch.rs — 实际实现

struct RecvSlot {
    buf: Vec<u8>,
    addr: SocketAddr,
    recv_len: u16,
    #[cfg(feature = "timestamping")]
    timestamp: Option<Timestamp>,    // 内核时间戳
    #[cfg(feature = "pktinfo")]
    dst_addr: Option<SocketAddr>,    // 目标地址
}

pub struct RecvBatchRaw {
    slots: Vec<RecvSlot>,
    buf_size: usize,
    len: usize,
}

// 公开泛型包装
pub struct RecvBatch<const N: usize> {
    raw: RecvBatchRaw,
}
```

`recv_len` 使用 `u16`（最大 65535 字节，适合 UDP 数据报）。feature-gated 字段：
- `timestamp` (feature = `timestamping`) — 内核 `SCM_TIMESTAMPNS` 时间戳
- `dst_addr` (feature = `pktinfo`) — 接收时的目标地址（用于多宿主机器的源地址确认）

### 4.5 零分配栈数组路径

Linux 平台的 `try_send_batch` 和 `try_recv_batch` 实现了双路径分配策略：

```rust
// platform/linux.rs
const MAX_STACK: usize = 64;  // 栈路径阈值
const CMSG_BUF_SIZE: usize = 256;  // 每个 slot 的 cmsg 缓冲区

if len <= MAX_STACK {
    // 栈路径：零分配
    let mut msgs: [libc::mmsghdr; MAX_STACK] = unsafe { mem::zeroed() };
    let mut iovecs: [libc::iovec; MAX_STACK] = unsafe { mem::zeroed() };
    let mut addrs: [libc::sockaddr_storage; MAX_STACK] = unsafe { mem::zeroed() };
    // 接收路径还有 cmsg_bufs: [[u8; CMSG_BUF_SIZE]; MAX_STACK]
    
    // 单次填充，直接 syscall
    let sent = unsafe { libc::sendmmsg(fd, msgs.as_mut_ptr(), len as u32, 0) };
} else {
    // Vec 路径：堆分配，两阶段指针修复
    let mut msgs: Vec<libc::mmsghdr> = Vec::with_capacity(len);
    // ... 先填充数据，再修复 msg_iov/msg_name 指针
}
```

**栈路径优势**：batch <= 64 时完全零分配。栈数组使用 `mem::zeroed()` 一次性初始化（POD C 结构体全零位模式合法），单次填充后直接 syscall。

**Vec 路径特点**：batch > 64 时退化为 `Vec`。因为 `Vec::push` 可能触发 reallocation 使地址不稳定，使用两阶段策略：先 push 所有条目，再遍历修复 `msg_iov` 和 `msg_name` 指针。

**接收路径额外特性**：
- 每个 slot 预留 256 字节 cmsg 缓冲区用于 GRO/timestamping/pktinfo 解析
- 使用 `retry_eintr` 包装 `recvmmsg` 自动处理 `EINTR` 信号中断
- 解析 `CMSG_FIRSTHDR` / `CMSG_NXTHDR` 链读取 `UDP_GRO`、`SCM_TIMESTAMPNS`、`IP_PKTINFO`、`IPV6_PKTINFO`

---

## 5. 异步集成设计

### 5.1 Tokio Poll 模型

```rust
// socket.rs — 实际实现

pub struct BingerUdp {
    fd: Fd,
    #[cfg(feature = "tokio")]
    tokio_sock: tokio::net::UdpSocket,  // 用于 poll
    #[cfg(feature = "metrics")]
    metrics: Option<BingerMetrics>,
    adaptive_batching: bool,
    adaptive_state: Option<Mutex<AdaptiveState>>,
}

impl BingerUdp {
    pub async fn send_batch(&self, batch: &mut SendBatchRaw) -> io::Result<usize> {
        loop {
            match self.try_send_batch(batch) {
                Ok(n) => return Ok(n),
                Err(ref e) if e.kind() == WouldBlock => {
                    // 记录 WouldBlock 事件给自适应批量
                    if let Some(ref state) = self.adaptive_state {
                        let mut s = state.lock().unwrap();
                        s.record_would_block();
                        s.maybe_adjust();
                    }
                    self.wait_writable().await?;
                }
                Err(e) => return Err(e),
            }
        }
    }

    pub async fn recv_batch(&self, batch: &mut RecvBatchRaw) -> io::Result<usize> {
        loop {
            match self.try_recv_batch(batch) {
                Ok(n) => return Ok(n),
                Err(ref e) if e.kind() == WouldBlock => {
                    if let Some(ref state) = self.adaptive_state {
                        let mut s = state.lock().unwrap();
                        s.record_would_block();
                        s.maybe_adjust();
                    }
                    self.wait_readable().await?;
                }
                Err(e) => return Err(e),
            }
        }
    }

    // 非 async 版本
    pub fn try_send_batch(&self, batch: &mut SendBatchRaw) -> io::Result<usize> {
        let sent = crate::platform::try_send_batch(self.fd, batch)?;
        if let Some(ref state) = self.adaptive_state {
            let mut s = state.lock().unwrap();
            s.record_send(sent);
        }
        // ... metrics 记录
        Ok(sent)
    }
}
```

关键差异：实际实现使用 `tokio_sock.writable()` / `tokio_sock.readable()`（而非 `Registration`），通过 Tokio 的 UdpSocket 原生 poll 接口。

### 5.2 平台适配

所有平台通过 `tokio::net::UdpSocket::from_std()` 统一注册到 Tokio reactor（Linux epoll / macOS kqueue / Windows IOCP），内部平台代码仅提供同步的 `try_send_batch` / `try_recv_batch`。`BingerUdp` 持有转换后的 tokio UdpSocket 用于 poll：

```rust
#[cfg(feature = "tokio")]
let tokio_sock = tokio::net::UdpSocket::from_std(socket)?;
// 后续使用 tokio_sock.writable().await / tokio_sock.readable().await
```

### 5.3 自适应批量集成

自适应批量系统是一个位于 `socket.rs` 中的闭环控制器：

```rust
// socket.rs — AdaptiveState

struct AdaptiveState {
    target_size: usize,           // 当前推荐批量大小
    would_block_count: u64,       // 当前周期内 WouldBlock 次数
    total_send_count: u64,        // 当前周期内总发送操作数
    last_adjustment: Instant,     // 上次调整时间
}

impl AdaptiveState {
    const MIN_BATCH: usize = 1;
    const MAX_BATCH: usize = 1024;
    const ADJUSTMENT_INTERVAL: Duration = Duration::from_millis(100);

    fn maybe_adjust(&mut self) {
        if self.last_adjustment.elapsed() < Self::ADJUSTMENT_INTERVAL {
            return;  // 100ms 内不重复调整
        }
        if self.total_send_count == 0 {
            return;
        }
        let wb_rate = self.would_block_count as f64 / self.total_send_count as f64;
        if wb_rate > 0.3 {
            self.target_size = (self.target_size / 2).max(Self::MIN_BATCH);
        } else if wb_rate < 0.1 && self.target_size < Self::MAX_BATCH {
            self.target_size = (self.target_size * 3 / 2).min(Self::MAX_BATCH);
        }
        self.would_block_count = 0;
        self.total_send_count = 0;
        self.last_adjustment = Instant::now();
    }
}
```

**触发流程**:

1. `send_batch()` / `recv_batch()` async 方法遇到 `WouldBlock` → 调用 `record_would_block()` + `maybe_adjust()`
2. `try_send_batch()` 成功时 → 调用 `record_send()`
3. `maybe_adjust()` 每 100ms 检查一次 WouldBlock 率:
   - **> 30%**: 批量大小减半（socket 拥塞，降低负载）
   - **< 10%**: 批量大小增加 50%（带宽有余，提高效率）
   - **10-30%**: 保持当前值（处于理想区间）
4. `recommended_batch_size()` 暴露当前 `target_size`，调用者可据此构造更小/更大的 batch

**Mutex 策略**: `AdaptiveState` 使用 `Option<Mutex<AdaptiveState>>` 存储在 `BingerUdp` 中。`Config::with_adaptive_batching(true)` 启用时创建，否则为 `None`。`Mutex` 是因为 `Send`/`Sync` 约束 — `BingerUdp` 需要是 `Sync`（`&self` 方法在多线程可用），`AdaptiveState` 的计数器更新需要内部可变性。

---

## 6. 可观测性设计

### 6.1 指标层次

```
BingerMetrics (feature = "metrics" 开启，否则零开销编译消除)

全部计数器（AtomicU64, Relaxed ordering）:
    ├── packets_sent          — 总发送包数
    ├── packets_received      — 总接收包数
    ├── batches_sent          — 总批次数
    ├── batches_received      — 总接收批次数
    ├── send_syscalls         — 总发送 syscall 数
    ├── recv_syscalls         — 总接收 syscall 数
    ├── send_errors           — 发送错误数
    ├── recv_errors           — 接收错误数
    ├── send_would_block      — WouldBlock 次数
    └── recv_would_block      — WouldBlock 次数

派生指标（通过 snapshot() 计算）:
    └── syscall_efficiency    — 包/syscall 比（packets_sent / send_syscalls）
```

所有计数器都是基础原子计数器，没有滑动平均或 tracing span（这些属于外部可观测性系统的职责）。`snapshot()` 返回 `MetricsSnapshot` 结构体的一次性快照。

### 6.2 自适应批量（实际实现）

自适应批量不依赖 metrics 系统，而是独立的 `AdaptiveState` 控制器（`socket.rs`）：

```
AdaptiveState (socket.rs, 独立于 metrics feature)
    ├── target_size              — 当前推荐批量大小
    ├── would_block_count        — 当前 100ms 窗口内的 WouldBlock 次数
    ├── total_send_count         — 当前 100ms 窗口内的总发送次数
    └── last_adjustment          — 上次调整时间

调整逻辑 (maybe_adjust, 每 100ms 触发):
    如果 would_block_rate > 30%:  target_size /= 2
    如果 would_block_rate < 10%:  target_size = target_size * 3 / 2
    否则:                         target_size 保持不变

暴露接口:
    recommended_batch_size() -> usize  // 默认 32, 范围 [1, 1024]
```

**与 metrics 的关系**: 自适应批量与 metrics 是正交的。即使 `feature = "metrics"` 未启用，自适应批量仍然正常工作。metrics 提供的是全局观测，自适应批量提供的是实时控制。

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
| IoT 网关 | 多 device | 50-200B | 低-中 | 小包批量 | recvmmsg |

### 7.2 版本路线图

| 版本 | 内容 | 时间 |
|------|------|------|
| v0.1 | Linux sendmmsg/recvmmsg + fallback + Tokio async + BufferPool | MVP |
| v0.2 | macOS sendmsg_x/recvmsg_x + Windows WSASendMsg/WSARecvMsg | 全平台 |
| v0.3 | GSO/GRO + 自适应批量 + Metrics | ✅ 已完成 |
| v0.4 | Pacing + busy-poll + timestamping + pktinfo | ✅ 已完成 |
| v1.0 | 稳定 API + 全面测试 + 文档 | 生产就绪 |

---

## 8. 安全边界

### 8.1 unsafe 隔离

所有 `unsafe` 代码**仅**存在于 `sys/`、`sockaddr/` 和 `platform/` 子模块中：

- `sys/unix.rs`：`libc::close(RawFd)`
- `sys/windows.rs`：`closesocket(SOCKET)`
- `sockaddr.rs`：`libc::sendto()`、`libc::send()`、`libc::recvfrom()`、`libc::getsockname()`、`libc::connect()`、`libc::setsockopt()` （c_int/u32/timeval）、`libc::getsockopt()`、`encode_sockaddr` 中的指针写（`dst.write(raw)`）
- `platform/linux.rs`：
  - `sendmmsg` / `recvmmsg` syscall（栈数组路径 + vec 路径）
  - `try_send_gso`：`sendmsg` + `CMSG_FIRSTHDR` / `CMSG_DATA` cmsg 宏操作
  - `try_recv_batch` cmsg 解析：`CMSG_FIRSTHDR` / `CMSG_NXTHDR` 遍历 + `CMSG_DATA` 读取 `UDP_GRO` / `SCM_TIMESTAMPNS` / `IP_PKTINFO` / `IPV6_PKTINFO`
  - `mem::zeroed()` 初始化 POD 栈数组（`[libc::mmsghdr; 64]` 等）
  - `batch.set_recv_len()` 不安全函数调用
- `platform/macos.rs`：`dlsym(RTLD_DEFAULT)` 解析函数指针 + `mem::transmute` 转为函数类型 + `msghdr_x` 操作 + `retry_eintr`
- `platform/windows.rs`：`WSASendMsg()` + `WSARecvMsg()` + `WSAIoctl()` 加载扩展函数 + sockaddr 编组
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
