#[cfg(windows)]
mod capture;
mod pixel;

use std::{
    fs,
    io::{self, IsTerminal as _},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use clap::{ArgAction, Parser, Subcommand, ValueEnum};
use serde::Serialize;
use tracing_subscriber::EnvFilter;

use crate::pixel::{LAYOUT_CORNER, Rect, color_bounds_bgra};

#[derive(Debug, Parser)]
#[command(
    version,
    about = "Capture and compare Windows Graphics Capture viewport frames",
    after_help = "Environment:\n  RUST_LOG  Layer tracing directives over -v/--verbose or -q/--quiet.\n  NO_COLOR  Disable automatic color output."
)]
struct Cli {
    /// Increase default log verbosity (`-v` info, `-vv` debug, `-vvv` trace).
    #[arg(short, long, action = ArgAction::Count, global = true)]
    verbose: u8,

    /// Restrict default diagnostics to errors. `RUST_LOG` can add directives.
    #[arg(short, long, conflicts_with = "verbose", global = true)]
    quiet: bool,

    /// When to colorize diagnostic output.
    #[arg(long, value_enum, default_value_t = ColorArg::Auto, global = true)]
    color: ColorArg,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Capture a composed window through Windows Graphics Capture and analyze
    /// each display-cadence frame in memory. Windows only.
    Capture {
        /// Unique substring of the target window title.
        #[arg(long, default_value = "Aether")]
        title: String,
        /// Requested capture cadence in frames per second.
        #[arg(long, default_value_t = 120)]
        fps: u32,
        /// Number of display-cadence frames to capture.
        #[arg(long, default_value_t = 1200)]
        frames: u32,
        /// Directory for the JSON report and diagnostic PNGs.
        #[arg(long = "out-dir", value_name = "DIR")]
        out_dir: PathBuf,
        /// Replace capture artifacts that already exist in the output directory.
        #[arg(long)]
        force: bool,
        /// Resolve the target window and validate output paths without starting capture.
        #[arg(long)]
        dry_run: bool,
    },
    /// Compare two WGC PNGs while excluding the diagnostic viewport rectangle.
    Compare {
        /// Baseline PNG.
        #[arg(long)]
        before: PathBuf,
        /// Candidate PNG.
        #[arg(long)]
        after: PathBuf,
        /// Maximum per-channel delta treated as unchanged.
        #[arg(long, default_value_t = 2)]
        tolerance: u8,
        /// Optional JSON report file. Omit it to print JSON to standard output.
        #[arg(long = "out", value_name = "FILE")]
        out: Option<PathBuf>,
        /// Replace an existing JSON report file.
        #[arg(long, requires = "out")]
        force: bool,
        /// Compare the images and print the report without writing the output file.
        #[arg(long, requires = "out")]
        dry_run: bool,
    },
}

#[derive(Debug, Clone, Copy, Default, ValueEnum)]
enum ColorArg {
    /// Colorize diagnostics only on a terminal when `NO_COLOR` is unset.
    #[default]
    Auto,
    /// Always colorize diagnostic output.
    Always,
    /// Never colorize diagnostic output.
    Never,
}

impl ColorArg {
    fn stderr_ansi(self) -> bool {
        match self {
            Self::Auto => std::env::var_os("NO_COLOR").is_none() && io::stderr().is_terminal(),
            Self::Always => true,
            Self::Never => false,
        }
    }
}

const fn default_log_directives(verbosity: u8, quiet: bool) -> &'static str {
    match (quiet, verbosity) {
        (true, _) => "error",
        (false, 0) => "warn",
        (false, 1) => "info",
        (false, 2) => "debug",
        (false, _) => "trace",
    }
}

fn install_logging(verbosity: u8, quiet: bool, color: ColorArg) {
    let default = default_log_directives(verbosity, quiet);
    let filter = std::env::var_os("RUST_LOG").map_or_else(
        || EnvFilter::new(default),
        |rust_log| {
            let rust_log = rust_log.to_string_lossy();
            EnvFilter::try_new(format!("{default},{rust_log}")).unwrap_or_else(|error| {
                eprintln!("warn: ignoring invalid RUST_LOG directives ({error})");
                EnvFilter::new(default)
            })
        },
    );
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(color.stderr_ansi())
        .with_writer(io::stderr)
        .init();
}

