# udp-binger API 设计

## 1. 设计原则

1. **批量优先**：`send_batch` / `recv_batch` 是核心 API，`send_to` / `recv_from` 只是便利包装
2. **编译期容量**：`SendBatch<const N: usize>` / `RecvBatch<const N: usize>` 泛型确定大小，无运行时分配
3. **平台透明**：用户不写 `#[cfg]`，自动选择最优 syscall
4. **Builder 模式**：`Config` 用 `with_*()` 链式构造，不用 `#[cfg]` 污染结构体字段
5. **async 原生**：`.await` 等待可读/可写，不与 `try_io` 打交道
6. **平台特异性后移到实例方法**：GSO/GRO/busy-poll 等平台特性在 `BingerUdp` 上设置，不在 `Config` 中

---

## 2. 核心类型

### 2.1 `Config`

```rust
#[derive(Debug, Clone)]
pub struct Config { /* fields private */ }

impl Default for Config {
    fn default() -> Self {
        Config::new()
            .with_batch_size(32)
            .with_recv_buf_size(2048)
    }
}

impl Config {
    pub fn new() -> Self;

    /// 批量大小（默认 32）
    pub fn with_batch_size(self, n: usize) -> Self;

    /// 每个包接收缓冲区大小（默认 2048）
    pub fn with_recv_buf_size(self, n: usize) -> Self;

    /// OS 发送缓冲区 SO_SNDBUF（默认不设置）
    pub fn with_send_buf_size(self, n: usize) -> Self;

    /// OS 接收缓冲区 SO_RCVBUF（默认不设置）
    pub fn with_recv_os_buf_size(self, n: usize) -> Self;

    /// 根据背压自适应调整 batch_size（默认 false）
    pub fn with_adaptive_batching(self, enabled: bool) -> Self;

    /// 启用内置指标（默认 false，需 feature = "metrics"）
    #[cfg(feature = "metrics")]
    pub fn with_metrics(self, enabled: bool) -> Self;
}
```

### 2.2 `BingerUdp`

```rust
pub struct BingerUdp { /* internal */ }
```

#### 构造

```rust
impl BingerUdp {
    /// 从 std::net::UdpSocket 构造（推荐方式）。
    pub fn from_std(socket: std::net::UdpSocket, config: Config) -> io::Result<Self>;
}
```

#### 批量 API（核心）

```rust
impl BingerUdp {
    /// 批量发送。一次 syscall（Linux: sendmmsg, macOS: sendmsg_x, Windows: WSASendMsg 循环）。
    /// 返回实际发送的包数。
    pub async fn send_batch(&self, batch: &mut SendBatchRaw) -> io::Result<usize>;

    /// 批量接收。一次 syscall（Linux: recvmmsg, macOS: recvmsg_x, Windows: WSARecvMsg 循环）。
    /// 返回实际接收的包数。
    pub async fn recv_batch(&self, batch: &mut RecvBatchRaw) -> io::Result<usize>;

    /// 非阻塞批量发送。
    pub fn try_send_batch(&self, batch: &mut SendBatchRaw) -> io::Result<usize>;

    /// 非阻塞批量接收。
    pub fn try_recv_batch(&self, batch: &mut RecvBatchRaw) -> io::Result<usize>;
}
```

由于 `SendBatch<N>` / `RecvBatch<N>` 实现了 `DerefMut<Target = SendBatchRaw>` / `DerefMut<Target = RecvBatchRaw>`，用户可直接传入 `&mut SendBatch<N>` / `&mut RecvBatch<N>`，Rust 会自动解引用。

#### 单包 API（便利包装）

```rust
impl BingerUdp {
    pub async fn send_to(&self, buf: &[u8], addr: SocketAddr) -> io::Result<usize>;
    pub async fn recv_from(&self, buf: &mut [u8]) -> io::Result<(usize, SocketAddr)>;
    pub fn try_send_to(&self, buf: &[u8], addr: SocketAddr) -> io::Result<usize>;
}
```

#### 连接管理

