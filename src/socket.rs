use std::io;
use std::net::{SocketAddr, ToSocketAddrs};
use std::os::fd::RawFd;

use crate::batch::{RecvBatchRaw, SendBatchRaw};
#[cfg(feature = "metrics")]
use crate::metrics::BingerMetrics;

#[derive(Debug, Clone)]
pub struct Config {
    pub(crate) batch_size: usize,
    pub(crate) recv_buf_size: usize,
    pub(crate) send_buf_size: Option<usize>,
    pub(crate) recv_os_buf_size: Option<usize>,
    pub(crate) adaptive_batching: bool,
    pub(crate) metrics_enabled: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            batch_size: 32,
            recv_buf_size: 2048,
            send_buf_size: None,
            recv_os_buf_size: None,
            adaptive_batching: false,
            metrics_enabled: false,
        }
    }
}

impl Config {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_batch_size(mut self, n: usize) -> Self {
        self.batch_size = n;
        self
    }

    pub fn with_recv_buf_size(mut self, n: usize) -> Self {
        self.recv_buf_size = n;
        self
    }

    pub fn with_send_buf_size(mut self, n: usize) -> Self {
        self.send_buf_size = Some(n);
        self
    }

    pub fn with_recv_os_buf_size(mut self, n: usize) -> Self {
        self.recv_os_buf_size = Some(n);
        self
    }

    pub fn with_adaptive_batching(mut self, enabled: bool) -> Self {
        self.adaptive_batching = enabled;
        self
    }

    #[cfg(feature = "metrics")]
    pub fn with_metrics(mut self, enabled: bool) -> Self {
        self.metrics_enabled = enabled;
        self
    }
}

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

