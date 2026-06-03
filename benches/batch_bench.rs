//! Comprehensive benchmarks comparing batch vs single-packet UDP I/O.
//!
//! Benchmarks:
//! 1. **send_batch vs loop send_to** — Send N packets (1, 4, 8, 16, 32, 64)
//!    with payload sizes (64, 512, 1400 bytes)
//! 2. **recv_batch vs loop recv_from** — Receive N packets (1, 4, 8, 16, 32)
//! 3. **Throughput** — Batch send+recv MB/s for batch sizes (1, 8, 16, 32)

use std::net::SocketAddr;
use std::time::Duration;

use criterion::{
    black_box, criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput,
};
use binger_udp::{BingerUdp, Config, RecvBatch, SendBatch};

/// Max batch capacity used for all const-generic batch types.
/// Individual benchmarks only push `n` items where `n <= MAX_BATCH`.
const MAX_BATCH: usize = 64;

/// Per-slot receive buffer size.
const RECV_BUF_SIZE: usize = 2048;

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

/// Create a sender/receiver [`BingerUdp`] pair bound to `127.0.0.1:0`.
///
/// Returns `(sender, receiver, receiver_addr)`.
fn setup_pair() -> (BingerUdp, BingerUdp, SocketAddr) {
    static RT: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    let rt =
        RT.get_or_init(|| tokio::runtime::Runtime::new().expect("failed to create tokio runtime"));

    let receiver =
        std::net::UdpSocket::bind("127.0.0.1:0").expect("failed to bind receiver socket");
    let receiver_addr = receiver.local_addr().expect("failed to get receiver addr");
    let sender = std::net::UdpSocket::bind("127.0.0.1:0").expect("failed to bind sender socket");

    let config = Config::default();
    let sender = rt
        .block_on(async { BingerUdp::from_std(sender, config.clone()) })
        .expect("failed to create sender");
    let receiver = rt
        .block_on(async { BingerUdp::from_std(receiver, config) })
        .expect("failed to create receiver");

    (sender, receiver, receiver_addr)
}

// ---------------------------------------------------------------------------
// 1. Send benchmarks
// ---------------------------------------------------------------------------

