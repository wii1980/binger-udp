use crate::sys::{self, Fd};
use std::io;
use std::net::SocketAddr;
#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(all(unix, not(feature = "tokio")))]
use std::os::fd::IntoRawFd;
#[cfg(windows)]
use std::os::windows::io::AsRawSocket;
#[cfg(all(windows, not(feature = "tokio")))]
use std::os::windows::io::IntoRawSocket;

use crate::batch::{RecvBatchRaw, SendBatchRaw};
#[cfg(feature = "metrics")]
use crate::metrics::BingerMetrics;
use crate::sockaddr;

/// Socket configuration for [`BingerUdp`].
///
/// Create a `Config` with [`Config::new()`] or [`Config::default()`], then
/// customize it using the builder-style `with_*` methods.
///
/// # Default values
///
/// | Field | Default |
/// |-------|---------|
/// | `batch_size` | 32 |
/// | `send_buf_size` (kernel `SO_SNDBUF`) | Not set (OS default) |
/// | `recv_os_buf_size` (kernel `SO_RCVBUF`) | Not set (OS default) |
/// | `adaptive_batching` | `false` |
/// | `metrics_enabled` | `false` |
///
/// # Example
///
/// ```rust
/// use binger_udp::Config;
///
/// let config = Config::new()
///     .with_batch_size(64)
///     .with_adaptive_batching(true);
/// ```
#[derive(Debug, Clone)]
pub struct Config {
    pub(crate) batch_size: usize,
    pub(crate) send_buf_size: Option<usize>,
    pub(crate) recv_os_buf_size: Option<usize>,
    pub(crate) adaptive_batching: bool,
    #[cfg(feature = "metrics")]
    pub(crate) metrics_enabled: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            batch_size: 32,
            send_buf_size: None,
            recv_os_buf_size: None,
            adaptive_batching: false,
            #[cfg(feature = "metrics")]
            metrics_enabled: false,
        }
    }
}

impl Config {
    /// Creates a new `Config` with default values.
    ///
    /// Equivalent to [`Config::default()`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the target batch size for [`BingerUdp::send_batch`] and
    /// [`BingerUdp::recv_batch`] operations.
    ///
    /// The actual batch size used may be adjusted if adaptive batching is
    /// enabled via [`Config::with_adaptive_batching`].
    ///
    /// Default: `32`.
    #[must_use]
    pub fn with_batch_size(mut self, n: usize) -> Self {
        self.batch_size = n;
        self
    }

    /// Sets the kernel send buffer size (`SO_SNDBUF`) for the socket.
    ///
    /// This is an OS-level socket option that controls how much data the kernel
    /// buffers for sending. Larger values can improve throughput at the cost
    /// of memory.
    ///
    /// Default: `None` (OS default is used).
    #[must_use]
    pub fn with_send_buf_size(mut self, n: usize) -> Self {
        self.send_buf_size = Some(n);
        self
    }

    /// Sets the kernel receive buffer size (`SO_RCVBUF`) for the socket.
    ///
    /// This is an OS-level socket option. The kernel may double the requested
    /// value. Useful for high-throughput receivers that want to avoid packet
    /// drops in the kernel.
    ///
    /// Default: `None` (OS default is used).
    #[must_use]
    pub fn with_recv_os_buf_size(mut self, n: usize) -> Self {
        self.recv_os_buf_size = Some(n);
        self
    }

    /// Enables or disables adaptive batching.
    ///
    /// When enabled, the effective batch size is dynamically adjusted based on
    /// the rate of `WouldBlock` events. The batch size is reduced when the
    /// `WouldBlock` rate exceeds 30%, and increased when it drops below 10%.
    /// Adjustments occur at most every 100 ms.
    ///
    /// Default: `false`.
    ///
    /// # Related
    ///
    /// * [`BingerUdp::recommended_batch_size`] — queries the current
    ///   effective batch size.
    #[must_use]
    pub fn with_adaptive_batching(mut self, enabled: bool) -> Self {
        self.adaptive_batching = enabled;
        self
    }

    /// Enables or disables built-in metrics collection.
    ///
    /// When enabled, [`BingerUdp::metrics`] returns a reference to the
    /// [`BingerMetrics`] instance with atomic counters for packets
    /// sent/received, batch operations, syscalls, and error events.
    ///
    /// Default: `false`.
    ///
    /// Only available with the `metrics` feature.
    #[cfg(feature = "metrics")]
    #[must_use]
    pub fn with_metrics(mut self, enabled: bool) -> Self {
        self.metrics_enabled = enabled;
        self
    }
}

