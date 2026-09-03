//! Choosing which manifest entries a set of products, platforms, and one SDK
//! version needs, and where each one lands in the output layout.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::manifest::{Blob, Manifest, Pack};
use crate::platform::Platform;

/// The three separately licensed Oodle products Unreal ships.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Product {
    Data,
    Network,
    Texture,
}

impl Product {
    pub const ALL: [Self; 3] = [Self::Data, Self::Network, Self::Texture];

    /// Where the product's SDKs live inside an Unreal checkout.
    #[must_use]
    pub const fn manifest_root(self) -> &'static str {
        match self {
            Self::Data => "Engine/Source/Runtime/OodleDataCompression/Sdks",
            Self::Network => "Engine/Plugins/Compression/OodleNetwork/Sdks",
            Self::Texture => "Engine/Source/Developer/TextureFormatOodle/Sdks",
        }
    }

    /// The product's directory in the output layout.
    #[must_use]
    pub const fn directory(self) -> &'static str {
        match self {
            Self::Data => "data",
            Self::Network => "network",
            Self::Texture => "texture",
        }
    }

    /// Library stems that belong to this product.
    ///
    /// This doubles as the filter that drops `oo2ext`, the shared extension
    /// library Unreal ships next to Oodle Data but which no product owns.
    #[must_use]
    pub const fn library_stems(self) -> &'static [&'static str] {
        match self {
            Self::Data => &["oo2core"],
            Self::Network => &["oo2net"],
            Self::Texture => &["oo2tex", "oo2texrt"],
        }
    }
}

impl fmt::Display for Product {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.directory())
    }
}

/// One file to fetch, joined across the manifest's three tables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedFile {
    pub product: Product,
    pub platform: Platform,
    /// The manifest's repository-relative name.
    pub source: String,
    /// Destination relative to the output root.
    pub destination: PathBuf,
    /// SHA-1 the extracted bytes must hash to.
    pub hash: String,
    pub blob: Blob,
    pub pack: Pack,
}

/// Everything one run will fetch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub version: String,
    pub files: Vec<PlannedFile>,
}

impl Plan {
    /// The packs to download, deduplicated; several files often share one.
    #[must_use]
    pub fn packs(&self) -> Vec<&Pack> {
        let mut packs = Vec::new();
        for file in &self.files {
            if !packs.iter().any(|pack: &&Pack| pack.hash == file.pack.hash) {
                packs.push(&file.pack);
            }
        }
        packs
    }

    #[must_use]
    pub fn pack_count(&self) -> usize {
        self.packs().len()
    }

    /// Bytes that come off the network: whole packs, not just wanted blobs.
    #[must_use]
    pub fn download_size(&self) -> u64 {
        self.packs().iter().map(|pack| pack.compressed_size).sum()
    }

    /// Bytes that will be written, before pack compression.
    #[must_use]
    pub fn extracted_size(&self) -> u64 {
        self.files.iter().map(|file| file.blob.size).sum()
    }

    /// Products the plan touches, in declaration order.
    #[must_use]
    pub fn products(&self) -> Vec<Product> {
        let mut products = self
            .files
            .iter()
            .map(|file| file.product)
            .collect::<Vec<_>>();
        products.sort_unstable();
        products.dedup();
        products
    }
}

#[derive(Debug, Error)]
pub enum SelectionError {
    #[error("the manifest lists no {product} libraries for {platform} at SDK version {version}")]
    NoFiles {
        product: Product,
        platform: Platform,
        version: String,
    },
    #[error("the manifest has no blob {hash} for {name}")]
    UnknownBlob { name: String, hash: String },
    #[error("the manifest has no pack {hash} for {name}")]
    UnknownPack { name: String, hash: String },
}

