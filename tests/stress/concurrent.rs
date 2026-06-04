use std::io;
use std::net::UdpSocket;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Instant;

use binger_udp::{BingerUdp, Config, RecvBatch, SendBatch};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Send batch with retry on `WouldBlock`, using `try_send_batch` (sync).
/// The caller must ensure batch data outlives this call.
fn send_retry<const N: usize>(socket: &BingerUdp, batch: &mut SendBatch<N>) -> io::Result<usize> {
    loop {
        match socket.try_send_batch(batch) {
            Ok(n) => return Ok(n),
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                thread::yield_now();
            }
            Err(e) => return Err(e),
        }
    }
}

/// Recv batch with retry on `WouldBlock`, using `try_recv_batch` (sync).
fn recv_retry<const N: usize>(socket: &BingerUdp, batch: &mut RecvBatch<N>) -> io::Result<usize> {
    loop {
        match socket.try_recv_batch(batch) {
            Ok(n) => return Ok(n),
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                thread::yield_now();
            }
            Err(e) => return Err(e),
        }
    }
}

// ---------------------------------------------------------------------------
// 1. Multiple concurrent senders, one receiver
// ---------------------------------------------------------------------------

#[test]
fn test_concurrent_senders_one_receiver() -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        const N_SENDERS: usize = 4;
        const PER_SENDER: usize = 16;
        const TOTAL: usize = N_SENDERS * PER_SENDER;

        let recv = BingerUdp::from_std(UdpSocket::bind("127.0.0.1:0")?, Config::default())?;
        let recv_addr = recv.local_addr()?;
        let recv = Arc::new(recv);

        let barrier = Arc::new(Barrier::new(N_SENDERS + 1));

        // Spawn sender threads, each with its own runtime
        let mut handles = Vec::with_capacity(N_SENDERS);
        for id in 0..N_SENDERS {
            let dst = recv_addr;
            let bar = Arc::clone(&barrier);

            handles.push(thread::spawn(move || -> io::Result<()> {
                let rt = tokio::runtime::Runtime::new()?;
                rt.block_on(async {
                    let sender =
                        BingerUdp::from_std(UdpSocket::bind("127.0.0.1:0")?, Config::default())?;
                    bar.wait();

                    // Keep payloads alive until send_retry completes
                    let payloads: Vec<Vec<u8>> = (0..PER_SENDER)
                        .map(|i| format!("s{id}-p{i:02}").into_bytes())
                        .collect();
                    let mut batch = SendBatch::<PER_SENDER>::new();
                    for p in &payloads {
                        batch.push(p, dst).expect("batch should not overflow");
                    }
                    send_retry(&sender, &mut batch)?;
                    // payloads still alive here
                    Ok(())
                })
            }));
        }

        // Spawn receiver thread
        let recv_clone = Arc::clone(&recv);
        let bar = Arc::clone(&barrier);
        let receiver = thread::spawn(move || -> io::Result<()> {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(async {
                bar.wait();

                let start = Instant::now();
                let deadline = std::time::Duration::from_secs(30);

                let mut rb = RecvBatch::<PER_SENDER>::new(2048);
                let mut received: Vec<Vec<u8>> = Vec::with_capacity(TOTAL);

                while received.len() < TOTAL {
                    assert!(
                        start.elapsed() < deadline,
                        "Receiver timeout: got {}/{} after {:.1}s",
                        received.len(),
                        TOTAL,
                        start.elapsed().as_secs_f64(),
                    );
                    let n = recv_retry(&recv_clone, &mut rb)?;
                    for i in 0..n {
                        received.push(rb.data(i).to_vec());
                    }
                }

                assert_eq!(received.len(), TOTAL);
                let mut per_sender = [0usize; N_SENDERS];
                for payload in &received {
                    let s = String::from_utf8_lossy(payload);
                    let id: usize = s
                        .as_bytes()
                        .get(1)
                        .map(|&c| (c - b'0') as usize)
                        .expect("valid sender id");
                    assert!(id < N_SENDERS, "sender id out of range: {id}");
                    per_sender[id] += 1;
                }
                for &count in &per_sender {
                    assert_eq!(count, PER_SENDER);
                }

                Ok(())
            })
        });

        for h in handles {
            h.join().expect("sender thread panicked")?;
        }
        receiver.join().expect("receiver thread panicked")?;
        Ok(())
    })
}