```rust
impl BingerUdp {
    /// 连接到远程地址（启用 connected 模式，解锁 GSO 优化）。
    pub fn connect(&self, addr: SocketAddr) -> io::Result<()>;
}
```

#### Socket 选项

```rust
impl BingerUdp {
    pub fn local_addr(&self) -> io::Result<SocketAddr>;
    pub fn ttl(&self) -> io::Result<u32>;
    pub fn set_ttl(&self, ttl: u32) -> io::Result<()>;

    /// 获取底层 fd。Unix 上是 RawFd (c_int)，Windows 上是 SOCKET (usize)。
    pub fn as_raw_fd(&self) -> crate::sys::Fd;
}
```

#### 平台特异性（不在 Config 中，避免 #[cfg] 污染）

```rust
impl BingerUdp {
    /// 启用 GSO（Linux connected 模式，需 feature = "gso"）
    #[cfg(all(target_os = "linux", feature = "gso"))]
    pub fn set_gso(&self, enabled: bool) -> io::Result<()>;

    /// 启用 GRO（Linux 接收，需 feature = "gro"）
    #[cfg(all(target_os = "linux", feature = "gro"))]
    pub fn set_gro(&self, enabled: bool) -> io::Result<()>;

    /// GSO 分段发送 — 内核按 segment_size 分片发送大数据报。
    /// 需 connected 模式 + Linux + feature = "gso" + 非 miri-safe。
    #[cfg(all(target_os = "linux", feature = "gso", not(feature = "miri-safe")))]
    pub fn try_send_gso(&self, data: &[u8], segment_size: u16) -> io::Result<usize>;

    /// GSO 分段发送（WouldBlock 重试版）。
    #[cfg(all(target_os = "linux", feature = "gso", not(feature = "miri-safe")))]
    pub async fn send_gso(&self, data: &[u8], segment_size: u16) -> io::Result<usize>;

    /// 设置最大 pacing 速率（字节/秒）。0 表示禁用。需 fq qdisc。
    #[cfg(all(target_os = "linux", feature = "pacing"))]
    pub fn set_pacing_rate(&self, bytes_per_sec: u32) -> io::Result<()>;

    /// 启用 busy-poll（SO_BUSY_POLL）。usecs 为忙轮询微秒数。
    /// 需 CAP_NET_ADMIN 或 net.core.busy_poll sysctl 配置。
    #[cfg(all(target_os = "linux", feature = "busy-poll"))]
    pub fn set_busy_poll(&self, usecs: u32) -> io::Result<()>;

    /// 启用/禁用软件接收时间戳（SO_TIMESTAMPNS）。
    /// 启用后可通过 RecvBatch::timestamp() 获取内核时间戳。
    #[cfg(all(target_os = "linux", feature = "timestamping"))]
    pub fn enable_timestamping(&self, enabled: bool) -> io::Result<()>;

    /// 启用/禁用接收目标地址（IP_PKTINFO / IPV6_RECVPKTINFO）。
    /// 启用后可通过 RecvBatch::dst_addr() 获取每个包的目标地址。
    #[cfg(all(target_os = "linux", feature = "pktinfo"))]
    pub fn enable_pktinfo(&self, enabled: bool) -> io::Result<()>;
}
```

#### 查询

```rust
impl BingerUdp {
    /// 获取当前平台能力。
    pub fn capabilities(&self) -> PlatformCaps;

    /// 返回推荐 batch size。
    /// 自适应 batching 启用时动态调整；禁用时固定返回 32。
    pub fn recommended_batch_size(&self) -> usize;

    /// 获取指标引用（仅 feature = "metrics"）。
    #[cfg(feature = "metrics")]
    pub fn metrics(&self) -> Option<&BingerMetrics>;
}
```

### 2.3 `SendBatch<const N: usize>`

