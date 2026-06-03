// Basic example: single-packet send and receive.
//
// Creates two BingerUdp sockets on 127.0.0.1:0, sends "hello binger"
// from one to the other, and prints the received message.

use std::net::UdpSocket;
use udp_binger::{BingerUdp, Config};

#[tokio::main]
async fn main() -> std::io::Result<()> {
    // Create sender and receiver, both bound to OS-assigned ports
    let sender = BingerUdp::from_std(
        UdpSocket::bind("127.0.0.1:0")?,
        Config::default(),
    )?;
    let receiver = BingerUdp::from_std(
        UdpSocket::bind("127.0.0.1:0")?,
        Config::default(),
    )?;
    let recv_addr = receiver.local_addr()?;

    // Send a single packet
    let msg = b"hello binger";
    let n = sender.send_to(msg, recv_addr).await?;
    println!("sent {n} bytes to {recv_addr}");

    // Receive the packet
    let mut buf = vec![0u8; 2048];
    let (n, src) = receiver.recv_from(&mut buf).await?;
    let received = &buf[..n];
    println!(
        "received {n} bytes from {src}: {:?}",
        String::from_utf8_lossy(received),
    );

    Ok(())
}
