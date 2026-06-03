// Config builder example: custom configuration and platform capabilities.
//
// Creates a BingerUdp with non-default batch_size, recv_buf_size, and
// send_buf_size. Prints platform capabilities, then sends and receives
// one packet to verify the custom config works correctly.

use binger_udp::{platform_capabilities, BingerUdp, Config, RecvBatch, SendBatch};
use std::net::UdpSocket;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    // Display platform capabilities
    let caps = platform_capabilities();
    println!("Platform backend: {}", caps.backend_name);
    println!("  supports sendmmsg: {}", caps.supports_sendmmsg);
    println!("  supports recvmmsg: {}", caps.supports_recvmmsg);
    println!("  supports GSO: {}", caps.supports_gso);
    println!("  supports GRO: {}", caps.supports_gro);
    println!("  supports busy-poll: {}", caps.supports_busy_poll);
    println!("  supports pacing: {}", caps.supports_pacing);
    println!("  max batch size: {}", caps.max_batch_size);

    // Build a custom config
    let config = Config::new()
        .with_batch_size(16)
        .with_send_buf_size(65536);

    println!("\nConfig: batch_size=16, send_buf_size=65536");

    // Create a receiver with custom config
    let receiver = BingerUdp::from_std(UdpSocket::bind("127.0.0.1:0")?, config)?;
    let recv_addr = receiver.local_addr()?;

    // Create a sender with default config
    let sender = BingerUdp::from_std(UdpSocket::bind("127.0.0.1:0")?, Config::default())?;

    // Verify the capabilities method works
    let sock_caps = receiver.capabilities();
    println!(
        "Socket backend: {} (max batch {})",
        sock_caps.backend_name, sock_caps.max_batch_size,
    );

    // Send and receive one packet to verify custom config works
    let mut send_batch = SendBatch::<1>::new();
    send_batch.push(b"custom-config-works", recv_addr).unwrap();
    // unwrap: capacity is 1, we push exactly 1 item
    let sent = sender.send_batch(&mut send_batch).await?;
    println!("\nsent {sent} packet");

    let mut recv_batch = RecvBatch::<1>::new(2048);
    let n = receiver.recv_batch(&mut recv_batch).await?;
    println!("received {n} packet");

    if n > 0 {
        let data = recv_batch.data(0);
        println!("data: {:?}", String::from_utf8_lossy(data));
    }

    Ok(())
}
