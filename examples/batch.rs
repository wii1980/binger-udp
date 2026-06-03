// Batch example: send 32 packets and receive them in one batch each.
//
// Demonstrates SendBatch::<32> and RecvBatch::<32> with the core
// send_batch / recv_batch API. Prints the count and first/last packet.

use std::net::UdpSocket;
use binger_udp::{BingerUdp, Config, RecvBatch, SendBatch};

#[tokio::main]
async fn main() -> std::io::Result<()> {
    // Create sender and receiver
    let sender = BingerUdp::from_std(UdpSocket::bind("127.0.0.1:0")?, Config::default())?;
    let receiver = BingerUdp::from_std(UdpSocket::bind("127.0.0.1:0")?, Config::default())?;
    let recv_addr = receiver.local_addr()?;

    // --- Batch send ---
    // Collect messages first: SendBatch stores raw pointers, data must
    // outlive the send_batch() call.
    let msgs: Vec<String> = (0..32).map(|i| format!("packet-{i}")).collect();
    let mut send_batch = SendBatch::<32>::new();
    for msg in &msgs {
        // unwrap: capacity is 32, we push exactly 32 items
        send_batch.push(msg.as_bytes(), recv_addr).unwrap();
    }
    let sent = sender.send_batch(&mut *send_batch).await?;
    println!("sent {sent} packets in one batch");

    // --- Batch receive ---
    let mut recv_batch = RecvBatch::<32>::new(2048);
    let n = receiver.recv_batch(&mut *recv_batch).await?;
    println!("received {n} packets in one batch");

    // Print first and last packet
    if n > 0 {
        let (first_data, first_addr) = recv_batch.iter().next().unwrap();
        println!(
            "first packet: {} bytes from {first_addr}: {:?}",
            first_data.len(),
            String::from_utf8_lossy(first_data),
        );

        let last = recv_batch.iter().last().unwrap();
        println!(
            "last packet: {} bytes: {:?}",
            last.0.len(),
            String::from_utf8_lossy(last.0),
        );
    }

    Ok(())
}