fn bench_send(c: &mut Criterion) {
    let mut group = c.benchmark_group("send");
    group.warm_up_time(Duration::from_secs(2));
    group.measurement_time(Duration::from_secs(5));

    let payload_sizes: &[usize] = &[64, 512, 1400];
    let batch_sizes: &[usize] = &[1, 4, 8, 16, 32, 64];

    for &payload_size in payload_sizes {
        let payload = vec![0u8; payload_size];

        for &n in batch_sizes {
            let id = format!("{n}/{payload_size}b");
            group.throughput(Throughput::Elements(n as u64));

            // --- Batch: one SendBatch + try_send_batch (1 sendmmsg call) ---
            group.bench_with_input(
                BenchmarkId::new("batch", &id),
                &(n, &payload[..]),
                |b, &(n, payload)| {
                    b.iter_batched(
                        || {
                            let (sender, _receiver, addr) = setup_pair();
                            let mut batch = SendBatch::<MAX_BATCH>::new();
                            for _ in 0..n {
                                batch.push(payload, addr).expect("batch capacity exceeded");
                            }
                            (sender, batch)
                        },
                        |(sender, mut batch)| {
                            let sent = sender
                                .try_send_batch(&mut *batch)
                                .expect("try_send_batch failed");
                            black_box(sent);
                        },
                        BatchSize::SmallInput,
                    );
                },
            );

            // --- Individual: N × try_send_to (N separate sendto syscalls) ---
            group.bench_with_input(
                BenchmarkId::new("individual", &id),
                &(n, &payload[..]),
                |b, &(n, payload)| {
                    b.iter_batched(
                        || {
                            let (sender, _receiver, addr) = setup_pair();
                            (sender, addr)
                        },
                        |(sender, addr)| {
                            for _ in 0..n {
                                let sent = sender
                                    .try_send_to(payload, addr)
                                    .expect("try_send_to failed");
                                black_box(sent);
                            }
                        },
                        BatchSize::SmallInput,
                    );
                },
            );
        }
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// 2. Receive benchmarks
// ---------------------------------------------------------------------------

/// Pre-fill `n` packets into the receiver's socket buffer, then return the
/// receiver so the measured closure can drain it.
fn setup_recv(n: usize, payload: &[u8]) -> BingerUdp {
    let (sender, receiver, addr) = setup_pair();

    let mut send_batch = SendBatch::<MAX_BATCH>::new();
    for _ in 0..n {
        send_batch
            .push(payload, addr)
            .expect("batch capacity exceeded");
    }
    let sent = sender
        .try_send_batch(&mut *send_batch)
        .expect("pre-fill try_send_batch failed");
    assert_eq!(sent, n, "pre-fill: expected {n} packets sent, got {sent}");

    receiver
}

fn bench_recv(c: &mut Criterion) {
    let mut group = c.benchmark_group("recv");
    group.warm_up_time(Duration::from_secs(2));
    group.measurement_time(Duration::from_secs(5));

    let payload = vec![0u8; 64];
    let batch_sizes: &[usize] = &[1, 4, 8, 16, 32];

    for &n in batch_sizes {
        let id = format!("{n}");
        group.throughput(Throughput::Elements(n as u64));

        // --- Batch: one RecvBatch + try_recv_batch (1 recvmmsg call) ---
        group.bench_with_input(BenchmarkId::new("batch", &id), &n, |b, &n| {
            b.iter_batched(
                || setup_recv(n, &payload),
                |receiver| {
                    let mut recv_batch = RecvBatch::<MAX_BATCH>::new(RECV_BUF_SIZE);
                    let received = receiver
                        .try_recv_batch(&mut *recv_batch)
                        .expect("try_recv_batch failed");
                    black_box(received);
                },
                BatchSize::SmallInput,
            );
        });

        // --- Individual: N × RecvBatch<1> + try_recv_batch (N individual recvs) ---
        group.bench_with_input(BenchmarkId::new("individual", &id), &n, |b, &n| {
            b.iter_batched(
                || setup_recv(n, &payload),
                |receiver| {
                    for _ in 0..n {
                        let mut single = RecvBatch::<1>::new(RECV_BUF_SIZE);
                        let received = receiver
                            .try_recv_batch(&mut *single)
                            .expect("try_recv_batch single failed");
                        black_box(received);
                    }
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// 3. Throughput benchmarks
// ---------------------------------------------------------------------------

fn bench_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("throughput");
    group.warm_up_time(Duration::from_secs(2));
    group.measurement_time(Duration::from_secs(5));

    let payload_size = 512;
    let payload = vec![0u8; payload_size];
    let batch_sizes: &[usize] = &[1, 8, 16, 32];

    for &n in batch_sizes {
        let total_bytes = (n * payload_size) as u64;
        group.throughput(Throughput::Bytes(total_bytes));

        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter_batched(
                || {
                    let (sender, receiver, addr) = setup_pair();
                    (sender, receiver, addr)
                },
                |(sender, receiver, addr)| {
                    // Batch send
                    let mut send_batch = SendBatch::<MAX_BATCH>::new();
                    for _ in 0..n {
                        send_batch
                            .push(&payload, addr)
                            .expect("batch capacity exceeded");
                    }
                    let sent = sender
                        .try_send_batch(&mut *send_batch)
                        .expect("throughput try_send_batch failed");
                    black_box(sent);

                    // Batch recv
                    let mut recv_batch = RecvBatch::<MAX_BATCH>::new(RECV_BUF_SIZE);
                    let received = receiver
                        .try_recv_batch(&mut *recv_batch)
                        .expect("throughput try_recv_batch failed");
                    black_box(received);
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Criterion harness
// ---------------------------------------------------------------------------

criterion_group!(benches, bench_send, bench_recv, bench_throughput);
criterion_main!(benches);