```rust
pub struct SendBatch<const N: usize> { /* internal */ }

impl<const N: usize> SendBatch<N> {
    /// 创建空批量发送容器。`N` 为最大容量。
    pub fn new() -> Self;

    /// 添加一个包（connectionless 模式）。
    /// 返回 `BingerError::BatchFull` 如果 `len == N`。
    pub fn push(&mut self, buf: &[u8], addr: SocketAddr) -> Result<(), BingerError>;

    /// 添加一个包（connected 模式）。
    pub fn push_connected(&mut self, buf: &[u8]) -> Result<(), BingerError>;

    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    pub const fn capacity(&self) -> usize;
    pub fn clear(&mut self);
}

impl<const N: usize> Default for SendBatch<N> { ... }
impl<const N: usize> Deref for SendBatch<N> { type Target = SendBatchRaw; }
impl<const N: usize> DerefMut for SendBatch<N> { ... }
```

### 2.4 `RecvBatch<const N: usize>`

```rust
pub struct RecvBatch<const N: usize> { /* internal */ }

impl<const N: usize> RecvBatch<N> {
    /// 创建批量接收容器。`buf_size` 为每个包的最大字节数。
    pub fn new(buf_size: usize) -> Self;

    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    pub const fn capacity(&self) -> usize;

    /// 获取第 `idx` 个包的数据。
    pub fn data(&self, idx: usize) -> &[u8];

    /// 获取第 `idx` 个包的来源地址。
    pub fn addr(&self, idx: usize) -> SocketAddr;

    /// 遍历所有接收到的包。
    pub fn iter(&self) -> impl Iterator<Item = (&[u8], SocketAddr)> + '_;

    pub fn clear(&mut self);

    /// 获取第 `idx` 个包的内核软件时间戳。
    /// 需 feature = "timestamping" 并通过 enable_timestamping(true) 启用。
    #[cfg(feature = "timestamping")]
    pub fn timestamp(&self, idx: usize) -> Option<Timestamp>;

    /// 获取第 `idx` 个包的目标地址。
    /// 需 feature = "pktinfo" 并通过 enable_pktinfo(true) 启用。
    #[cfg(feature = "pktinfo")]
    pub fn dst_addr(&self, idx: usize) -> Option<SocketAddr>;
}

impl<const N: usize> Deref for RecvBatch<N> { type Target = RecvBatchRaw; }
impl<const N: usize> DerefMut for RecvBatch<N> { ... }
```

### 2.5 `SendBatchRaw` / `RecvBatchRaw`

`pub(crate)` 类型，通过 `SendBatch<N>` / `RecvBatch<N>` 的 `Deref` trait 暴露给 `BingerUdp` 的 `send_batch` / `recv_batch` 方法。一般用户不需要直接使用这些类型。

### 2.6 `BufferPool`

```rust
pub struct BufferPool { /* internal */ }

impl BufferPool {
    pub fn new(capacity: usize, buf_size: usize) -> Self;
    pub fn get(&self) -> BytesMut;
    pub fn put(&self, buf: BytesMut);
    pub fn available(&self) -> usize;
    pub fn capacity(&self) -> usize;
}
```

### 2.7 `BingerMetrics`（feature = "metrics"）

```rust
pub struct BingerMetrics { /* internal */ }

impl BingerMetrics {
    pub fn packets_sent(&self) -> u64;
    pub fn packets_received(&self) -> u64;
    pub fn batches_sent(&self) -> u64;
    pub fn batches_received(&self) -> u64;
    pub fn send_syscalls(&self) -> u64;
    pub fn recv_syscalls(&self) -> u64;
    pub fn send_errors(&self) -> u64;
    pub fn recv_errors(&self) -> u64;
    pub fn send_would_block(&self) -> u64;
    pub fn recv_would_block(&self) -> u64;

    /// syscall 效率：packets_sent / send_syscalls。1.0=无批处理，32.0=完美批处理
    pub fn syscall_efficiency_ratio(&self) -> f64;

    pub fn reset(&self);
    pub fn snapshot(&self) -> MetricsSnapshot;
}
```

### 2.8 `PlatformCaps`