pub fn platform_capabilities() -> PlatformCaps {
    PlatformCaps {
        supports_sendmmsg: cfg!(target_os = "linux"),
        supports_recvmmsg: cfg!(target_os = "linux"),
        supports_gso: cfg!(all(target_os = "linux", feature = "gso")),
        supports_gro: cfg!(all(target_os = "linux", feature = "gro")),
        supports_busy_poll: cfg!(all(target_os = "linux", feature = "busy-poll")),
        supports_pacing: cfg!(all(target_os = "linux", feature = "pacing")),
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

pub struct BingerUdp {
    fd: RawFd,
    #[cfg(feature = "tokio")]
    tokio_sock: tokio::net::UdpSocket,
    #[cfg(feature = "metrics")]
    metrics: Option<BingerMetrics>,
    adaptive_batching: bool,
}

impl BingerUdp {
    pub fn from_std(socket: std::net::UdpSocket, config: Config) -> io::Result<Self> {
        socket.set_nonblocking(true)?;

        if let Some(size) = config.send_buf_size {
            set_sockopt_int(&socket, libc::SOL_SOCKET, libc::SO_SNDBUF, size as libc::c_int)?;
        }
        if let Some(size) = config.recv_os_buf_size {
            set_sockopt_int(&socket, libc::SOL_SOCKET, libc::SO_RCVBUF, size as libc::c_int)?;
        }

        #[cfg(feature = "tokio")]
        let fd = socket.as_raw_fd();

        #[cfg(feature = "tokio")]
        let tokio_sock = tokio::net::UdpSocket::from_std(socket)?;

        #[cfg(not(feature = "tokio"))]
        let fd = socket.into_raw_fd();
        #[cfg(not(feature = "tokio"))]
        let _ = socket;

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
            adaptive_batching: config.adaptive_batching,
        })
    }

    pub async fn send_batch(&self, batch: &mut SendBatchRaw) -> io::Result<usize> {
        loop {
            match self.try_send_batch(batch) {
                Ok(n) => return Ok(n),
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
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
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    self.wait_readable().await?;
                }
                Err(e) => return Err(e),
            }
        }
    }

    pub fn try_send_batch(&self, batch: &mut SendBatchRaw) -> io::Result<usize> {
        let sent = crate::platform::try_send_batch(self.fd, batch)?;

        #[cfg(feature = "metrics")]
        if let Some(ref m) = self.metrics {
            m.inc_packets_sent(sent as u64);
            m.inc_batches_sent();
            m.inc_send_syscalls();
        }

        Ok(sent)
    }

    pub fn try_recv_batch(&self, batch: &mut RecvBatchRaw) -> io::Result<usize> {
        let received = crate::platform::try_recv_batch(self.fd, batch)?;

        #[cfg(feature = "metrics")]
        if let Some(ref m) = self.metrics {
            m.inc_packets_received(received as u64);
            m.inc_batches_received();
            m.inc_recv_syscalls();
        }

        Ok(received)
    }

    pub async fn send_to(&self, buf: &[u8], addr: SocketAddr) -> io::Result<usize> {
        let mut batch = SendBatchRaw::with_capacity(1);
        batch.push(buf, Some(addr)).expect("batch capacity 1");
        self.send_batch(&mut batch).await?;
        Ok(buf.len())
    }

    pub async fn recv_from(&self, buf: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        let mut batch = RecvBatchRaw::with_capacity(1, buf.len());
        self.recv_batch(&mut batch).await?;
        let data = batch.data(0);
        let addr = batch.addr(0);
        let len = data.len().min(buf.len());
        buf[..len].copy_from_slice(&data[..len]);
        Ok((len, addr))
    }

    pub fn try_send_to(&self, buf: &[u8], addr: SocketAddr) -> io::Result<usize> {
        let mut batch = SendBatchRaw::with_capacity(1);
        batch.push(buf, Some(addr)).expect("batch capacity 1");
        self.try_send_batch(&mut batch)?;
        Ok(buf.len())
    }

    pub fn connect(&self, addr: SocketAddr) -> io::Result<()> {
        let (addr_ptr, addr_len) = sockaddr_from(addr);
        let ret = unsafe { libc::connect(self.fd, addr_ptr, addr_len) };
        if ret < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        getsockname(self.fd)
    }

    pub fn ttl(&self) -> io::Result<u32> {
        getsockopt_int(self.fd, libc::IPPROTO_IP, libc::IP_TTL).map(|v| v as u32)
    }

    pub fn set_ttl(&self, ttl: u32) -> io::Result<()> {
        set_sockopt_int_raw(self.fd, libc::IPPROTO_IP, libc::IP_TTL, ttl as libc::c_int)
    }

    pub fn as_raw_fd(&self) -> RawFd {
        self.fd
    }

    #[cfg(all(target_os = "linux", feature = "gso"))]
    pub fn set_gso(&self, enabled: bool) -> io::Result<()> {
        let val: libc::c_int = if enabled { 1 } else { 0 };
        set_sockopt_int_raw(self.fd, libc::IPPROTO_UDP, libc::UDP_SEGMENT, val)
    }

    #[cfg(all(target_os = "linux", feature = "gro"))]
    pub fn set_gro(&self, enabled: bool) -> io::Result<()> {
        let val: libc::c_int = if enabled { 1 } else { 0 };
        set_sockopt_int_raw(self.fd, libc::IPPROTO_UDP, libc::UDP_GRO, val)
    }

    pub fn capabilities(&self) -> PlatformCaps {
        platform_capabilities()
    }

    #[cfg(feature = "metrics")]
    pub fn metrics(&self) -> Option<&BingerMetrics> {
        self.metrics.as_ref()
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
        Err(io::Error::new(io::ErrorKind::Other, "tokio feature disabled"))
    }

    #[cfg(not(feature = "tokio"))]
    async fn wait_readable(&self) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::Other, "tokio feature disabled"))
    }
}

#[cfg(not(feature = "tokio"))]
impl Drop for BingerUdp {
    fn drop(&mut self) {
        unsafe { libc::close(self.fd) };
    }
}

