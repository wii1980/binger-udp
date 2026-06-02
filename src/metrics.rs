use std::sync::atomic::{AtomicU64, Ordering};

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
    pub fn packets_sent(&self) -> u64 {
        self.packets_sent.load(Ordering::Relaxed)
    }

    pub fn packets_received(&self) -> u64 {
        self.packets_received.load(Ordering::Relaxed)
    }

    pub fn batches_sent(&self) -> u64 {
        self.batches_sent.load(Ordering::Relaxed)
    }

    pub fn batches_received(&self) -> u64 {
        self.batches_received.load(Ordering::Relaxed)
    }

    pub fn send_syscalls(&self) -> u64 {
        self.send_syscalls.load(Ordering::Relaxed)
    }

    pub fn recv_syscalls(&self) -> u64 {
        self.recv_syscalls.load(Ordering::Relaxed)
    }

    pub fn send_errors(&self) -> u64 {
        self.send_errors.load(Ordering::Relaxed)
    }

    pub fn recv_errors(&self) -> u64 {
        self.recv_errors.load(Ordering::Relaxed)
    }

    pub fn send_would_block(&self) -> u64 {
        self.send_would_block.load(Ordering::Relaxed)
    }

    pub fn recv_would_block(&self) -> u64 {
        self.recv_would_block.load(Ordering::Relaxed)
    }

    pub fn syscall_efficiency_ratio(&self) -> f64 {
        let pkt = self.packets_sent() as f64;
        let sc = self.send_syscalls() as f64;
        if sc > 0.0 {
            pkt / sc
        } else {
            0.0
        }
    }

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

    pub(crate) fn inc_packets_sent(&self, n: u64) {
        self.packets_sent.fetch_add(n, Ordering::Relaxed);
    }

    pub(crate) fn inc_packets_received(&self, n: u64) {
        self.packets_received.fetch_add(n, Ordering::Relaxed);
    }

    pub(crate) fn inc_batches_sent(&self) {
        self.batches_sent.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn inc_batches_received(&self) {
        self.batches_received.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn inc_send_syscalls(&self) {
        self.send_syscalls.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn inc_recv_syscalls(&self) {
        self.recv_syscalls.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn inc_send_errors(&self) {
        self.send_errors.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn inc_recv_errors(&self) {
        self.recv_errors.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn inc_send_would_block(&self) {
        self.send_would_block.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn inc_recv_would_block(&self) {
        self.recv_would_block.fetch_add(1, Ordering::Relaxed);
    }
}

#[derive(Debug, Clone, Copy)]
pub struct MetricsSnapshot {
    pub packets_sent: u64,
    pub packets_received: u64,
    pub batches_sent: u64,
    pub batches_received: u64,
    pub send_syscalls: u64,
    pub recv_syscalls: u64,
    pub send_errors: u64,
    pub recv_errors: u64,
    pub send_would_block: u64,
    pub recv_would_block: u64,
    pub syscall_efficiency: f64,
}
