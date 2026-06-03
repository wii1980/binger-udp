// Select example: concurrent send and receive with tokio::select!.
//
// Creates a sender/receiver pair and uses select! with branch guards
// to alternate between writable (send batches) and readable (receive)
// events. Runs for a few iterations then exits.
//
// The guards (if conditions on select! branches) are essential:
// tokio's readable()/writable() use edge-triggered readiness, so we
// must control which branch is active at each step.

use std::net::UdpSocket;
use std::time::Duration;
use udp_binger::{BingerUdp, SendBatch, RecvBatch, Config};

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let sender = BingerUdp::from_std(
        UdpSocket::bind("127.0.0.1:0")?,
        Config::default(),
    )?;
    let receiver = BingerUdp::from_std(
        UdpSocket::bind("127.0.0.1:0")?,
        Config::default(),
    )?;
    let recv_addr = receiver.local_addr()?;

    let mut send_batch = SendBatch::<8>::new();
    let mut recv_batch = RecvBatch::<8>::new(2048);
    let mut iteration = 0u32;
    let mut waiting_for_recv = false;
    const MAX_ITER: u32 = 3;

    // Pre-fill the first send batch.
    // SendBatch stores raw pointers: data must outlive send_batch().
    let mut pending_msgs: Vec<String> = (0..8).map(|i| format!("data-{i}")).collect();
    for msg in &pending_msgs {
        send_batch.push(msg.as_bytes(), recv_addr).unwrap();
    }

    loop {
        tokio::select! {
            biased;

            _ = receiver.readable(), if waiting_for_recv => {
                let n = receiver.recv_batch(&mut *recv_batch).await?;
                println!("received {n} packets:");
                for (data, addr) in recv_batch.iter() {
                    println!(
                        "  {} bytes from {addr}: {:?}",
                        data.len(),
                        String::from_utf8_lossy(data),
                    );
                }
                recv_batch.clear();
                waiting_for_recv = false;
            }
            _ = sender.writable(), if !waiting_for_recv => {
                if !send_batch.is_empty() {
                    let n = sender.send_batch(&mut *send_batch).await?;
                    println!("iteration {iteration}: sent {n} packets");
                    send_batch.clear();
                    iteration += 1;
                    waiting_for_recv = true;
                }
            }
        }

        if iteration >= MAX_ITER {
            // Drain remaining data
            tokio::time::sleep(Duration::from_millis(100)).await;
            if receiver.readable().await.is_ok() {
                if let Ok(n) = receiver.recv_batch(&mut *recv_batch).await {
                    println!("drained {n} remaining packets");
                }
            }
            break;
        }

        // Re-fill the send batch for the next iteration
        if send_batch.is_empty() && iteration < MAX_ITER {
            pending_msgs = (0..8)
                .map(|i| format!("data-{i}-round{iteration}"))
                .collect();
            for msg in &pending_msgs {
                send_batch.push(msg.as_bytes(), recv_addr).unwrap();
            }
        }
    }

    Ok(())
}