/// Join the manifest's tables into the set of files these products need on
/// these platforms.
///
/// # Errors
///
/// Returns [`SelectionError`] when a product/platform pair is absent at
/// `version`, or when the manifest's blob or pack table is missing an entry a
/// selected file points at.
pub fn select(
    manifest: &Manifest,
    version: &str,
    products: &[Product],
    platforms: &[Platform],
) -> Result<Plan, SelectionError> {
    let blobs = manifest
        .blobs()
        .iter()
        .map(|blob| (blob.hash.as_str(), blob))
        .collect::<HashMap<_, _>>();
    let packs = manifest
        .packs()
        .iter()
        .map(|pack| (pack.hash.as_str(), pack))
        .collect::<HashMap<_, _>>();

    let mut files = Vec::new();
    for &product in products {
        let prefix = format!("{}/{version}/", product.manifest_root());
        for &platform in platforms {
            let matches = manifest
                .files()
                .iter()
                .filter_map(|file| {
                    let remainder = file.name.strip_prefix(&prefix)?;
                    let destination = destination_for(product, platform, remainder)?;
                    Some((file, destination))
                })
                .collect::<Vec<_>>();
            if matches.is_empty() {
                return Err(SelectionError::NoFiles {
                    product,
                    platform,
                    version: version.to_owned(),
                });
            }
            for (file, destination) in matches {
                let Some(blob) = blobs.get(file.hash.as_str()) else {
                    return Err(SelectionError::UnknownBlob {
                        name: file.name.clone(),
                        hash: file.hash.clone(),
                    });
                };
                let Some(pack) = packs.get(blob.pack_hash.as_str()) else {
                    return Err(SelectionError::UnknownPack {
                        name: file.name.clone(),
                        hash: blob.pack_hash.clone(),
                    });
                };
                files.push(PlannedFile {
                    product,
                    platform,
                    source: file.name.clone(),
                    destination,
                    hash: file.hash.clone(),
                    blob: (*blob).clone(),
                    pack: (*pack).clone(),
                });
            }
        }
    }

    Ok(Plan {
        version: version.to_owned(),
        files,
    })
}

/// Decide where a manifest entry lands, or `None` when it is not wanted.
///
/// `remainder` is the manifest name with the `<product root>/<version>/` prefix
/// already stripped, so it reads `lib/<Platform>/<file>` or
/// `redist/<Platform>/<file>`. `include/` is skipped here because headers are
/// copied from the checkout rather than fetched.
fn destination_for(product: Product, platform: Platform, remainder: &str) -> Option<PathBuf> {
    let mut segments = remainder.split('/');
    let kind = match segments.next()? {
        "lib" => "lib",
        "redist" => "bin",
        _ => return None,
    };
    if segments.next()? != platform.directory() {
        return None;
    }
    let name = segments.next()?;
    if segments.next().is_some() {
        return None;
    }
    if !is_shipping_library(product, name) {
        return None;
    }
    Some(Path::new(product.directory()).join(kind).join(name))
}