```rust
pub struct PlatformCaps {
    pub supports_sendmmsg: bool,
    pub supports_recvmmsg: bool,
    #[cfg(target_os = "macos")]
    pub supports_sendmsg_x: bool,
    #[cfg(target_os = "macos")]
    pub supports_recvmsg_x: bool,
    #[cfg(target_os = "windows")]
    pub supports_wsa_send_msg: bool,
    #[cfg(target_os = "windows")]
    pub supports_wsa_recv_msg: bool,
    pub supports_gso: bool,
    pub supports_gro: bool,
    pub supports_busy_poll: bool,
    pub supports_pacing: bool,
    #[cfg(feature = "timestamping")]
    pub supports_timestamping: bool,
    #[cfg(feature = "pktinfo")]
    pub supports_pktinfo: bool,
    pub max_batch_size: usize,
    pub backend_name: &'static str,
}

pub fn platform_capabilities() -> PlatformCaps;
```

每个 `#[cfg]` 字段仅在对应平台上编译，用户可直接引用而无需在代码中写自身 `#[cfg]`：

| 字段 | 条件 |
|------|------|
| `supports_sendmsg_x`, `supports_recvmsg_x` | `target_os = "macos"` |
| `supports_wsa_send_msg`, `supports_wsa_recv_msg` | `target_os = "windows"` |
| `supports_timestamping` | `feature = "timestamping"` |
| `supports_pktinfo` | `feature = "pktinfo"` |

### 2.9 `Timestamp`（feature = "timestamping"）

```rust
#[cfg(feature = "timestamping")]
#[derive(Clone, Copy, Debug, Default)]
pub struct Timestamp {
    pub tv_sec: i64,
    pub tv_nsec: i64,
}

#[cfg(feature = "timestamping")]
impl Timestamp {
    /// 转换为 std::time::Duration（相对于 Unix epoch）。
    pub fn as_duration(&self) -> std::time::Duration;
}
```

`Timestamp` 表示内核通过 `SO_TIMESTAMPNS` 附加到接收包上的软件时间戳。`tv_sec` 和 `tv_nsec` 对应 `timespec` 结构体的字段。

通过 [`RecvBatch::timestamp`] 方法获取：

```rust
#[cfg(all(feature = "timestamping", target_os = "linux"))]
if let Some(ts) = recv_batch.timestamp(i) {
    let dur = ts.as_duration();
    println!("packet {i} received at: {dur:?}");
}
```

---

## 3. 使用示例

### 基础用法

```rust
use udp_binger::{BingerUdp, SendBatch, RecvBatch, Config};
use std::net::UdpSocket;

#[tokio::main]
async fn main() -> io::Result<()> {
    let sock = UdpSocket::bind("0.0.0.0:0")?;
    let binger = BingerUdp::from_std(sock, Config::default())?;

    // 批量发送
    let mut send = SendBatch::<32>::new();
    for i in 0..32 {
        send.push(format!("packet-{i}").as_bytes(), "192.168.1.1:8080".parse()?)?;
    }
    let sent = binger.send_batch(&mut send).await?;
    assert_eq!(sent, 32);

    // 批量接收
    let mut recv = RecvBatch::<32>::new(2048);
    let n = binger.recv_batch(&mut recv).await?;
    for (data, addr) in recv.iter() {
        println!("from {addr}: {data:?}");
    }
    Ok(())
}
```

### 自定义配置

```rust
let config = Config::new()
    .with_batch_size(64)
    .with_recv_os_buf_size(65536);

let binger = BingerUdp::from_std(socket, config)?;
```

### 启用指标

```rust
let config = Config::new()
    .with_metrics(true)
    .with_adaptive_batching(true);

let binger = BingerUdp::from_std(socket, config)?;

// ... send/recv ...

if let Some(m) = binger.metrics() {
    println!("efficiency: {:.1}x", m.syscall_efficiency_ratio());
}
```

### Linux GSO/GRO

```rust
let binger = BingerUdp::from_std(socket, Config::default())?;

#[cfg(all(target_os = "linux", feature = "gso"))]
binger.set_gso(true)?;

#[cfg(all(target_os = "linux", feature = "gro"))]
binger.set_gro(true)?;

binger.connect(remote_addr)?;  // GSO 需要 connected 模式
```

