//! Fetch the Oodle SDKs out of a licensed Unreal Engine checkout.

use std::path::PathBuf;

use anyhow::{Context as _, bail};
use az_filesystem::AzothDataHome;
use clap::{Parser, ValueEnum};
use oodle_fetch::{
    Manifest, PackFetchError, PackSource, Platform, PlatformRequest, Product, copy_headers,
    materialize, resolve_platforms, select,
};

/// The SDK version Unreal 5.7 and Azoth are known to work with.
const DEFAULT_SDK_VERSION: &str = "2.9.16";

/// Unreal's dependency manifest, relative to the checkout root.
const MANIFEST_PATH: &str = "Engine/Build/Commit.gitdeps.xml";

#[derive(Debug, Parser)]
#[command(
    name = "oodle-fetch",
    about = "Materialize the Oodle Data, Network, and Texture SDKs from an Unreal Engine checkout",
    long_about = None,
)]
struct Arguments {
    /// Root of a licensed Unreal Engine source checkout. It is only read.
    #[arg(long, value_name = "DIR")]
    unreal_root: PathBuf,

    /// Oodle SDK version to take out of the checkout.
    #[arg(long, value_name = "VERSION", default_value = DEFAULT_SDK_VERSION)]
    sdk_version: String,

    /// Platform to fetch libraries for; repeat for several. Defaults to this host.
    #[arg(long, value_name = "PLATFORM")]
    platform: Vec<PlatformArgument>,

    /// Output root. Defaults to the Azoth data home's `oodle/<version>`.
    #[arg(long, value_name = "DIR")]
    destination: Option<PathBuf>,

    /// List what would be fetched without downloading or writing anything.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum PlatformArgument {
    Win64,
    Linux,
    LinuxArm64,
    Mac,
    All,
}

impl From<PlatformArgument> for PlatformRequest {
    fn from(argument: PlatformArgument) -> Self {
        match argument {
            PlatformArgument::Win64 => Self::One(Platform::Win64),
            PlatformArgument::Linux => Self::One(Platform::Linux),
            PlatformArgument::LinuxArm64 => Self::One(Platform::LinuxArm64),
            PlatformArgument::Mac => Self::One(Platform::Mac),
            PlatformArgument::All => Self::All,
        }
    }
}

/// Epic's CDN, which serves each pack as one gzip stream.
struct CdnPacks {
    base_url: String,
    client: reqwest::blocking::Client,
}

impl CdnPacks {
    fn new(base_url: &str) -> anyhow::Result<Self> {
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            client: reqwest::blocking::Client::builder()
                .build()
                .context("could not build an HTTP client")?,
        })
    }
}

impl PackSource for CdnPacks {
    fn pack(&self, remote_path: &str, hash: &str) -> Result<Vec<u8>, PackFetchError> {
        let url = format!("{}/{remote_path}/{hash}", self.base_url);
        println!("  fetching {url}");
        let transport = |error: reqwest::Error| PackFetchError::Transport {
            remote_path: remote_path.to_owned(),
            hash: hash.to_owned(),
            reason: error.to_string(),
        };

        let response = self.client.get(&url).send().map_err(transport)?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(PackFetchError::Missing {
                remote_path: remote_path.to_owned(),
                hash: hash.to_owned(),
            });
        }
        let bytes = response
            .error_for_status()
            .map_err(transport)?
            .bytes()
            .map_err(transport)?;
        Ok(bytes.to_vec())
    }
}

fn main() -> anyhow::Result<()> {
    let arguments = Arguments::parse();

    let manifest_path = arguments.unreal_root.join(MANIFEST_PATH);
    if !manifest_path.is_file() {
        bail!(
            "no Unreal dependency manifest at {}; --unreal-root must point at an Unreal Engine \
             source checkout",
            manifest_path.display()
        );
    }
    let document = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("could not read {}", manifest_path.display()))?;
    let manifest = Manifest::parse(&document)
        .with_context(|| format!("could not parse {}", manifest_path.display()))?;

    let requests = arguments
        .platform
        .iter()
        .copied()
        .map(PlatformRequest::from)
        .collect::<Vec<_>>();
    let platforms = resolve_platforms(&requests, std::env::consts::OS, std::env::consts::ARCH)?;
    let plan = select(&manifest, &arguments.sdk_version, &Product::ALL, &platforms)?;

    let destination = arguments.destination.unwrap_or_else(|| {
        AzothDataHome::resolve()
            .root()
            .join("oodle")
            .join(&arguments.sdk_version)
    });

    let platform_list = platforms
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    println!(
        "Oodle {} for {platform_list}: {} files, {} bytes, from {} pack(s) totalling {} bytes to \
         download",
        arguments.sdk_version,
        plan.files.len(),
        plan.extracted_size(),
        plan.pack_count(),
        plan.download_size()
    );

    if arguments.dry_run {
        for file in &plan.files {
            println!(
                "  {} -> {} ({} bytes)",
                file.source,
                file.destination.display(),
                file.blob.size
            );
        }
        println!("Dry run: nothing was downloaded or written.");
        return Ok(());
    }

    let headers = copy_headers(
        &arguments.unreal_root,
        &arguments.sdk_version,
        &plan.products(),
        &destination,
    )?;
    let written = materialize(&plan, &CdnPacks::new(&manifest.base_url)?, &destination)?;

    println!(
        "Wrote {} libraries and {} headers under {}",
        written.len(),
        headers.len(),
        destination.display()
    );
    for product in plan.products() {
        println!(
            "  {product}: {}",
            destination.join(product.directory()).display()
        );
    }
    println!(
        "Set OODLE_LIB_DIR={}",
        destination.join("data").join("lib").display()
    );

    Ok(())
}
