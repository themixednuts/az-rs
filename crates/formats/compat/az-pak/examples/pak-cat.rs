//! Write one decompressed pak entry to stdout.

use std::io::{self, Write};

use az_pak::PakFile;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os();
    let executable = args.next().unwrap_or_default();
    let Some(pak_path) = args.next() else {
        return Err(format!(
            "usage: {} <pak-path> <entry-path>",
            std::path::Path::new(&executable).display()
        )
        .into());
    };
    let Some(entry_path) = args.next() else {
        return Err(format!(
            "usage: {} <pak-path> <entry-path>",
            std::path::Path::new(&executable).display()
        )
        .into());
    };
    if args.next().is_some() {
        return Err("pak-cat accepts exactly one pak path and one entry path".into());
    }
    let entry_path = entry_path
        .to_str()
        .ok_or("pak entry path must be valid UTF-8")?;
    let mut pak = PakFile::open(pak_path)?;
    let bytes = pak.read(entry_path)?;
    io::stdout().lock().write_all(&bytes)?;
    Ok(())
}
