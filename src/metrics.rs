use std::sync::atomic::{AtomicU64, Ordering};

/// Atomic counters for observing batch I/O behaviour.
///
/// All counters use `Relaxed` ordering — they are suitable for monitoring and
/// debugging, not for synchronisation.
///
/// `BingerMetrics` is feature-gated behind `features = ["metrics"]` and adds
/// zero cost when disabled.
///
/// # Example
///
/// ```rust,no_run
/// use binger_udp::BingerMetrics;
///
/// let m = BingerMetrics::default();
/// let snapshot = m.snapshot();
/// println!("packets sent: {}", snapshot.packets_sent);
/// ```
#[derive(Default)]
pub struct BingerMetrics {
    packets_sent: AtomicU64,
    packets_received: AtomicU64,
    batches_sent: AtomicU64,
    batches_received: AtomicU64,
    send_syscalls: AtomicU64,
    recv_syscalls: AtomicU64,
    send_errors: AtomicU64,
    recv_errors: AtomicU64,
    send_would_block: AtomicU64,
    recv_would_block: AtomicU64,
}

impl BingerMetrics {
    /// Total number of UDP datagrams sent.
    #[must_use]
    pub fn packets_sent(&self) -> u64 {
        self.packets_sent.load(Ordering::Relaxed)
    }

    /// Total number of UDP datagrams received.
    #[must_use]
    pub fn packets_received(&self) -> u64 {
        self.packets_received.load(Ordering::Relaxed)
    }

    /// Number of batch-send operations.
    #[must_use]
    pub fn batches_sent(&self) -> u64 {
        self.batches_sent.load(Ordering::Relaxed)
    }

    /// Number of batch-receive operations.
    #[must_use]
    pub fn batches_received(&self) -> u64 {
        self.batches_received.load(Ordering::Relaxed)
    }

    /// Number of `sendto`/`sendmmsg`/`WSASendMsg` syscalls made.
    #[must_use]
    pub fn send_syscalls(&self) -> u64 {
        self.send_syscalls.load(Ordering::Relaxed)
    }

    /// Number of `recvfrom`/`recvmmsg`/`WSARecvMsg` syscalls made.
    #[must_use]
    pub fn recv_syscalls(&self) -> u64 {
        self.recv_syscalls.load(Ordering::Relaxed)
    }

    /// Number of send errors encountered.
    #[must_use]
    pub fn send_errors(&self) -> u64 {
        self.send_errors.load(Ordering::Relaxed)
    }

    /// Number of receive errors encountered.
    #[must_use]
    pub fn recv_errors(&self) -> u64 {
        self.recv_errors.load(Ordering::Relaxed)
    }

    /// Number of `WouldBlock` events on send.
    #[must_use]
    pub fn send_would_block(&self) -> u64 {
        self.send_would_block.load(Ordering::Relaxed)
    }

    /// Number of `WouldBlock` events on receive.
    #[must_use]
    pub fn recv_would_block(&self) -> u64 {
        self.recv_would_block.load(Ordering::Relaxed)
    }

    /// Ratio of packets sent per send syscall.
    ///
    /// A value of `1.0` means no batching (each packet is its own syscall).
    /// Higher values indicate more efficient batching.
    #[allow(clippy::cast_precision_loss)]
    #[must_use]
    pub fn syscall_efficiency_ratio(&self) -> f64 {
        let pkt = self.packets_sent() as f64;
        let sc = self.send_syscalls() as f64;
        if sc > 0.0 {
            pkt / sc
        } else {
            0.0
        }
    }

