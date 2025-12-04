//! Multi-connection MQTT test - simulates what conformance tests do

#![allow(clippy::vec_init_then_push)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::thread;
use std::time::Duration;

fn connect(client_id: &str) -> std::io::Result<()> {
    println!("Test: Connecting with client_id={}", client_id);
    let mut stream = TcpStream::connect("127.0.0.1:1883")?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;

    // Build MQTT v3.1.1 CONNECT packet
    let client_id_bytes = client_id.as_bytes();
    let remaining_length = 2 + 4 + 1 + 1 + 2 + 2 + client_id_bytes.len();

    let mut packet = Vec::new();
    packet.push(0x10); // CONNECT
    packet.push(remaining_length as u8);
    packet.push(0x00);
    packet.push(0x04);
    packet.extend_from_slice(b"MQTT");
    packet.push(0x04); // v3.1.1
    packet.push(0x02); // Clean session
    packet.push(0x00);
    packet.push(0x3C); // Keep alive = 60
    packet.push(0x00);
    packet.push(client_id_bytes.len() as u8);
    packet.extend_from_slice(client_id_bytes);

    stream.write_all(&packet)?;
    stream.flush()?;
    println!("  CONNECT sent");

    // Read response
    let mut buf = [0u8; 256];
    match stream.read(&mut buf) {
        Ok(n) => {
            println!("  Received {} bytes: {:02x?}", n, &buf[..n]);
            if n >= 4 && buf[0] >> 4 == 2 {
                println!(
                    "  CONNACK: session_present={}, return_code={}",
                    buf[2], buf[3]
                );
                if buf[3] == 0 {
                    println!("  SUCCESS");

                    // Send DISCONNECT
                    stream.write_all(&[0xE0, 0x00])?;
                    println!("  DISCONNECT sent");
                } else {
                    println!("  FAILED: return code {}", buf[3]);
                }
            }
        }
        Err(e) => {
            println!("  Read error: {}", e);
            return Err(e);
        }
    }
    Ok(())
}

fn main() {
    // Run multiple connection tests
    let tests = [
        "basic-connect",
        "test-client-1",
        "test-client-2",
        "long-client-id-with-many-characters",
    ];

    for (i, client_id) in tests.iter().enumerate() {
        println!("\n=== Test {} ===", i + 1);
        if let Err(e) = connect(client_id) {
            println!("Test {} FAILED: {}", i + 1, e);
        }
        thread::sleep(Duration::from_millis(100));
    }

    println!("\n=== Testing clean session=false ===");
    {
        let mut stream = TcpStream::connect("127.0.0.1:1883").unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();

        // Connect with clean_session=false
        let client_id = b"persistent-session";
        let remaining_length = 2 + 4 + 1 + 1 + 2 + 2 + client_id.len();
        let mut packet = Vec::new();
        packet.push(0x10);
        packet.push(remaining_length as u8);
        packet.push(0x00);
        packet.push(0x04);
        packet.extend_from_slice(b"MQTT");
        packet.push(0x04);
        packet.push(0x00); // clean_session=false
        packet.push(0x00);
        packet.push(0x3C);
        packet.push(0x00);
        packet.push(client_id.len() as u8);
        packet.extend_from_slice(client_id);

        stream.write_all(&packet).unwrap();
        stream.flush().unwrap();

        let mut buf = [0u8; 256];
        match stream.read(&mut buf) {
            Ok(n) => {
                println!("Received {} bytes: {:02x?}", n, &buf[..n]);
                if n >= 4 && buf[0] >> 4 == 2 {
                    println!(
                        "CONNACK: session_present={}, return_code={}",
                        buf[2], buf[3]
                    );
                }
            }
            Err(e) => println!("Read error: {}", e),
        }
    }

    println!("\nDone!");
}
