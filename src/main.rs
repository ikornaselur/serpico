use anyhow::{bail, Result};
use clap::Parser;
use std::fs::File;
use std::io::prelude::*;
use std::path::PathBuf;

use serpico::serial::{execute, find_micropython_devices};

#[derive(Parser, Debug)]
#[clap(author, version, about)]
struct Args {
    /// A file to execute on the MicroPython device
    #[clap(value_parser)]
    file: PathBuf,

    /// An optional device to connect to, if not provided, Serpico will try to discover and use a
    /// a discovered MicroPython device, only if one is found.
    #[clap(short, long)]
    device: Option<PathBuf>,

    /// Optional timeout in seconds to set while waiting to read a message. If no timeout set, then
    /// serpico will wait forever for messages.
    #[clap(short, long)]
    timeout: Option<usize>,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let device = match args.device {
        Some(device) => device,
        None => {
            let mut devices = find_micropython_devices()?;
            match devices.len() {
                0 => bail!("No MicroPython devices founds"),
                1 => {
                    let device = devices.pop().unwrap();
                    println!("MicroPython device discovered at {}", device.display());
                    device
                }
                _ => bail!(
                    "Multiple MicroPython devices found, please specify with the device option"
                ),
            }
        }
    };

    let mut file = match File::open(&args.file.as_path()) {
        Ok(file) => file,
        Err(e) => bail!("Couldn't open file {}: {}", args.file.display(), e),
    };
    let mut content = String::new();
    match file.read_to_string(&mut content) {
        Ok(_) => {}
        Err(e) => bail!("Couldn't read file {}: {}", args.file.display(), e),
    }

    execute(device, content, args.timeout)?;

    Ok(())
}