    /// Captures a point-in-time snapshot of all counters.
    #[must_use]
    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            packets_sent: self.packets_sent(),
            packets_received: self.packets_received(),
            batches_sent: self.batches_sent(),
            batches_received: self.batches_received(),
            send_syscalls: self.send_syscalls(),
            recv_syscalls: self.recv_syscalls(),
            send_errors: self.send_errors(),
            recv_errors: self.recv_errors(),
            send_would_block: self.send_would_block(),
            recv_would_block: self.recv_would_block(),
            syscall_efficiency: self.syscall_efficiency_ratio(),
        }
    }

    /// Resets all counters to zero.
    pub fn reset(&self) {
        self.packets_sent.store(0, Ordering::Relaxed);
        self.packets_received.store(0, Ordering::Relaxed);
        self.batches_sent.store(0, Ordering::Relaxed);
        self.batches_received.store(0, Ordering::Relaxed);
        self.send_syscalls.store(0, Ordering::Relaxed);
        self.recv_syscalls.store(0, Ordering::Relaxed);
        self.send_errors.store(0, Ordering::Relaxed);
        self.recv_errors.store(0, Ordering::Relaxed);
        self.send_would_block.store(0, Ordering::Relaxed);
        self.recv_would_block.store(0, Ordering::Relaxed);
    }

    #[allow(dead_code)]
    pub(crate) fn inc_packets_sent(&self, n: u64) {
        self.packets_sent.fetch_add(n, Ordering::Relaxed);
    }

    #[allow(dead_code)]
    pub(crate) fn inc_packets_received(&self, n: u64) {
        self.packets_received.fetch_add(n, Ordering::Relaxed);
    }

    #[allow(dead_code)]
    pub(crate) fn inc_batches_sent(&self) {
        self.batches_sent.fetch_add(1, Ordering::Relaxed);
    }

    #[allow(dead_code)]
    pub(crate) fn inc_batches_received(&self) {
        self.batches_received.fetch_add(1, Ordering::Relaxed);
    }

    #[allow(dead_code)]
    pub(crate) fn inc_send_syscalls(&self) {
        self.send_syscalls.fetch_add(1, Ordering::Relaxed);
    }

    #[allow(dead_code)]
    pub(crate) fn inc_recv_syscalls(&self) {
        self.recv_syscalls.fetch_add(1, Ordering::Relaxed);
    }

    #[allow(dead_code)]
    pub(crate) fn inc_send_errors(&self) {
        self.send_errors.fetch_add(1, Ordering::Relaxed);
    }

    #[allow(dead_code)]
    pub(crate) fn inc_recv_errors(&self) {
        self.recv_errors.fetch_add(1, Ordering::Relaxed);
    }

    #[allow(dead_code)]
    pub(crate) fn inc_send_would_block(&self) {
        self.send_would_block.fetch_add(1, Ordering::Relaxed);
    }

    #[allow(dead_code)]
    pub(crate) fn inc_recv_would_block(&self) {
        self.recv_would_block.fetch_add(1, Ordering::Relaxed);
    }
}

/// Point-in-time snapshot of [`BingerMetrics`] counters.
#[derive(Debug, Clone, Copy)]
pub struct MetricsSnapshot {
    /// Total UDP datagrams sent.
    pub packets_sent: u64,
    /// Total UDP datagrams received.
    pub packets_received: u64,
    /// Number of batch-send operations.
    pub batches_sent: u64,
    /// Number of batch-receive operations.
    pub batches_received: u64,
    /// Number of send syscalls made.
    pub send_syscalls: u64,
    /// Number of receive syscalls made.
    pub recv_syscalls: u64,
    /// Number of send errors.
    pub send_errors: u64,
    /// Number of receive errors.
    pub recv_errors: u64,
    /// Number of `WouldBlock` on send.
    pub send_would_block: u64,
    /// Number of `WouldBlock` on receive.
    pub recv_would_block: u64,
    /// Packets-per-syscall ratio (higher is better).
    pub syscall_efficiency: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_all_zero() {
        let m = BingerMetrics::default();
        assert_eq!(m.packets_sent(), 0);
        assert_eq!(m.packets_received(), 0);
        assert_eq!(m.batches_sent(), 0);
        assert_eq!(m.batches_received(), 0);
        assert_eq!(m.send_syscalls(), 0);
        assert_eq!(m.recv_syscalls(), 0);
        assert_eq!(m.send_errors(), 0);
        assert_eq!(m.recv_errors(), 0);
        assert_eq!(m.send_would_block(), 0);
        assert_eq!(m.recv_would_block(), 0);
    }

