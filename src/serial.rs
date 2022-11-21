use anyhow::{bail, Result};
use serialport::{SerialPort, SerialPortType};
use std::char;
use std::cmp::min;
use std::collections::VecDeque;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::thread::sleep;
use std::time::Duration;

const BUFFER_SIZE: usize = 16;

pub fn find_micropython_devices() -> Result<Vec<PathBuf>> {
    let ports = serialport::available_ports()?;
    let mut micropython_ports: Vec<PathBuf> = Vec::new();
    for p in ports {
        if p.port_name.starts_with("/dev/cu.") {
            // Skip /dev/cu.X devices in MacOS
            continue;
        }
        if let SerialPortType::UsbPort(info) = p.port_type {
            if let Some(manufacturer) = info.manufacturer {
                println!("{}: {}", p.port_name, manufacturer);
                if manufacturer == "MicroPython" {
                    micropython_ports.push(PathBuf::from(p.port_name));
                }
            }
        }
    }

    Ok(micropython_ports)
}

fn read_until(
    port: &mut dyn SerialPort,
    bytes: &[u8],
    echo: bool,
    timeout: Option<usize>,
) -> Result<()> {
    let mut deque: VecDeque<u8> = VecDeque::from(vec![0; bytes.len()]);
    let mut buf: Vec<u8> = vec![0; 1];

    let sleep_time = Duration::from_millis(10);
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
                if let Some(timeout) = timeout {
                    timeout_count += 1;
                    if timeout_count > timeout * 100 {
                        bail!("Timed out in read_until");
                    }
                }
                sleep(sleep_time);
            }
            Err(e) => bail!(e),
            _ => bail!("Unhandled state"),
        }
    }
    Ok(())
}

pub fn execute(device: PathBuf, script: String, timeout: Option<usize>) -> Result<()> {
    let device_path = match device.into_os_string().into_string() {
        Ok(path) => path,
        Err(e) => bail!("Unable to convert path to string: {:?}", e),
    };
    let mut port = serialport::new(device_path, 115_200)
        .timeout(Duration::from_millis(10))
        .open()?;

    let mut buf: Vec<u8> = vec![0; BUFFER_SIZE];
    let mut byte_buf = [0; 1];
    let mut double_buf = [0; 2];

    // Ctrl-C twice: Interrupt any running program
    port.write_all("\r\x03\x03".as_bytes())?;

    loop {
        match port.read(&mut buf) {
            Ok(_) => continue,
            Err(ref e) if e.kind() == ErrorKind::TimedOut => break,
            Err(e) => return Err(e.into()),
        }
    }

    port.write_all("\r\x01".as_bytes())?;

    read_until(
        &mut *port,
        "raw REPL; CTRL-B to exit\r\n".as_bytes(),
        false,
        timeout,
    )?;

    port.write_all("\x04".as_bytes())?;

    read_until(&mut *port, "soft reboot\r\n".as_bytes(), false, timeout)?;
    read_until(
        &mut *port,
        "raw REPL; CTRL-B to exit\r\n".as_bytes(),
        false,
        timeout,
    )?;

    read_until(&mut *port, ">".as_bytes(), false, timeout)?;

    port.write_all("\x05A\x01".as_bytes())?;

    port.read_exact(&mut double_buf)?;
    match double_buf {
        [82, 0] => bail!("Device doesn't support raw-paste"),
        [82, 1] => {}
        _ => bail!("Unknown response"),
    }

    port.read_exact(&mut double_buf)?;
    let window_size: usize = (double_buf[0] as usize) | (double_buf[1] as usize) << 8;
    let mut window_remain = 0;

    let script_bytes = script.as_bytes();

    let mut i: usize = 0;
    while i < script.len() {
        while window_remain == 0 || port.bytes_to_read()? > 0 {
            match port.read_exact(&mut byte_buf) {
                Ok(_) => (),
                Err(ref e) if e.kind() == ErrorKind::TimedOut => (),
                Err(e) => bail!("Unable to read from port: {:?}", e),
            }

            match byte_buf {
                [1] => window_remain += window_size,
                [4] => {
                    port.write_all("\x04".as_bytes())?;
                    bail!("Device indicated abrupt end.");
                }
                [byte] => bail!("Unexpected error during raw paste: {:?}", byte),
            }
        }

        let chunk_size = min(window_remain, script_bytes.len() - i);

        let mut chunk = vec![0; chunk_size];
        chunk.copy_from_slice(&script_bytes[i..i + chunk_size]);

        port.write_all(&chunk)?;
        window_remain -= chunk.len();
        i += chunk.len();
    }

    port.write_all("\x04".as_bytes())?;

    read_until(&mut *port, "\x04".as_bytes(), false, timeout)?;

    // stdout
    read_until(&mut *port, "\x04".as_bytes(), true, timeout)?;

    // stderr
    read_until(&mut *port, "\x04".as_bytes(), true, timeout)?;

    Ok(())
}