/// A shipping library for `product`: right product, not a debug build, not an
/// iOS/tvOS simulator slice.
fn is_shipping_library(product: Product, name: &str) -> bool {
    let stem = name.strip_prefix("lib").unwrap_or(name);
    product
        .library_stems()
        .iter()
        .any(|prefix| stem.starts_with(prefix))
        && !name.contains("_dbg")
        && !name.contains("_debug")
        && !name.contains(".sim.")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every shape the selector has to rule on: the three products, the four
    /// desktop platform directories, the `oo2ext` sibling, debug variants,
    /// simulator variants, and a second SDK version.
    const SELECTION_MANIFEST: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<DependencyManifest BaseUrl="https://cdn.unrealengine.com/dependencies">
  <Files>
    <File Name="Engine/Source/Runtime/OodleDataCompression/Sdks/2.9.16/lib/Win64/oo2core_win64.lib" Hash="f0" />
    <File Name="Engine/Source/Runtime/OodleDataCompression/Sdks/2.9.16/lib/Win64/oo2core_win64_debug.lib" Hash="f0" />
    <File Name="Engine/Source/Runtime/OodleDataCompression/Sdks/2.9.16/lib/Win64/oo2ext_win64.lib" Hash="f0" />
    <File Name="Engine/Source/Runtime/OodleDataCompression/Sdks/2.9.16/lib/Linux/liboo2corelinux64.a" Hash="f0" />
    <File Name="Engine/Source/Runtime/OodleDataCompression/Sdks/2.9.16/lib/Linux/liboo2corelinux64.so.9" Hash="f0" />
    <File Name="Engine/Source/Runtime/OodleDataCompression/Sdks/2.9.16/lib/Linux/liboo2corelinux64_dbg.a" Hash="f0" />
    <File Name="Engine/Source/Runtime/OodleDataCompression/Sdks/2.9.16/lib/Linux/liboo2extlinux64.a" Hash="f0" />
    <File Name="Engine/Source/Runtime/OodleDataCompression/Sdks/2.9.16/lib/LinuxArm64/liboo2corelinuxarm64.a" Hash="f0" />
    <File Name="Engine/Source/Runtime/OodleDataCompression/Sdks/2.9.16/lib/Mac/liboo2coremac64.a" Hash="f0" />
    <File Name="Engine/Source/Runtime/OodleDataCompression/Sdks/2.9.16/lib/Mac/liboo2coremac64.2.9.16.dylib" Hash="f0" />
    <File Name="Engine/Source/Runtime/OodleDataCompression/Sdks/2.9.16/lib/IOS/liboo2coreios.sim.a" Hash="f0" />
    <File Name="Engine/Source/Runtime/OodleDataCompression/Sdks/2.9.15/lib/Win64/oo2core_win64.lib" Hash="f0" />
    <File Name="Engine/Plugins/Compression/OodleNetwork/Sdks/2.9.16/lib/Win64/oo2net_win64.lib" Hash="f0" />
    <File Name="Engine/Plugins/Compression/OodleNetwork/Sdks/2.9.16/lib/Win64/oo2net_win64_debug.lib" Hash="f0" />
    <File Name="Engine/Plugins/Compression/OodleNetwork/Sdks/2.9.16/lib/Mac/liboo2netmac64.a" Hash="f0" />
    <File Name="Engine/Plugins/Compression/OodleNetwork/Sdks/2.9.16/lib/Linux/liboo2netlinux64.a" Hash="f0" />
    <File Name="Engine/Plugins/Compression/OodleNetwork/Sdks/2.9.16/lib/Linux/liboo2netlinux64_dbg.a" Hash="f0" />
    <File Name="Engine/Plugins/Compression/OodleNetwork/Sdks/2.9.16/lib/LinuxArm64/liboo2netlinuxarm64.a" Hash="f0" />
    <File Name="Engine/Source/Developer/TextureFormatOodle/Sdks/2.9.16/lib/Linux/liboo2texlinux64.a" Hash="f0" />
    <File Name="Engine/Source/Developer/TextureFormatOodle/Sdks/2.9.16/lib/Linux/liboo2texrtlinux64.so.9" Hash="f0" />
    <File Name="Engine/Source/Developer/TextureFormatOodle/Sdks/2.9.16/lib/LinuxArm64/liboo2texlinuxarm64.a" Hash="f0" />
    <File Name="Engine/Source/Developer/TextureFormatOodle/Sdks/2.9.16/lib/Mac/liboo2texmac64.a" Hash="f0" />
    <File Name="Engine/Source/Developer/TextureFormatOodle/Sdks/2.9.16/lib/Win64/oo2tex_win64.lib" Hash="f0" />
    <File Name="Engine/Source/Developer/TextureFormatOodle/Sdks/2.9.16/lib/Win64/oo2texrt_win64.lib" Hash="f0" />
    <File Name="Engine/Source/Developer/TextureFormatOodle/Sdks/2.9.16/lib/Win64/oo2tex_win64_debug.lib" Hash="f0" />
    <File Name="Engine/Source/Developer/TextureFormatOodle/Sdks/2.9.16/redist/Win64/oo2tex_win64_2.9.16.dll" Hash="f0" />
    <File Name="Engine/Source/Developer/TextureFormatOodle/Sdks/2.9.16/redist/Win64/oo2texrt_win64_2.9.16.dll" Hash="f0" />
    <File Name="Engine/Source/Developer/TextureFormatOodle/Sdks/2.9.16/redist/Mac/liboo2texmac64.2.9.16.dylib" Hash="f0" />
  </Files>
  <Blobs>
    <Blob Hash="f0" Size="16" PackHash="p0" PackOffset="0" />
  </Blobs>
  <Packs>
    <Pack Hash="p0" Size="16" CompressedSize="12" RemotePath="UnrealEngine-1" />
  </Packs>
</DependencyManifest>
"#;

    fn selected_sources(plan: &Plan) -> Vec<&str> {
        plan.files.iter().map(|file| file.source.as_str()).collect()
    }

    #[test]
    fn select_takes_one_product_on_one_platform() {
        let manifest = Manifest::parse(SELECTION_MANIFEST).expect("fixture parses");
        let plan = select(&manifest, "2.9.16", &[Product::Data], &[Platform::Win64])
            .expect("data/win64 resolves");

        assert_eq!(
            selected_sources(&plan),
            ["Engine/Source/Runtime/OodleDataCompression/Sdks/2.9.16/lib/Win64/oo2core_win64.lib"]
        );
        assert_eq!(
            plan.files[0].destination,
            Path::new("data").join("lib").join("oo2core_win64.lib")
        );
        assert_eq!(plan.pack_count(), 1);
        assert_eq!(plan.download_size(), 12);
        assert_eq!(plan.extracted_size(), 16);
    }

    #[test]
    fn select_excludes_debug_simulator_and_extension_libraries() {
        let manifest = Manifest::parse(SELECTION_MANIFEST).expect("fixture parses");
        let plan = select(&manifest, "2.9.16", &Product::ALL, &Platform::ALL)
            .expect("every product resolves");

        let sources = selected_sources(&plan);
        assert!(
            !sources
                .iter()
                .any(|source| source.contains("_debug") || source.contains("_dbg")),
            "debug variants leaked: {sources:?}"
        );
        assert!(
            !sources.iter().any(|source| source.contains(".sim.")),
            "simulator variants leaked: {sources:?}"
        );
        assert!(
            !sources.iter().any(|source| source.contains("oo2ext")),
            "the extension library leaked: {sources:?}"
        );
        assert!(
            !sources.iter().any(|source| source.contains("2.9.15")),
            "another SDK version leaked: {sources:?}"
        );
        assert!(
            !sources.iter().any(|source| source.contains("/IOS/")),
            "an unrequested platform leaked: {sources:?}"
        );
    }

    #[test]
    fn select_maps_redist_libraries_into_the_product_bin_directory() {
        let manifest = Manifest::parse(SELECTION_MANIFEST).expect("fixture parses");
        let plan = select(&manifest, "2.9.16", &[Product::Texture], &[Platform::Win64])
            .expect("texture/win64 resolves");

        let mut destinations = plan
            .files
            .iter()
            .map(|file| file.destination.clone())
            .collect::<Vec<_>>();
        destinations.sort();
        assert_eq!(
            destinations,
            [
                Path::new("texture")
                    .join("bin")
                    .join("oo2tex_win64_2.9.16.dll"),
                Path::new("texture")
                    .join("bin")
                    .join("oo2texrt_win64_2.9.16.dll"),
                Path::new("texture").join("lib").join("oo2tex_win64.lib"),
                Path::new("texture").join("lib").join("oo2texrt_win64.lib"),
            ]
        );
    }

    #[test]
    fn select_rejects_a_version_the_manifest_does_not_carry() {
        let manifest = Manifest::parse(SELECTION_MANIFEST).expect("fixture parses");
        let error = select(&manifest, "2.9.13", &[Product::Data], &[Platform::Win64])
            .expect_err("an absent version is an error");

        assert!(
            matches!(error, SelectionError::NoFiles { .. }),
            "unexpected error: {error}"
        );
    }
}