    #[test]
    fn inc_counters() {
        let m = BingerMetrics::default();
        m.inc_packets_sent(10);
        m.inc_packets_received(5);
        m.inc_batches_sent();
        m.inc_batches_received();
        m.inc_send_syscalls();
        m.inc_recv_syscalls();
        m.inc_send_errors();
        m.inc_recv_errors();
        m.inc_send_would_block();
        m.inc_recv_would_block();

        assert_eq!(m.packets_sent(), 10);
        assert_eq!(m.packets_received(), 5);
        assert_eq!(m.batches_sent(), 1);
        assert_eq!(m.batches_received(), 1);
        assert_eq!(m.send_syscalls(), 1);
        assert_eq!(m.recv_syscalls(), 1);
        assert_eq!(m.send_errors(), 1);
        assert_eq!(m.recv_errors(), 1);
        assert_eq!(m.send_would_block(), 1);
        assert_eq!(m.recv_would_block(), 1);
    }

    #[test]
    fn inc_accumulates() {
        let m = BingerMetrics::default();
        m.inc_packets_sent(3);
        m.inc_packets_sent(7);
        assert_eq!(m.packets_sent(), 10);
    }

    #[test]
    fn snapshot_matches_counters() {
        let m = BingerMetrics::default();
        m.inc_packets_sent(32);
        m.inc_batches_sent();

        let snap = m.snapshot();
        assert_eq!(snap.packets_sent, 32);
        assert_eq!(snap.batches_sent, 1);
        assert_eq!(snap.packets_received, 0);
    }

    #[test]
    fn reset_clears_all() {
        let m = BingerMetrics::default();
        m.inc_packets_sent(100);
        m.inc_send_syscalls();
        m.reset();
        assert_eq!(m.packets_sent(), 0);
        assert_eq!(m.send_syscalls(), 0);
    }

    #[test]
    fn efficiency_ratio_no_syscalls() {
        let m = BingerMetrics::default();
        let ratio = m.syscall_efficiency_ratio();
        assert!(ratio.abs() < f64::EPSILON, "expected 0.0, got {ratio}");
    }

    #[test]
    fn efficiency_ratio_perfect_batching() {
        let m = BingerMetrics::default();
        m.inc_packets_sent(32);
        m.inc_send_syscalls();
        let ratio = m.syscall_efficiency_ratio();
        assert!((ratio - 32.0).abs() < f64::EPSILON);
    }

    #[test]
    fn efficiency_ratio_no_batching() {
        let m = BingerMetrics::default();
        m.inc_packets_sent(1);
        m.inc_send_syscalls();
        let ratio = m.syscall_efficiency_ratio();
        assert!((ratio - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn efficiency_ratio_mixed() {
        let m = BingerMetrics::default();
        m.inc_packets_sent(100);
        m.inc_send_syscalls();
        m.inc_send_syscalls();
        m.inc_send_syscalls();
        m.inc_send_syscalls();
        m.inc_send_syscalls();
        m.inc_send_syscalls();
        m.inc_send_syscalls();
        m.inc_send_syscalls();
        m.inc_send_syscalls();
        m.inc_send_syscalls();
        let ratio = m.syscall_efficiency_ratio();
        assert!((ratio - 10.0).abs() < f64::EPSILON);
    }

    #[test]
    fn snapshot_efficiency_matches() {
        let m = BingerMetrics::default();
        m.inc_packets_sent(64);
        m.inc_send_syscalls();
        let snap = m.snapshot();
        assert!((snap.syscall_efficiency - 64.0).abs() < f64::EPSILON);
    }

    #[test]
    fn snapshot_debug_clone_copy() {
        let m = BingerMetrics::default();
        m.inc_packets_sent(1);
        let snap = m.snapshot();
        let cloned = snap;
        assert_eq!(cloned.packets_sent, 1);
        let _debug = format!("{snap:?}");
    }
}