// ---------------------------------------------------------------------------
// 2. Bidirectional concurrent send/recv on the same socket pair
// ---------------------------------------------------------------------------

#[test]
fn test_concurrent_send_recv_same_socket() -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        const ROUNDS: usize = 20;
        const PER_ROUND: usize = 4;
        const EXPECTED: usize = ROUNDS * PER_ROUND;

        let a = Arc::new(BingerUdp::from_std(
            UdpSocket::bind("127.0.0.1:0")?,
            Config::default(),
        )?);
        let b = Arc::new(BingerUdp::from_std(
            UdpSocket::bind("127.0.0.1:0")?,
            Config::default(),
        )?);
        let addr_a = a.local_addr()?;
        let addr_b = b.local_addr()?;

        // Sender A -> B (thread with own runtime)
        let a_to_b = {
            let a = Arc::clone(&a);
            thread::spawn(move || -> io::Result<()> {
                let rt = tokio::runtime::Runtime::new()?;
                rt.block_on(async {
                    for r in 0..ROUNDS {
                        let payloads: Vec<Vec<u8>> = (0..PER_ROUND)
                            .map(|p| format!("a->b:{r},{p}").into_bytes())
                            .collect();
                        let mut sb = SendBatch::<PER_ROUND>::new();
                        for p in &payloads {
                            sb.push(p, addr_b).expect("batch should not overflow");
                        }
                        send_retry(&a, &mut sb)?;
                    }
                    Ok(())
                })
            })
        };

        // Sender B -> A
        let b_to_a = {
            let b = Arc::clone(&b);
            thread::spawn(move || -> io::Result<()> {
                let rt = tokio::runtime::Runtime::new()?;
                rt.block_on(async {
                    for r in 0..ROUNDS {
                        let payloads: Vec<Vec<u8>> = (0..PER_ROUND)
                            .map(|p| format!("b->a:{r},{p}").into_bytes())
                            .collect();
                        let mut sb = SendBatch::<PER_ROUND>::new();
                        for p in &payloads {
                            sb.push(p, addr_a).expect("batch should not overflow");
                        }
                        send_retry(&b, &mut sb)?;
                    }
                    Ok(())
                })
            })
        };

        // Receiver on A (thread with own runtime)
        let recv_a = {
            let a = Arc::clone(&a);
            thread::spawn(move || -> io::Result<usize> {
                let rt = tokio::runtime::Runtime::new()?;
                rt.block_on(async {
                    let mut rb = RecvBatch::<PER_ROUND>::new(2048);
                    let mut total = 0;
                    while total < EXPECTED {
                        total += recv_retry(&a, &mut rb)?;
                    }
                    Ok(total)
                })
            })
        };

        // Receiver on B
        let recv_b = {
            let b = Arc::clone(&b);
            thread::spawn(move || -> io::Result<usize> {
                let rt = tokio::runtime::Runtime::new()?;
                rt.block_on(async {
                    let mut rb = RecvBatch::<PER_ROUND>::new(2048);
                    let mut total = 0;
                    while total < EXPECTED {
                        total += recv_retry(&b, &mut rb)?;
                    }
                    Ok(total)
                })
            })
        };

        a_to_b.join().expect("sender A->B panicked")?;
        b_to_a.join().expect("sender B->A panicked")?;

        let a_got = recv_a.join().expect("receiver A panicked")?;
        let b_got = recv_b.join().expect("receiver B panicked")?;

        assert_eq!(a_got, EXPECTED);
        assert_eq!(b_got, EXPECTED);

        Ok(())
    })
}