/// Reports which platform-optimized syscalls and features are available
/// at compile time.
///
/// `PlatformCaps` is obtained via [`platform_capabilities()`] or
/// [`BingerUdp::capabilities()`]. Use it to dynamically select code paths
/// or to log which backend is active.
///
/// Some fields are conditionally compiled:
///
/// | Field | Platform / Feature |
/// |-------|--------------------|
/// | `supports_sendmsg_x`, `supports_recvmsg_x` | `target_os = "macos"` |
/// | `supports_wsa_send_msg`, `supports_wsa_recv_msg` | `target_os = "windows"` |
/// | `supports_timestamping` | `feature = "timestamping"` |
/// | `supports_pktinfo` | `feature = "pktinfo"` |
///
/// # Example
///
/// ```rust
/// use binger_udp::platform_capabilities;
///
/// let caps = platform_capabilities();
/// println!("Backend: {}", caps.backend_name);
/// ```
#[allow(clippy::struct_excessive_bools)]
pub struct PlatformCaps {
    /// Whether `sendmmsg` is available (Linux only).
    pub supports_sendmmsg: bool,
    /// Whether `recvmmsg` is available (Linux only).
    pub supports_recvmmsg: bool,
    /// Whether `sendmsg_x` is available (macOS only, runtime-detected via dlsym).
    #[cfg(target_os = "macos")]
    pub supports_sendmsg_x: bool,
    /// Whether `recvmsg_x` is available (macOS only, runtime-detected via dlsym).
    #[cfg(target_os = "macos")]
    pub supports_recvmsg_x: bool,
    /// Whether `WSASendMsg` is available (Windows only, runtime-detected via `WSAIoctl`).
    #[cfg(target_os = "windows")]
    pub supports_wsa_send_msg: bool,
    /// Whether `WSARecvMsg` is available (Windows only, runtime-detected via `WSAIoctl`).
    #[cfg(target_os = "windows")]
    pub supports_wsa_recv_msg: bool,
    /// Whether Generic Segmentation Offload (GSO) is available (Linux, requires `gso` feature).
    pub supports_gso: bool,
    /// Whether Generic Receive Offload (GRO) is available (Linux, requires `gro` feature).
    pub supports_gro: bool,
    /// Whether `SO_BUSY_POLL` is available (Linux, requires `busy-poll` feature).
    pub supports_busy_poll: bool,
    /// Whether `SO_MAX_PACING_RATE` is available (Linux, requires `pacing` feature).
    pub supports_pacing: bool,
    /// Whether kernel timestamping is available (Linux, requires `timestamping` feature).
    #[cfg(feature = "timestamping")]
    pub supports_timestamping: bool,
    /// Whether `IP_PKTINFO` / `IPV6_RECVPKTINFO` is available (Linux, requires `pktinfo` feature).
    #[cfg(feature = "pktinfo")]
    pub supports_pktinfo: bool,
    /// Maximum batch size supported by the backend.
    ///
    /// Linux backends typically support up to 1024; other platforms up to 32.
    pub max_batch_size: usize,
    /// Human-readable name of the active backend.
    ///
    /// Examples: `"sendmmsg/recvmmsg (Linux)"`, `"fallback (loop sendto/recvfrom)"`.
    pub backend_name: &'static str,
}

/// Returns the [`PlatformCaps`] for the current platform and enabled features.
///
/// This function is stateless — it returns compile-time constants that reflect
/// the target OS and the feature flags the crate was built with.
///
/// # Example
///
/// ```rust
/// use binger_udp::platform_capabilities;
///
/// let caps = platform_capabilities();
/// assert!(!caps.backend_name.is_empty());
/// ```
#[must_use]
pub fn platform_capabilities() -> PlatformCaps {
    PlatformCaps {
        supports_sendmmsg: cfg!(target_os = "linux"),
        supports_recvmmsg: cfg!(target_os = "linux"),
        #[cfg(target_os = "macos")]
        supports_sendmsg_x: true,
        #[cfg(target_os = "macos")]
        supports_recvmsg_x: true,
        #[cfg(target_os = "windows")]
        supports_wsa_send_msg: true,
        #[cfg(target_os = "windows")]
        supports_wsa_recv_msg: true,
        supports_gso: cfg!(all(target_os = "linux", feature = "gso")),
        supports_gro: cfg!(all(target_os = "linux", feature = "gro")),
        supports_busy_poll: cfg!(all(target_os = "linux", feature = "busy-poll")),
        supports_pacing: cfg!(all(target_os = "linux", feature = "pacing")),
        #[cfg(feature = "timestamping")]
        supports_timestamping: cfg!(all(target_os = "linux", feature = "timestamping")),
        #[cfg(feature = "pktinfo")]
        supports_pktinfo: cfg!(all(target_os = "linux", feature = "pktinfo")),
        max_batch_size: if cfg!(target_os = "linux") { 1024 } else { 32 },
        backend_name: backends(),
    }
}

