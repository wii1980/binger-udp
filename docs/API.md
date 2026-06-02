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
    /// 批量发送。一次 syscall（Linux: sendmmsg/GSO，macOS: sendmsg_x）。
    /// 返回实际发送的包数。
    pub async fn send_batch(&self, batch: &mut impl DerefMut<Target = SendBatchRaw>) -> io::Result<usize>;

    /// 批量接收。一次 syscall（Linux: recvmmsg，macOS: recvmsg_x）。
    /// 返回实际接收的包数。
    pub async fn recv_batch(&self, batch: &mut impl DerefMut<Target = RecvBatchRaw>) -> io::Result<usize>;

    /// 非阻塞批量发送。
    pub fn try_send_batch(&self, batch: &mut SendBatchRaw) -> io::Result<usize>;

    /// 非阻塞批量接收。
    pub fn try_recv_batch(&self, batch: &mut RecvBatchRaw) -> io::Result<usize>;
}
```

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

    /// 获取底层 fd，供高级用户直接操作。
    pub fn as_raw_fd(&self) -> RawFd;
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
}
```

#### 查询

```rust
impl BingerUdp {
    /// 获取当前平台能力。
    pub fn capabilities(&self) -> PlatformCaps;

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
    pub const fn capacity(&self) -> usize;

    /// 获取第 `idx` 个包的数据。
    pub fn data(&self, idx: usize) -> &[u8];

    /// 获取第 `idx` 个包的来源地址。
    pub fn addr(&self, idx: usize) -> SocketAddr;

    /// 遍历所有接收到的包。
    pub fn iter(&self) -> impl Iterator<Item = (&[u8], SocketAddr)> + '_;

    pub fn clear(&mut self);
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
    pub supports_gso: bool,
    pub supports_gro: bool,
    pub supports_busy_poll: bool,
    pub supports_pacing: bool,
    pub max_batch_size: usize,
    pub backend_name: &'static str,
}

pub fn platform_capabilities() -> PlatformCaps;
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
| `busy-poll` | 启用 SO_BUSY_POLL | ❌ |
| `pacing` | 启用 SO_MAX_PACING_RATE | ❌ |
| `metrics` | 启用内置指标 | ❌ |
| `miri-safe` | 强制 fallback 路径（纯 safe） | ❌ |

---

## 6. 与 API.md v1 的变更摘要

| v1 问题 | v2 修复 |
|---------|---------|
| `BingerConfig` 公开字段 + `#[cfg]` 混合 | → `Config` builder 模式，平台特异性移到 `BingerUdp` 方法 |
| `BingerSocket` 暗示可能支持 TCP | → `BingerUdp` 明确只有 UDP |
| `SendBatch<const N>` 未实现 | → 通过 `Deref` 桥接 `SendBatchRaw` |
| `recv_buf_size` 字段名重复 | → `recv_buf_size`（per-packet）+ `recv_os_buf_size`（SO_RCVBUF） |
| `mem::forget` 提取 fd | → `into_raw_fd()` 或 `tokio::UdpSocket` 持有 |
| 每轮 WouldBlock 重建 tokio socket | → 持有 `tokio::UdpSocket` 复用 poll |
| `#[repr(C)]` on RecvSlot | → 移除 |
| `PhantomData<*const ()>` | → `PhantomData<*const [u8]>` 语义更清晰 |
| `bind` 同步构造 + config 参数 | → `from_std` 接收 `std::UdpSocket`（由调用者决定 bind/connect） |
