use anyhow::{bail, Result};
use serialport::{SerialPort, SerialPortType};
use std::collections::VecDeque;
use std::io::ErrorKind;
use std::time::Duration;

const BUFFER_SIZE: usize = 16;

fn find_micropython_ports() -> Result<Vec<String>> {
    let ports = serialport::available_ports()?;
    let mut micropython_ports: Vec<String> = Vec::new();
    for p in ports {
        if p.port_name.starts_with("/dev/cu.") {
            // Skip /dev/cu.X devices in MacOS
            continue;
        }
        if let SerialPortType::UsbPort(info) = p.port_type {
            if let Some(manufacturer) = info.manufacturer {
                if manufacturer == "MicroPython" {
                    micropython_ports.push(p.port_name);
                }
            }
        }
    }

    Ok(micropython_ports)
}

fn read_until(port: &mut dyn SerialPort, bytes: &[u8]) -> Result<()> {
    let mut deque: VecDeque<u8> = VecDeque::from(vec![0; bytes.len()]);
    let mut buf: Vec<u8> = vec![0; 1];
    loop {
        match port.read(&mut buf) {
            Ok(0) => bail!("Unable to read"),
            Ok(1) => {
                let byte = buf[0];
                deque.pop_front();
                deque.push_back(byte);

                if deque.iter().copied().collect::<Vec<u8>>() == bytes {
                    break;
                }
            }
            Err(e) => bail!(e),
            _ => bail!("Unhandled state"),
        }
    }
    Ok(())
}

fn hello_world(port: String) -> Result<()> {
    println!("Opening serial port");
    let mut sport = serialport::new(port, 115_200)
        .timeout(Duration::from_millis(10))
        .open()?;
    let mut buf: Vec<u8> = vec![0; BUFFER_SIZE];

    println!("Sending Ctrl-C twice");
    // Ctrl-C twice: Interrupt any running program
    sport.write_all("\r\x03\x03".as_bytes())?;

    println!("Flush input");
    loop {
        match sport.read(&mut buf) {
            Ok(_) => continue,
            Err(ref e) if e.kind() == ErrorKind::TimedOut => break,
            Err(e) => return Err(e.into()),
        }
    }

    println!("Ctrl-A: enter raw REPL");
    sport.write_all("\r\x01".as_bytes())?;

    read_until(&mut *sport, "raw REPL; CTRL-B to exit\r\n".as_bytes())?;

    println!("Soft reset");
    sport.write_all("\x04".as_bytes())?;

    read_until(&mut *sport, "soft reboot\r\n".as_bytes())?;
    read_until(&mut *sport, "raw REPL; CTRL-B to exit\r\n".as_bytes())?;

    println!("Checking we have a prompt");
    read_until(&mut *sport, ">".as_bytes())?;

    println!("Exec buffer");
    sport.write_all("\x05A\x01".as_bytes())?;

    println!("Reading response");
    let mut buf = [0; 2];
    sport.read_exact(&mut buf)?;
    println!("{:?}", buf);
    match buf {
        [82, 0] => bail!("Device doesn't support raw-paste"),
        [82, 1] => {}
        _ => bail!("Unknown response"),
    }

    println!("Getting initial header with window size");
    sport.read_exact(&mut buf)?;
    println!("{:?}", buf);
    let window_size: u16 = (buf[0] as u16) | (buf[1] as u16) << 8;

    println!("Window size: {}", window_size);

    Ok(())
}

fn main() -> Result<()> {
    let mut ports = find_micropython_ports()?;
    match ports.len() {
        0 => eprintln!("No MicroPython ports found"),
        1 => {
            let port = ports.pop().expect("Unable to pop port");
            hello_world(port)?;
        }
        _ => println!("Found multiple MicroPython ports"),
    }
    Ok(())
}