### select! 并行收发

```rust
let mut send = SendBatch::<32>::new();
let mut recv = RecvBatch::<32>::new(2048);

loop {
    tokio::select! {
        _ = binger.writable() => {
            if !send.is_empty() {
                binger.send_batch(&mut send).await?;
                send.clear();
            }
        }
        _ = binger.readable() => {
            let n = binger.recv_batch(&mut recv).await?;
            for (data, addr) in recv.iter() {
                process(data, addr);
            }
            recv.clear();
        }
    }
}
```

### GSO 分段发送（Linux）

```rust
use udp_binger::{BingerUdp, Config};

let binger = BingerUdp::from_std(socket, Config::default())?;

#[cfg(all(target_os = "linux", feature = "gso"))]
{
    binger.connect(remote_addr)?;       // GSO 需要 connected 模式
    binger.set_gso(true)?;              // 启用 GSO
}

// 一次发送 64KB 数据，内核自动分片为 1460 字节段
#[cfg(all(target_os = "linux", feature = "gso", not(feature = "miri-safe")))]
{
    let large_payload = vec![0xABu8; 64_000];
    let sent = binger.try_send_gso(&large_payload, 1460)?;
    // sent == 64_000 (整个数据报发送成功)
    println!("GSO sent {} bytes", sent);
}
```

### 自适应批量大小

```rust
let config = Config::new()
    .with_batch_size(64)
    .with_adaptive_batching(true);      // 启用自适应

let binger = BingerUdp::from_std(socket, config)?;

// 查询当前推荐 batch size（非自适应时固定返回 32）
let recommended = binger.recommended_batch_size();
println!("recommended batch size: {recommended}");

// 自适应逻辑：当 WouldBlock 率 > 30% 时减半 batch_size；
// 当 WouldBlock 率 < 10% 时增加 50%（上限 1024）。
// 每 100ms 调整一次。
```

### 接收时间戳（Linux）

```rust
// 需 features = ["timestamping"]
#[cfg(all(target_os = "linux", feature = "timestamping"))]
{
    use std::net::UdpSocket;
    use udp_binger::{BingerUdp, RecvBatch, Config};

    let binger = BingerUdp::from_std(
        UdpSocket::bind("0.0.0.0:0")?,
        Config::default(),
    )?;

    binger.enable_timestamping(true)?;

    let mut recv = RecvBatch::<32>::new(2048);
    let n = binger.recv_batch(&mut recv).await?;
    for i in 0..n {
        if let Some(ts) = recv.timestamp(i) {
            let dur = ts.as_duration();
            println!("packet {i} from {} at {dur:?}",
                recv.addr(i));
        }
    }
}
```

### 接收目标地址（Linux）

```rust
// 需 features = ["pktinfo"]
#[cfg(all(target_os = "linux", feature = "pktinfo"))]
{
    use std::net::UdpSocket;
    use udp_binger::{BingerUdp, RecvBatch, Config};

    let binger = BingerUdp::from_std(
        UdpSocket::bind("0.0.0.0:0")?,
        Config::default(),
    )?;

    binger.enable_pktinfo(true)?;

    let mut recv = RecvBatch::<32>::new(2048);
    let n = binger.recv_batch(&mut recv).await?;
    for i in 0..n {
        let src = recv.addr(i);          // 来源地址
        let dst = recv.dst_addr(i);      // 目标地址（本机接口地址）
        println!("{src} -> {dst:?}: {} bytes",
            recv.data(i).len());
    }
}
```

---

## 4. 错误类型

```rust
#[derive(Debug, thiserror::Error)]
pub enum BingerError {
    #[error("batch is full (capacity: {capacity})")]
    BatchFull { capacity: usize },

    #[error("buffer too small: need {required}, have {available}")]
    BufferTooSmall { required: usize, available: usize },

    #[error("feature `{feature}` not available on this platform")]
    UnsupportedFeature { feature: &'static str },

    #[error("{0}")]
    Io(#[from] io::Error),
}
```

