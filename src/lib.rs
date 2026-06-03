//! Cross-platform, batch-native UDP I/O with platform-optimal syscalls.
//!
//! `binger-udp` gives you a clean batch API for sending and receiving UDP
//! datagrams, automatically selecting the most efficient system call available
//! on the current platform:
//!
//! | Platform | Send | Recv |
//! |----------|------|------|
//! | Linux | `sendmmsg` / `sendmsg` w/ GSO | `recvmmsg` |
//! | macOS | `sendmsg_x` (via `dlsym`) | `recvmsg_x` (via `dlsym`) |
//! | Windows | `WSASendMsg` | `WSARecvMsg` (via `WSAIoctl`) |
//! | Any | `sendto` / `recvfrom` fallback | `recvfrom` fallback |
//!
//! # Quick start
//!
//! ```rust,no_run
//! use std::error::Error;
//! use binger_udp::{BingerUdp, SendBatch, RecvBatch, Config};
//!
//! # async fn example() -> Result<(), Box<dyn Error>> {
//! let socket = BingerUdp::from_std(
//!     std::net::UdpSocket::bind("0.0.0.0:0")?,
//!     Config::default(),
//! )?;
//!
//! // Batch-send 32 packets in one syscall
//! let mut send = SendBatch::<32>::new();
//! send.push(b"hello", "192.168.1.1:8080".parse().unwrap())?;
//! let sent = socket.send_batch(&mut send).await?;
//!
//! // Batch-receive up to 32 packets in one syscall
//! let mut recv = RecvBatch::<32>::new(2048);
//! let n = socket.recv_batch(&mut recv).await?;
//! # Ok(()) }
//! ```

pub mod batch;
pub mod bufs;
pub mod error;
#[cfg(feature = "metrics")]
pub mod metrics;
mod platform;
pub mod sockaddr;
pub mod socket;
mod sys;

#[cfg(feature = "timestamping")]
pub use batch::Timestamp;
pub use batch::{RecvBatch, SendBatch};
pub use bufs::BufferPool;
pub use error::BingerError;
#[cfg(feature = "metrics")]
pub use metrics::BingerMetrics;
pub use socket::{platform_capabilities, BingerUdp, Config, PlatformCaps};