#[derive(Debug, Serialize)]
struct CompareReport {
    width: u32,
    height: u32,
    compared_pixels: u64,
    changed_pixels: u64,
    max_channel_delta: u8,
    excluded_rect: Option<Rect>,
    pixel_identical: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    install_logging(cli.verbose, cli.quiet, cli.color);
    tracing::info!(command = ?cli.command, "starting viewport compositor utility");
    match cli.command {
        #[cfg(windows)]
        Command::Capture {
            title,
            fps,
            frames,
            out_dir,
            force,
            dry_run,
        } => capture::run(&title, fps, frames, &out_dir, force, dry_run),
        #[cfg(not(windows))]
        Command::Capture {
            title,
            fps,
            frames,
            out_dir,
            force,
            dry_run,
        } => capture_is_windows_only(&title, fps, frames, &out_dir, force, dry_run),
        Command::Compare {
            before,
            after,
            tolerance,
            out,
            force,
            dry_run,
        } => compare(&before, &after, tolerance, out.as_deref(), force, dry_run),
    }
}

/// Windows Graphics Capture has no counterpart on other platforms, so the
/// request is refused up front with a status of its own. A caller can tell that
/// apart from status 1, which means a capture started and then failed.
#[cfg(not(windows))]
fn capture_is_windows_only(
    title: &str,
    fps: u32,
    frames: u32,
    out_dir: &Path,
    force: bool,
    dry_run: bool,
) -> ! {
    tracing::debug!(
        title,
        fps,
        frames,
        out_dir = %out_dir.display(),
        force,
        dry_run,
        "refused capture request"
    );
    eprintln!(
        "error: `capture` only runs on Windows because it records through Windows Graphics Capture. `compare` runs on every platform."
    );
    std::process::exit(2)
}

fn compare(
    before: &Path,
    after: &Path,
    tolerance: u8,
    output: Option<&Path>,
    force: bool,
    dry_run: bool,
) -> Result<()> {
    let before_image = image::open(before)
        .with_context(|| format!("opening {}", before.display()))?
        .to_rgba8();
    let after_image = image::open(after)
        .with_context(|| format!("opening {}", after.display()))?
        .to_rgba8();
    if before_image.dimensions() != after_image.dimensions() {
        bail!(
            "image dimensions differ: {:?} vs {:?}",
            before_image.dimensions(),
            after_image.dimensions()
        );
    }
    let (width, height) = before_image.dimensions();
    let before_bgra = rgba_to_bgra(before_image.as_raw());
    let after_bgra = rgba_to_bgra(after_image.as_raw());
    let excluded_rect = color_bounds_bgra(&after_bgra, width, height, LAYOUT_CORNER, 18)
        .or_else(|| color_bounds_bgra(&before_bgra, width, height, LAYOUT_CORNER, 18));
    let mut compared_pixels = 0_u64;
    let mut changed_pixels = 0_u64;
    let mut max_channel_delta = 0_u8;
    for y in 0..height {
        for x in 0..width {
            if excluded_rect.is_some_and(|rect| rect.contains(x, y)) {
                continue;
            }
            compared_pixels += 1;
            let left = before_image.get_pixel(x, y).0;
            let right = after_image.get_pixel(x, y).0;
            let delta = left
                .into_iter()
                .zip(right)
                .map(|(a, b)| a.abs_diff(b))
                .max()
                .unwrap_or(0);
            max_channel_delta = max_channel_delta.max(delta);
            if delta > tolerance {
                changed_pixels += 1;
            }
        }
    }
    let report = CompareReport {
        width,
        height,
        compared_pixels,
        changed_pixels,
        max_channel_delta,
        excluded_rect,
        pixel_identical: changed_pixels == 0,
    };
    let json = serde_json::to_vec_pretty(&report)?;
    if let Some(output) = output {
        guard_existing(output, force)?;
        if !dry_run {
            fs::write(output, &json).with_context(|| format!("writing {}", output.display()))?;
        }
    }
    println!("{}", String::from_utf8(json)?);
    Ok(())
}

pub(crate) fn guard_existing(path: &Path, force: bool) -> Result<()> {
    if path.exists() && !force {
        bail!(
            "output already exists: {} (use --force to replace it)",
            path.display()
        );
    }
    Ok(())
}

fn rgba_to_bgra(rgba: &[u8]) -> Vec<u8> {
    let mut bgra = Vec::with_capacity(rgba.len());
    for pixel in rgba.chunks_exact(4) {
        bgra.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
    }
    bgra
}