---

## 5. Feature Flags

| Feature | 作用 | 默认 |
|---------|------|------|
| `tokio` | 启用 Tokio async 集成 | ✅ |
| `gso` | 启用 `BingerUdp::set_gso()` | ❌ |
| `gro` | 启用 `BingerUdp::set_gro()` | ❌ |
| `busy-poll` | 启用 SO_BUSY_POLL / `set_busy_poll()` | ❌ |
| `pacing` | 启用 SO_MAX_PACING_RATE / `set_pacing_rate()` | ❌ |
| `timestamping` | 启用 SO_TIMESTAMPNS + cmsg 解析，`RecvBatch::timestamp()` + `Timestamp` | ❌ |
| `pktinfo` | 启用 IP_PKTINFO / IPV6_RECVPKTINFO，`RecvBatch::dst_addr()` | ❌ |
| `metrics` | 启用内置指标 | ❌ |
| `miri-safe` | 强制 fallback 路径（纯 safe） | ❌ |

### 运行时检测

以下平台能力通过运行时检测（非 feature flag）启用：

| 平台 | 检测方式 | 能力 |
|------|---------|------|
| macOS | `dlsym(RTLD_DEFAULT, "sendmsg_x")` | sendmsg_x / recvmsg_x |
| Windows | `WSAIoctl(SIO_GET_EXTENSION_FUNCTION_POINTER)` | WSARecvMsg |
| Linux | ENOSYS 处理 | sendmmsg / recvmmsg 自动降级 |

不需要 build.rs — 纯运行时策略，编译一次即可在所有支持的平台上运行。

---

## 6. 与 API.md v1 的变更摘要

| v1 → v2 |
|---------|
| `BingerConfig` 公开字段 + `#[cfg]` 混合 → `Config` builder 模式，平台特异性移到 `BingerUdp` 方法 |
| `BingerSocket` 暗示可能支持 TCP → `BingerUdp` 明确只有 UDP |
| `SendBatch<const N>` 未实现 → 通过 `Deref` 桥接 `SendBatchRaw` |
| `recv_buf_size` 字段名重复 → `recv_buf_size`（per-packet）+ `recv_os_buf_size`（SO_RCVBUF） |
| `mem::forget` 提取 fd → `into_raw_fd()` 或 `tokio::UdpSocket` 持有 |
| 每轮 WouldBlock 重建 tokio socket → 持有 `tokio::UdpSocket` 复用 poll |
| `#[repr(C)]` on RecvSlot → 移除 |
| `PhantomData<*const ()>` → `PhantomData<*const [u8]>` 语义更清晰 |
| `bind` 同步构造 + config 参数 → `from_std` 接收 `std::UdpSocket`（由调用者决定 bind/connect） |

### v0.3 → v0.4 新增

| v0.3 状态 | v0.4 变更 |
|-----------|-----------|
| GSO 仅在 `set_gso` 启用后内嵌于 `send_batch` | → 新增独立 `try_send_gso` / `send_gso` 分段发送接口（`feature = "gso"`, Linux only） |
| 自适应 batching 内部运作不透明 | → 新增 `recommended_batch_size()` 查询当前推荐值 |
| 无 pacing 速率控制 | → 新增 `set_pacing_rate(bytes_per_sec)`（`feature = "pacing"`, Linux only） |
| 无 busy-poll 低延迟模式 | → 新增 `set_busy_poll(usecs)`（`feature = "busy-poll"`, Linux only） |
| 无接收时间戳 | → 新增 `enable_timestamping()` + `RecvBatch::timestamp()` + `Timestamp` 类型（`feature = "timestamping"`, Linux only） |
| 无接收目标地址 | → 新增 `enable_pktinfo()` + `RecvBatch::dst_addr()`（`feature = "pktinfo"`, Linux only） |
| `PlatformCaps` 所有字段无条件暴露 | → macOS/Windows/feature 字段用 `#[cfg]` 保护，跨平台编译更安全 |