const fn backends() -> &'static str {
    #[cfg(target_os = "linux")]
    {
        "sendmmsg/recvmmsg (Linux)"
    }
    #[cfg(target_os = "macos")]
    {
        "sendmsg_x/recvmsg_x (macOS)"
    }
    #[cfg(target_os = "windows")]
    {
        "WSASendMsg/WSARecvMsg (Windows)"
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        "fallback (loop sendto/recvfrom)"
    }
}

struct AdaptiveState {
    target_size: usize,
    would_block_count: u64,
    total_send_count: u64,
    last_adjustment: std::time::Instant,
}

impl AdaptiveState {
    const MIN_BATCH: usize = 1;
    const MAX_BATCH: usize = 1024;
    const ADJUSTMENT_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

    fn new(initial: usize) -> Self {
        Self {
            target_size: initial.clamp(Self::MIN_BATCH, Self::MAX_BATCH),
            would_block_count: 0,
            total_send_count: 0,
            last_adjustment: std::time::Instant::now(),
        }
    }

    fn record_would_block(&mut self) {
        self.would_block_count += 1;
        self.total_send_count += 1;
    }

    fn record_event(&mut self) {
        self.total_send_count += 1;
    }

    #[allow(clippy::cast_precision_loss)]
    fn maybe_adjust(&mut self) {
        if self.last_adjustment.elapsed() < Self::ADJUSTMENT_INTERVAL {
            return;
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
        self.last_adjustment = std::time::Instant::now();
    }

    fn recommended_size(&self) -> usize {
        self.target_size
    }
}

/// A batch-native UDP socket that automatically selects the most efficient
/// platform syscall for send and receive operations.
///
/// # Platform backends
///
/// | Platform | Send backend | Recv backend |
/// |----------|-------------|--------------|
/// | Linux (connected) | `sendmsg` with GSO | `recvmmsg` with GRO |
/// | Linux (multi-dest) | `sendmmsg` | `recvmmsg` |
/// | macOS | `sendmsg_x` (via dlsym) | `recvmsg_x` (via dlsym) |
/// | Windows | `WSASendMsg` (via `WSAIoctl`) | `WSARecvMsg` (via `WSAIoctl`) |
/// | Fallback | `sendto` (loop) | `recvfrom` (loop) |
///
/// # Async support
///
/// With the default `tokio` feature, all `async` methods use Tokio's
/// readiness-based I/O ([`tokio::net::UdpSocket::readable`] /
/// [`tokio::net::UdpSocket::writable`]) without busy-looping.
///
/// # Example
///
/// ```rust,no_run
/// use binger_udp::{BingerUdp, SendBatch, RecvBatch, Config};
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let socket = BingerUdp::from_std(
///     std::net::UdpSocket::bind("0.0.0.0:0")?,
///     Config::default(),
/// )?;
///
/// let mut send = SendBatch::<32>::new();
/// send.push(b"hello", "192.168.1.1:8080".parse().unwrap())?;
/// socket.send_batch(&mut send).await?;
/// # Ok(())
/// # }
/// ```
pub struct BingerUdp {
    fd: Fd,
    #[cfg(feature = "tokio")]
    tokio_sock: tokio::net::UdpSocket,
    #[cfg(feature = "metrics")]
    metrics: Option<BingerMetrics>,
    adaptive_send: Option<std::sync::Mutex<AdaptiveState>>,
    adaptive_recv: Option<std::sync::Mutex<AdaptiveState>>,
}

impl BingerUdp {
    /// Get raw socket handle from a borrowed `UdpSocket`.
    fn raw_fd_std(socket: &std::net::UdpSocket) -> Fd {
        #[cfg(unix)]
        {
            socket.as_raw_fd()
        }
        #[cfg(windows)]
        {
            socket.as_raw_socket() as Fd
        }
    }

    /// Get raw socket handle from an owned `UdpSocket` (consumes it).
    #[cfg(not(feature = "tokio"))]
    fn raw_fd_std_owned(socket: std::net::UdpSocket) -> Fd {
        #[cfg(unix)]
        {
            socket.into_raw_fd()
        }
        #[cfg(windows)]
        {
            socket.into_raw_socket() as Fd
        }
    }

    /// Creates a [`BingerUdp`] from a standard [`std::net::UdpSocket`] and
    /// [`Config`].
    ///
    /// The socket is set to non-blocking mode. OS-level buffer sizes specified
    /// in the config (`SO_SNDBUF`, `SO_RCVBUF`) are applied before constructing
    /// the wrapper.
    ///
    /// With the default `tokio` feature, the socket is converted into a
    /// [`tokio::net::UdpSocket`] for async readiness-based I/O.
    ///
    /// This is the primary (and only) way to create a `BingerUdp` instance.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if:
    /// * Setting the socket to non-blocking fails.
    /// * Setting OS socket buffer sizes fails.
    /// * Converting to a tokio socket fails (with `tokio` feature).
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use binger_udp::{BingerUdp, Config};
    ///
    /// let socket = BingerUdp::from_std(
    ///     std::net::UdpSocket::bind("0.0.0.0:0").unwrap(),
    ///     Config::default(),
    /// ).unwrap();
    /// ```
    #[allow(clippy::needless_pass_by_value)]
    pub fn from_std(socket: std::net::UdpSocket, config: Config) -> io::Result<Self> {
        socket.set_nonblocking(true)?;

        if let Some(size) = config.send_buf_size {
            sockaddr::raw_setsockopt(
                Self::raw_fd_std(&socket),
                sys::SOL_SOCKET,
                sys::SO_SNDBUF,
                size as libc::c_int,
            )?;
        }
        if let Some(size) = config.recv_os_buf_size {
            sockaddr::raw_setsockopt(
                Self::raw_fd_std(&socket),
                sys::SOL_SOCKET,
                sys::SO_RCVBUF,
                size as libc::c_int,
            )?;
        }

        #[cfg(feature = "tokio")]
        let fd = Self::raw_fd_std(&socket);
        #[cfg(feature = "tokio")]
        let tokio_sock = tokio::net::UdpSocket::from_std(socket)?;

        #[cfg(not(feature = "tokio"))]
        let fd = Self::raw_fd_std_owned(socket);

        Ok(Self {
            fd,
            #[cfg(feature = "tokio")]
            tokio_sock,
            #[cfg(feature = "metrics")]
            metrics: if config.metrics_enabled {
                Some(BingerMetrics::default())
            } else {
                None
            },
            adaptive_send: if config.adaptive_batching {
                Some(std::sync::Mutex::new(AdaptiveState::new(config.batch_size)))
            } else {
                None
            },
            adaptive_recv: if config.adaptive_batching {
                Some(std::sync::Mutex::new(AdaptiveState::new(config.batch_size)))
            } else {
                None
            },
        })
    }

    /// Sends all packets in the batch, retrying on `WouldBlock`.
    ///
    /// This is the primary send API. It calls [`BingerUdp::try_send_batch`] in a
    /// loop, waiting for the socket to become writable whenever the kernel
    /// returns `WouldBlock`.
    ///
    /// The `batch` argument can be a [`SendBatch<N>`](crate::batch::SendBatch)
    /// or [`SendBatchRaw`] (the former dereferences via [`std::ops::DerefMut`]).
    ///
    /// Returns the total number of packets sent on success (always equal to
    /// `batch.len()` since retries are transparent).
    ///
    /// # Errors
    ///
    /// Returns the underlying I/O error on failure. `WouldBlock` is handled
    /// internally and never returned to the caller.
    ///
    /// # Related
    ///
    /// * [`BingerUdp::try_send_batch`] — non-blocking variant.
    /// * [`BingerUdp::send_to`] — single-packet convenience wrapper.
    pub async fn send_batch(&self, batch: &mut SendBatchRaw) -> io::Result<usize> {
        loop {
            match self.try_send_batch(batch) {
                Ok(n) => return Ok(n),
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    if let Some(ref state) = self.adaptive_send {
                        if let Ok(mut s) = state.lock() {
                            s.record_would_block();
                            s.maybe_adjust();
                        }
                    }
                    self.wait_writable().await?;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Receives a batch of packets, retrying on `WouldBlock`.
    ///
    /// This is the primary receive API. It calls [`BingerUdp::try_recv_batch`]
    /// in a loop, waiting for the socket to become readable whenever the kernel
    /// returns `WouldBlock`.
    ///
    /// The `batch` argument can be a [`RecvBatch<N>`](crate::batch::RecvBatch)
    /// or [`RecvBatchRaw`] (the former dereferences via [`std::ops::DerefMut`]).
    ///
    /// Returns the number of packets actually received (may be less than
    /// `batch.capacity()`).
    ///
    /// # Errors
    ///
    /// Returns the underlying I/O error on failure. `WouldBlock` is handled
    /// internally and never returned to the caller.
    ///
    /// # Related
    ///
    /// * [`BingerUdp::try_recv_batch`] — non-blocking variant.
    /// * [`BingerUdp::recv_from`] — single-packet convenience wrapper.
    pub async fn recv_batch(&self, batch: &mut RecvBatchRaw) -> io::Result<usize> {
        loop {
            match self.try_recv_batch(batch) {
                Ok(0) => {
                    if let Some(ref state) = self.adaptive_recv {
                        if let Ok(mut s) = state.lock() {
                            s.record_would_block();
                            s.maybe_adjust();
                        }
                    }
                    self.wait_readable().await?;
                }
                Ok(n) => return Ok(n),
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    if let Some(ref state) = self.adaptive_recv {
                        if let Ok(mut s) = state.lock() {
                            s.record_would_block();
                            s.maybe_adjust();
                        }
                    }
                    self.wait_readable().await?;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Attempts to send a batch of packets without retrying on `WouldBlock`.
    ///
    /// Unlike [`BingerUdp::send_batch`], this method returns
    /// [`io::ErrorKind::WouldBlock`] immediately if the socket is not ready
    /// for writing. This is useful for integrating with custom event loops or
    /// `select!`-based concurrency.
    ///
    /// The `batch` argument can be a [`SendBatch<N>`](crate::batch::SendBatch)
    /// or [`SendBatchRaw`].
    ///
    /// Returns the number of packets actually sent (may be less than
    /// `batch.len()` on a partial write, or 0 if `WouldBlock`).
    ///
    /// # Errors
    ///
    /// Returns the underlying I/O error on failure, including `WouldBlock`.
    ///
    /// # Related
    ///
    /// * [`BingerUdp::send_batch`] — retry-on-WouldBlock variant.
    pub fn try_send_batch(&self, batch: &mut SendBatchRaw) -> io::Result<usize> {
        #[cfg(not(feature = "metrics"))]
        let sent = crate::platform::try_send_batch(self.fd, batch)?;
        #[cfg(feature = "metrics")]
        let sent = crate::platform::try_send_batch(self.fd, batch).map_err(|e| {
            if let Some(ref m) = self.metrics {
                m.inc_send_errors();
                if e.kind() == io::ErrorKind::WouldBlock {
                    m.inc_send_would_block();
                }
            }
            e
        })?;

        if let Some(ref state) = self.adaptive_send {
            if let Ok(mut s) = state.lock() {
                s.record_event();
            }
        }

        #[cfg(feature = "metrics")]
        if let Some(ref m) = self.metrics {
            m.inc_packets_sent(sent as u64);
            m.inc_batches_sent();
            m.inc_send_syscalls();
        }

        Ok(sent)
    }

    /// Attempts to receive a batch of packets without retrying on `WouldBlock`.
    ///
    /// Unlike [`BingerUdp::recv_batch`], this method returns
    /// [`io::ErrorKind::WouldBlock`] immediately if the socket is not ready
    /// for reading.
    ///
    /// The `batch` argument can be a [`RecvBatch<N>`](crate::batch::RecvBatch)
    /// or [`RecvBatchRaw`].
    ///
    /// Returns the number of packets actually received (0 if `WouldBlock`).
    ///
    /// # Errors
    ///
    /// Returns the underlying I/O error on failure, including `WouldBlock`.
    ///
    /// # Related
    ///
    /// * [`BingerUdp::recv_batch`] — retry-on-WouldBlock variant.
    pub fn try_recv_batch(&self, batch: &mut RecvBatchRaw) -> io::Result<usize> {
        #[cfg(not(feature = "metrics"))]
        let received = crate::platform::try_recv_batch(self.fd, batch)?;
        #[cfg(feature = "metrics")]
        let received = crate::platform::try_recv_batch(self.fd, batch).map_err(|e| {
            if let Some(ref m) = self.metrics {
                m.inc_recv_errors();
                if e.kind() == io::ErrorKind::WouldBlock {
                    m.inc_recv_would_block();
                }
            }
            e
        })?;

        if let Some(ref state) = self.adaptive_recv {
            if let Ok(mut s) = state.lock() {
                s.record_event();
            }
        }

        #[cfg(feature = "metrics")]
        if let Some(ref m) = self.metrics {
            m.inc_packets_received(received as u64);
            m.inc_batches_received();
            m.inc_recv_syscalls();
        }

        Ok(received)
    }

    /// Sends a single UDP datagram to the specified address, retrying on
    /// `WouldBlock`.
    ///
    /// This is a convenience wrapper around [`BingerUdp::send_batch`] that
    /// constructs a single-element batch internally.
    ///
    /// # Errors
    ///
    /// Returns the underlying I/O error on failure.
    ///
    /// # Panics
    ///
    /// Panics if the internal single-element batch capacity is exceeded, which
    /// should never happen.
    ///
    /// # Related
    ///
    /// * [`BingerUdp::send_batch`] — batch variant for higher throughput.
    /// * [`BingerUdp::try_send_to`] — non-blocking single-packet variant.
    pub async fn send_to(&self, buf: &[u8], addr: SocketAddr) -> io::Result<usize> {
        let mut batch = SendBatchRaw::with_capacity(1);
        batch.push(buf, Some(addr)).expect("capacity 1");
        self.send_batch(&mut batch).await?;
        Ok(buf.len())
    }

    /// Receives a single UDP datagram into the buffer, retrying on `WouldBlock`.
    ///
    /// This is a convenience wrapper around [`BingerUdp::recv_batch`] that
    /// constructs a single-element batch internally.
    ///
    /// Returns the number of bytes received and the source address.
    ///
    /// # Errors
    ///
    /// Returns the underlying I/O error on failure.
    ///
    /// # Related
    ///
    /// * [`BingerUdp::recv_batch`] — batch variant for higher throughput.
    pub async fn recv_from(&self, buf: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        let mut batch = RecvBatchRaw::with_capacity(1, buf.len());
        self.recv_batch(&mut batch).await?;
        let data = batch.data(0);
        let addr = batch.addr(0);
        let len = data.len().min(buf.len());
        buf[..len].copy_from_slice(&data[..len]);
        Ok((len, addr))
    }

    /// Attempts to send a single UDP datagram without retrying on `WouldBlock`.
    ///
    /// This is a convenience wrapper around [`BingerUdp::try_send_batch`] that
    /// constructs a single-element batch internally.
    ///
    /// # Errors
    ///
    /// Returns the underlying I/O error on failure, including `WouldBlock`.
    ///
    /// # Panics
    ///
    /// Panics if the internal single-element batch capacity is exceeded, which
    /// should never happen.
    ///
    /// # Related
    ///
    /// * [`BingerUdp::send_to`] — retry-on-WouldBlock variant.
    /// * [`BingerUdp::try_send_batch`] — batch variant.
    pub fn try_send_to(&self, buf: &[u8], addr: SocketAddr) -> io::Result<usize> {
        let mut batch = SendBatchRaw::with_capacity(1);
        batch.push(buf, Some(addr)).expect("capacity 1");
        self.try_send_batch(&mut batch)?;
        Ok(buf.len())
    }

    /// Connects the UDP socket to a remote address.
    ///
    /// A connected UDP socket can only send to and receive from that address.
    /// This is required for GSO (Generic Segmentation Offload) on Linux.
    ///
    /// # Errors
    ///
    /// Returns the underlying I/O error on failure (e.g., invalid address).
    pub fn connect(&self, addr: SocketAddr) -> io::Result<()> {
        sockaddr::raw_connect(self.fd, addr)
    }

    /// Returns the local socket address.
    ///
    /// # Errors
    ///
    /// Returns the underlying IO error on failure.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        sockaddr::raw_getsockname(self.fd)
    }

    /// Returns the TTL of the socket.
    ///
    /// # Errors
    ///
    /// Returns the underlying IO error on failure.
    pub fn ttl(&self) -> io::Result<u32> {
        sockaddr::raw_getsockopt(self.fd, sys::IPPROTO_IP, sys::IP_TTL).map(|v| v as u32)
    }

    /// Sets the TTL of the socket.
    ///
    /// # Errors
    ///
    /// Returns the underlying IO error on failure.
    pub fn set_ttl(&self, ttl: u32) -> io::Result<()> {
        sockaddr::raw_setsockopt(self.fd, sys::IPPROTO_IP, sys::IP_TTL, ttl as libc::c_int)
    }

    /// Returns the underlying file descriptor (or socket handle on Windows).
    ///
    /// On Unix, this is a `RawFd` (`c_int`). On Windows, this is a `SOCKET`
    /// (`usize`).
    ///
    /// Use this to pass the socket to low-level system calls or integration
    /// with other I/O libraries.
    #[must_use]
    pub fn as_raw_fd(&self) -> Fd {
        self.fd
    }

    /// Enables or disables Generic Segmentation Offload (GSO).
    ///
    /// GSO allows the kernel to segment a large UDP datagram into MTU-sized
    /// segments, reducing the number of user-to-kernel transitions. Requires a
    /// connected socket (see [`BingerUdp::connect`]).
    ///
    /// Only available on Linux with the `gso` feature.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the setsockopt call fails.
    #[cfg(all(target_os = "linux", feature = "gso"))]
    pub fn set_gso(&self, enabled: bool) -> io::Result<()> {
        sockaddr::raw_setsockopt(
            self.fd,
            libc::IPPROTO_UDP,
            libc::UDP_SEGMENT,
            i32::from(enabled),
        )
    }

    /// Enables or disables Generic Receive Offload (GRO).
    ///
    /// GRO allows the kernel to coalesce multiple incoming UDP datagrams into
    /// a single large buffer, reducing the number of receive syscalls.
    ///
    /// Only available on Linux with the `gro` feature.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the setsockopt call fails.
    #[cfg(all(target_os = "linux", feature = "gro"))]
    pub fn set_gro(&self, enabled: bool) -> io::Result<()> {
        sockaddr::raw_setsockopt(
            self.fd,
            libc::IPPROTO_UDP,
            libc::UDP_GRO,
            i32::from(enabled),
        )
    }

    /// Sends a large datagram segmented by GSO in a single syscall
    /// (non-blocking).
    ///
    /// The kernel fragments `data` into segments of `segment_size` bytes each
    /// (the last segment may be smaller). The socket must be connected (see
    /// [`BingerUdp::connect`]) and GSO must be enabled via
    /// [`BingerUdp::set_gso(true)`](BingerUdp::set_gso).
    ///
    /// Only available on Linux with the `gso` feature and without `miri-safe`.
    ///
    /// Returns the total number of bytes sent (the entire `data` payload on
    /// success).
    ///
    /// # Errors
    ///
    /// Returns the underlying I/O error on failure, including `WouldBlock`.
    #[cfg(all(target_os = "linux", feature = "gso", not(feature = "miri-safe")))]
    pub fn try_send_gso(&self, data: &[u8], segment_size: u16) -> io::Result<usize> {
        crate::platform::try_send_gso(self.fd, data, segment_size)
    }

    /// Sends a large datagram segmented by GSO, retrying on `WouldBlock`.
    ///
    /// Calls [`BingerUdp::try_send_gso`] in a loop, waiting for the socket
    /// to become writable on `WouldBlock`.
    ///
    /// Only available on Linux with the `gso` feature and without `miri-safe`.
    ///
    /// # Errors
    ///
    /// Returns the underlying I/O error on failure. `WouldBlock` is handled
    /// internally.
    #[cfg(all(target_os = "linux", feature = "gso", not(feature = "miri-safe")))]
    pub async fn send_gso(&self, data: &[u8], segment_size: u16) -> io::Result<usize> {
        loop {
            match self.try_send_gso(data, segment_size) {
                Ok(n) => return Ok(n),
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    self.wait_writable().await?;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Sets the maximum pacing rate for UDP sends.
    ///
    /// `bytes_per_sec` is the rate in bytes per second. Set to `0` to disable
    /// pacing. Requires the `fq` qdisc (fair queueing) on the egress path.
    ///
    /// Only available on Linux with the `pacing` feature.
    ///
    /// # Errors
    ///
    /// Returns the underlying I/O error if setsockopt fails.
    #[cfg(all(target_os = "linux", feature = "pacing"))]
    pub fn set_pacing_rate(&self, bytes_per_sec: u32) -> io::Result<()> {
        sockaddr::raw_setsockopt_u32(
            self.fd,
            libc::SOL_SOCKET,
            crate::sys::SO_MAX_PACING_RATE,
            bytes_per_sec,
        )
    }

    /// Enables busy-polling for ultra-low latency (`SO_BUSY_POLL`).
    ///
    /// `usecs` is the time in microseconds to busy-poll. Higher values reduce
    /// latency at the cost of CPU usage. Requires `CAP_NET_ADMIN` or the
    /// `net.core.busy_poll` sysctl to be configured globally.
    ///
    /// Only available on Linux with the `busy-poll` feature.
    ///
    /// # Errors
    ///
    /// Returns the underlying I/O error if setsockopt fails.
    #[cfg(all(target_os = "linux", feature = "busy-poll"))]
    pub fn set_busy_poll(&self, usecs: u32) -> io::Result<()> {
        sockaddr::raw_setsockopt(
            self.fd,
            libc::SOL_SOCKET,
            crate::sys::SO_BUSY_POLL,
            usecs as libc::c_int,
        )
    }

    /// Enables software receive timestamping (`SO_TIMESTAMPNS`).
    ///
    /// When enabled, each received packet carries a kernel timestamp
    /// accessible via [`RecvBatch::timestamp`](crate::batch::RecvBatch::timestamp).
    ///
    /// # Errors
    ///
    /// Returns the underlying IO error if setsockopt fails.
    #[cfg(all(target_os = "linux", feature = "timestamping"))]
    pub fn enable_timestamping(&self, enabled: bool) -> io::Result<()> {
        sockaddr::raw_setsockopt(
            self.fd,
            libc::SOL_SOCKET,
            crate::sys::SO_TIMESTAMPNS,
            i32::from(enabled),
        )
    }

    /// Enables `IP_PKTINFO` / `IPV6_RECVPKTINFO` for receiving destination addresses.
    ///
    /// When enabled, each received packet's destination address is available
    /// via [`RecvBatch::dst_addr`](crate::batch::RecvBatch::dst_addr).
    ///
    /// # Errors
    ///
    /// Returns the underlying IO error if setsockopt fails.
    #[cfg(all(target_os = "linux", feature = "pktinfo"))]
    pub fn enable_pktinfo(&self, enabled: bool) -> io::Result<()> {
        sockaddr::raw_setsockopt(
            self.fd,
            libc::IPPROTO_IP,
            libc::IP_PKTINFO,
            i32::from(enabled),
        )?;
        sockaddr::raw_setsockopt(
            self.fd,
            libc::IPPROTO_IPV6,
            libc::IPV6_RECVPKTINFO,
            i32::from(enabled),
        )
    }

    /// Returns the [`PlatformCaps`] for the current platform and enabled features.
    ///
    /// This is a shorthand for calling [`platform_capabilities()`] directly.
    #[must_use]
    pub fn capabilities(&self) -> PlatformCaps {
        platform_capabilities()
    }

    /// Returns the recommended batch size based on adaptive batching state.
    ///
    /// When adaptive batching is enabled via
    /// [`Config::with_adaptive_batching`], this returns the dynamically
    /// adjusted batch size (pass `true` at construction time). The adjustment happens at most every 100 ms:
    ///
    /// * When the `WouldBlock` rate exceeds 30%, the batch size is halved.
    /// * When the rate drops below 10%, the batch size is increased by 50%.
    /// * The batch size is clamped to the range `[1, 1024]`.
    ///
    /// If adaptive batching is disabled, returns a fixed default of `32`.
    ///
    /// # Panics
    #[must_use]
    pub fn recommended_batch_size(&self) -> usize {
        self.adaptive_send
            .as_ref()
            .and_then(|s| s.lock().ok())
            .map_or(32, |g| g.recommended_size())
    }

    /// Returns a reference to the [`BingerMetrics`] instance, if metrics
    /// collection is enabled.
    ///
    /// Metrics must be enabled at construction time via
    /// [`Config::with_metrics`]. This requires the `metrics` feature.
    ///
    /// Returns `None` if metrics are disabled.
    #[cfg(feature = "metrics")]
    #[must_use]
    pub fn metrics(&self) -> Option<&BingerMetrics> {
        self.metrics.as_ref()
    }

    /// Waits until the socket becomes readable.
    ///
    /// This is a direct wrapper around
    /// [`tokio::net::UdpSocket::readable()`]. Use this in `tokio::select!`
    /// to be notified when data is available for
    /// [`BingerUdp::recv_batch`] or [`BingerUdp::recv_from`].
    ///
    /// Only available with the default `tokio` feature.
    ///
    /// # Errors
    ///
    /// Returns the underlying I/O error on failure.
    #[cfg(feature = "tokio")]
    pub async fn readable(&self) -> io::Result<()> {
        self.tokio_sock.readable().await
    }

    /// Waits until the socket becomes writable.
    ///
    /// This is a direct wrapper around
    /// [`tokio::net::UdpSocket::writable()`]. Use this in `tokio::select!`
    /// to be notified when the socket can accept data for
    /// [`BingerUdp::send_batch`] or [`BingerUdp::send_to`].
    ///
    /// Only available with the default `tokio` feature.
    ///
    /// # Errors
    ///
    /// Returns the underlying I/O error on failure.
    #[cfg(feature = "tokio")]
    pub async fn writable(&self) -> io::Result<()> {
        self.tokio_sock.writable().await
    }

    #[cfg(feature = "tokio")]
    async fn wait_writable(&self) -> io::Result<()> {
        self.tokio_sock.writable().await?;
        Ok(())
    }

    #[cfg(feature = "tokio")]
    async fn wait_readable(&self) -> io::Result<()> {
        self.tokio_sock.readable().await?;
        Ok(())
    }

    #[cfg(not(feature = "tokio"))]
    async fn wait_writable(&self) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Other,
            "tokio feature disabled",
        ))
    }

    #[cfg(not(feature = "tokio"))]
    async fn wait_readable(&self) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Other,
            "tokio feature disabled",
        ))
    }
}

#[cfg(not(feature = "tokio"))]
impl Drop for BingerUdp {
    /// Closes the underlying file descriptor.
    fn drop(&mut self) {
        sys::close_fd(self.fd);
    }
}

// SAFETY: BingerUdp wraps a raw fd and optionally a tokio UdpSocket.
// Both are safe to send/share across threads — the fd is used with
// thread-safe syscalls, and tokio::net::UdpSocket is Send + Sync.
unsafe impl Send for BingerUdp {}
unsafe impl Sync for BingerUdp {}
