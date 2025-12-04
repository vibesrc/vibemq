//! Simple MQTT connection test

#![allow(clippy::vec_init_then_push)]

use std::io::{Read, Write};
use std::net::TcpStream;

fn main() -> std::io::Result<()> {
    println!("Connecting to localhost:1883...");
    let mut stream = TcpStream::connect("127.0.0.1:1883")?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(std::time::Duration::from_secs(5)))?;

    println!("Connected, sending CONNECT packet...");

    // Build MQTT v3.1.1 CONNECT packet
    // Fixed header: 0x10 (CONNECT), remaining length
    // Variable header: Protocol name "MQTT", version 4, flags 0x02 (clean session), keep alive 60
    // Payload: client ID "test-client"

    let client_id = b"test-client";
    let remaining_length = 2 + 4 + 1 + 1 + 2 + 2 + client_id.len(); // 10 + 2 + 11 = 23

    let mut packet = Vec::new();
    // Fixed header
    packet.push(0x10); // CONNECT
    packet.push(remaining_length as u8);

    // Variable header
    // Protocol name
    packet.push(0x00); // Length MSB
    packet.push(0x04); // Length LSB
    packet.extend_from_slice(b"MQTT");

    // Protocol version
    packet.push(0x04); // v3.1.1

    // Connect flags
    packet.push(0x02); // Clean session

    // Keep alive
    packet.push(0x00); // MSB
    packet.push(0x3C); // LSB (60 seconds)

    // Payload - client ID
    packet.push(0x00); // Length MSB
    packet.push(client_id.len() as u8);
    packet.extend_from_slice(client_id);

    println!("Packet: {:02x?}", packet);
    println!("Packet length: {}", packet.len());

    stream.write_all(&packet)?;
    stream.flush()?;
    println!("CONNECT sent, waiting for CONNACK...");

    // Read response
    let mut buf = [0u8; 256];
    match stream.read(&mut buf) {
        Ok(n) => {
            println!("Received {} bytes:", n);
            println!("  {:02x?}", &buf[..n]);

            if n >= 2 {
                let packet_type = buf[0] >> 4;
                println!("  Packet type: {}", packet_type);
                if packet_type == 2 {
                    println!("  CONNACK received!");
                    if n >= 4 {
                        println!("  Session present: {}", buf[2]);
                        println!("  Return code: {}", buf[3]);
                    }
                }
            }
        }
        Err(e) => {
            println!("Read error: {}", e);
        }
    }

    Ok(())
}