// ---------------------------------------------------------------------------
// 3. Batch reuse in a tight loop (100 rounds)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_concurrent_batch_reuse() -> TestResult {
    let (send, recv, recv_addr) = make_pair_async().await?;

    let mut sb = SendBatch::<8>::new();
    let mut rb = RecvBatch::<8>::new(2048);

    for round in 0..100 {
        // Keep payloads alive until after send_batch
        let payloads: Vec<Vec<u8>> = (0..5)
            .map(|i| format!("r{round}-m{i}").into_bytes())
            .collect();

        for p in &payloads {
            sb.push(p.as_slice(), recv_addr)?;
        }
        let n = send.send_batch(&mut sb).await?;
        assert_eq!(n, 5, "round {round}: send 5 packets");

        let mut received_data: Vec<Vec<u8>> = Vec::new();
        while received_data.len() < 5 {
            let n = recv.recv_batch(&mut rb).await?;
            for i in 0..n {
                received_data.push(rb.data(i).to_vec());
            }
            rb.clear();
        }
        assert_eq!(received_data.len(), 5, "round {round}: receive 5 packets");

        for expected in &payloads {
            assert!(
                received_data.iter().any(|d| d == expected),
                "round {round}: missing packet {expected:?}"
            );
        }

        sb.clear();
        rb.clear();

        assert!(sb.is_empty());
        assert!(rb.is_empty());
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// 4. Multiple concurrent receivers, one sender
// ---------------------------------------------------------------------------

#[test]
fn test_multi_receiver_concurrent() -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let recv_a = Arc::new(BingerUdp::from_std(
            UdpSocket::bind("127.0.0.1:0")?,
            Config::default(),
        )?);
        let addr_a = recv_a.local_addr()?;

        let recv_b = Arc::new(BingerUdp::from_std(
            UdpSocket::bind("127.0.0.1:0")?,
            Config::default(),
        )?);
        let addr_b = recv_b.local_addr()?;

        let recv_c = Arc::new(BingerUdp::from_std(
            UdpSocket::bind("127.0.0.1:0")?,
            Config::default(),
        )?);
        let addr_c = recv_c.local_addr()?;

        let sender = BingerUdp::from_std(UdpSocket::bind("127.0.0.1:0")?, Config::default())?;

        let mut sb = SendBatch::<3>::new();
        sb.push(b"payload-for-A", addr_a)?;
        sb.push(b"payload-for-B", addr_b)?;
        sb.push(b"payload-for-C", addr_c)?;
        // Static data - no lifetime issue
        let n = send_retry(&sender, &mut sb)?;
        assert_eq!(n, 3);

        let h_a = thread::spawn(move || -> io::Result<()> {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(async {
                let mut rb = RecvBatch::<4>::new(2048);
                let n = recv_retry(&recv_a, &mut rb)?;
                assert!(n >= 1, "receiver A: got {n}");
                assert!((0..n).any(|i| rb.data(i) == b"payload-for-A"));
                Ok(())
            })
        });

        let h_b = thread::spawn(move || -> io::Result<()> {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(async {
                let mut rb = RecvBatch::<4>::new(2048);
                let n = recv_retry(&recv_b, &mut rb)?;
                assert!(n >= 1, "receiver B: got {n}");
                assert!((0..n).any(|i| rb.data(i) == b"payload-for-B"));
                Ok(())
            })
        });

        let h_c = thread::spawn(move || -> io::Result<()> {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(async {
                let mut rb = RecvBatch::<4>::new(2048);
                let n = recv_retry(&recv_c, &mut rb)?;
                assert!(n >= 1, "receiver C: got {n}");
                assert!((0..n).any(|i| rb.data(i) == b"payload-for-C"));
                Ok(())
            })
        });

        h_a.join().expect("receiver A panicked")?;
        h_b.join().expect("receiver B panicked")?;
        h_c.join().expect("receiver C panicked")?;

        Ok(())
    })
}

// ---------------------------------------------------------------------------
// Internal helpers (not tests)
// ---------------------------------------------------------------------------

/// Helper for creating a sender/receiver pair in `#[tokio::test]`.
#[allow(clippy::unused_async)]
async fn make_pair_async() -> io::Result<(BingerUdp, BingerUdp, std::net::SocketAddr)> {
    let recv = BingerUdp::from_std(UdpSocket::bind("127.0.0.1:0")?, Config::default())?;
    let recv_addr = recv.local_addr()?;
    let send = BingerUdp::from_std(UdpSocket::bind("127.0.0.1:0")?, Config::default())?;
    Ok((send, recv, recv_addr))
}
