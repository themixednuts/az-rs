//! Extract one decompressed pak entry to an explicit file.

use std::path::Path;

use az_pak::PakFile;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os();
    let executable = args.next().unwrap_or_default();
    let usage = || {
        format!(
            "usage: {} <pak-path> <entry-path> <output-path>",
            Path::new(&executable).display()
        )
    };
    let pak_path = args.next().ok_or_else(&usage)?;
    let entry_path = args.next().ok_or_else(&usage)?;
    let output_path = args.next().ok_or_else(&usage)?;
    if args.next().is_some() {
        return Err("pak-extract accepts exactly one pak, entry, and output path".into());
    }
    let entry_path = entry_path
        .to_str()
        .ok_or("pak entry path must be valid UTF-8")?;
    let mut pak = PakFile::open(pak_path)?;
    let bytes = pak.read(entry_path)?;
    std::fs::write(output_path, bytes)?;
    Ok(())
}