unsafe impl Send for BingerUdp {}
unsafe impl Sync for BingerUdp {}

fn sockaddr_from(addr: SocketAddr) -> (*const libc::sockaddr, libc::socklen_t) {
    match addr {
        SocketAddr::V4(v4) => {
            let raw = libc::sockaddr_in {
                sin_family: libc::AF_INET as libc::sa_family_t,
                sin_port: v4.port().to_be(),
                sin_addr: libc::in_addr {
                    s_addr: u32::from_ne_bytes(v4.ip().octets()),
                },
                sin_zero: [0; 8],
            };
            let ptr = &raw as *const _ as *const libc::sockaddr;
            let len = std::mem::size_of_val(&raw) as libc::socklen_t;
            (ptr, len)
        }
        SocketAddr::V6(v6) => {
            let raw = libc::sockaddr_in6 {
                sin6_family: libc::AF_INET6 as libc::sa_family_t,
                sin6_port: v6.port().to_be(),
                sin6_flowinfo: v6.flowinfo(),
                sin6_addr: libc::in6_addr {
                    s6_addr: v6.ip().octets(),
                },
                sin6_scope_id: v6.scope_id(),
            };
            let ptr = &raw as *const _ as *const libc::sockaddr;
            let len = std::mem::size_of_val(&raw) as libc::socklen_t;
            (ptr, len)
        }
    }
}

fn getsockname(fd: RawFd) -> io::Result<SocketAddr> {
    let mut storage: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of_val(&storage) as libc::socklen_t;
    let ret = unsafe {
        libc::getsockname(fd, &mut storage as *mut _ as *mut libc::sockaddr, &mut len)
    };
    if ret < 0 {
        return Err(io::Error::last_os_error());
    }
    match storage.ss_family as i32 {
        libc::AF_INET => {
            let sin: &libc::sockaddr_in = unsafe { &*(&storage as *const _ as *const _) };
            let ip = std::net::Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr));
            let port = u16::from_be(sin.sin_port);
            Ok(SocketAddr::V4(std::net::SocketAddrV4::new(ip, port)))
        }
        libc::AF_INET6 => {
            let sin6: &libc::sockaddr_in6 = unsafe { &*(&storage as *const _ as *const _) };
            let ip = std::net::Ipv6Addr::from(sin6.sin6_addr.s6_addr);
            let port = u16::from_be(sin6.sin6_port);
            Ok(SocketAddr::V6(std::net::SocketAddrV6::new(
                ip,
                port,
                sin6.sin6_flowinfo,
                sin6.sin6_scope_id,
            )))
        }
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unknown address family",
        )),
    }
}

fn getsockopt_int(fd: RawFd, level: libc::c_int, optname: libc::c_int) -> io::Result<libc::c_int> {
    let mut val: libc::c_int = 0;
    let mut len = std::mem::size_of_val(&val) as libc::socklen_t;
    let ret = unsafe {
        libc::getsockopt(fd, level, optname, &mut val as *mut _ as *mut libc::c_void, &mut len)
    };
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(val)
    }
}

fn set_sockopt_int(socket: &std::net::UdpSocket, level: libc::c_int, optname: libc::c_int, val: libc::c_int) -> io::Result<()> {
    let ret = unsafe {
        libc::setsockopt(
            socket.as_raw_fd(),
            level,
            optname,
            &val as *const _ as *const libc::c_void,
            std::mem::size_of_val(&val) as libc::socklen_t,
        )
    };
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn set_sockopt_int_raw(fd: RawFd, level: libc::c_int, optname: libc::c_int, val: libc::c_int) -> io::Result<()> {
    let ret = unsafe {
        libc::setsockopt(
            fd,
            level,
            optname,
            &val as *const _ as *const libc::c_void,
            std::mem::size_of_val(&val) as libc::socklen_t,
        )
    };
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
