use anyhow::{bail, Result};
use serialport::{SerialPort, SerialPortType};
use std::char;
use std::cmp::min;
use std::collections::VecDeque;
use std::io::ErrorKind;
use std::thread::sleep;
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

fn read_until(port: &mut dyn SerialPort, bytes: &[u8], echo: bool) -> Result<()> {
    let mut deque: VecDeque<u8> = VecDeque::from(vec![0; bytes.len()]);
    let mut buf: Vec<u8> = vec![0; 1];

    let sleep_time = Duration::from_millis(10);
    let timeout: usize = 10; // Timeout if nothing received for 10 seconds
    let mut timeout_count: usize = 0;

    loop {
        match port.read(&mut buf) {
            Ok(0) => bail!("Unable to read"),
            Ok(1) => {
                let byte = buf[0];
                deque.pop_front();
                deque.push_back(byte);
                if echo {
                    print!("{}", char::from(byte));
                }

                if deque.iter().copied().collect::<Vec<u8>>() == bytes {
                    break;
                }
            }
            Err(ref e) if e.kind() == ErrorKind::TimedOut => {
                timeout_count += 1;
                if timeout_count > timeout * 100 {
                    bail!("Timed out in read_until");
                }
                sleep(sleep_time);
            }
            Err(e) => bail!(e),
            _ => bail!("Unhandled state"),
        }
    }
    Ok(())
}

fn execute(port: String, script: String) -> Result<()> {
    let mut sport = serialport::new(port, 115_200)
        .timeout(Duration::from_millis(10))
        .open()?;
    let mut buf: Vec<u8> = vec![0; BUFFER_SIZE];
    let mut byte_buf = [0; 1];
    let mut double_buf = [0; 2];

    // Ctrl-C twice: Interrupt any running program
    sport.write_all("\r\x03\x03".as_bytes())?;

    loop {
        match sport.read(&mut buf) {
            Ok(_) => continue,
            Err(ref e) if e.kind() == ErrorKind::TimedOut => break,
            Err(e) => return Err(e.into()),
        }
    }

    sport.write_all("\r\x01".as_bytes())?;

    read_until(
        &mut *sport,
        "raw REPL; CTRL-B to exit\r\n".as_bytes(),
        false,
    )?;

    sport.write_all("\x04".as_bytes())?;

    read_until(&mut *sport, "soft reboot\r\n".as_bytes(), false)?;
    read_until(
        &mut *sport,
        "raw REPL; CTRL-B to exit\r\n".as_bytes(),
        false,
    )?;

    read_until(&mut *sport, ">".as_bytes(), false)?;

    sport.write_all("\x05A\x01".as_bytes())?;

    sport.read_exact(&mut double_buf)?;
    match double_buf {
        [82, 0] => bail!("Device doesn't support raw-paste"),
        [82, 1] => {}
        _ => bail!("Unknown response"),
    }

    sport.read_exact(&mut double_buf)?;
    let window_size: usize = (double_buf[0] as usize) | (double_buf[1] as usize) << 8;
    let mut window_remain = 0;

    let script_bytes = script.as_bytes();

    let mut i: usize = 0;
    while i < script.len() {
        while window_remain == 0 || sport.bytes_to_read()? > 0 {
            sport.read_exact(&mut byte_buf)?;
            match byte_buf {
                [1] => window_remain += window_size,
                [4] => {
                    sport.write_all("\x04".as_bytes())?;
                    bail!("Device indicated abrupt end.");
                }
                [byte] => bail!("Unexpected error during raw paste: {:?}", byte),
            }
        }

        let chunk_size = min(window_remain, script_bytes.len() - i);

        let mut chunk = vec![0; chunk_size];
        chunk.copy_from_slice(&script_bytes[i..i + chunk_size]);

        sport.write_all(&chunk)?;
        window_remain -= chunk.len();
        i += chunk.len();
    }

    sport.write_all("\x04".as_bytes())?;

    read_until(&mut *sport, "\x04".as_bytes(), false)?;

    read_until(&mut *sport, "\x04".as_bytes(), true)?;

    Ok(())
}

fn main() -> Result<()> {
    let mut ports = find_micropython_ports()?;

    //let script: String = String::from("print('Hello, world!')");

    let script: String = String::from(
        "
from machine import Pin
import time

print('Is this real life?')
print('Let\\'s blink some LEDs babbyyy!')

print('Getting the led pin')
led = Pin('LED', Pin.OUT)



led.off()
for i in range(10):
    print('Toggling led now, iteration {}'.format(i))
    led.toggle()
    time.sleep(0.5)
led.off()",
    );
    match ports.len() {
        0 => eprintln!("No MicroPython ports found"),
        1 => {
            let port = ports.pop().expect("Unable to pop port");
            execute(port, script)?;
        }
        _ => println!("Found multiple MicroPython ports"),
    }
    Ok(())
}
