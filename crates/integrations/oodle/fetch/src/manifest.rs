//! Parsing for Unreal's `Commit.gitdeps.xml` dependency manifest.

use serde::Deserialize;
use thiserror::Error;

/// Unreal's dependency manifest, reduced to the three tables this tool joins.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename = "DependencyManifest")]
pub struct Manifest {
    #[serde(rename = "@BaseUrl")]
    pub base_url: String,
    #[serde(rename = "Files")]
    files_table: FilesTable,
    #[serde(rename = "Blobs")]
    blobs_table: BlobsTable,
    #[serde(rename = "Packs")]
    packs_table: PacksTable,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct FilesTable {
    #[serde(default, rename = "File")]
    entries: Vec<ManifestFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct BlobsTable {
    #[serde(default, rename = "Blob")]
    entries: Vec<Blob>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct PacksTable {
    #[serde(default, rename = "Pack")]
    entries: Vec<Pack>,
}

/// One repository-relative file and the SHA-1 its contents must hash to.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ManifestFile {
    #[serde(rename = "@Name")]
    pub name: String,
    #[serde(rename = "@Hash")]
    pub hash: String,
}

/// A file's bytes, addressed as a window into one decompressed pack.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Blob {
    #[serde(rename = "@Hash")]
    pub hash: String,
    #[serde(rename = "@Size")]
    pub size: u64,
    #[serde(rename = "@PackHash")]
    pub pack_hash: String,
    #[serde(rename = "@PackOffset")]
    pub pack_offset: u64,
}

/// A downloadable gzip pack holding many blobs back to back.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Pack {
    #[serde(rename = "@Hash")]
    pub hash: String,
    #[serde(rename = "@Size")]
    pub size: u64,
    #[serde(rename = "@CompressedSize")]
    pub compressed_size: u64,
    #[serde(rename = "@RemotePath")]
    pub remote_path: String,
}

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("could not parse the Unreal dependency manifest: {0}")]
    Parse(#[from] quick_xml::DeError),
}

impl Manifest {
    /// Parse `Commit.gitdeps.xml` from memory.
    ///
    /// # Errors
    ///
    /// Returns [`ManifestError::Parse`] when the document is not a dependency
    /// manifest with the expected attributes.
    pub fn parse(document: &str) -> Result<Self, ManifestError> {
        Ok(quick_xml::de::from_str(document)?)
    }

    #[must_use]
    pub fn files(&self) -> &[ManifestFile] {
        &self.files_table.entries
    }

    #[must_use]
    pub fn blobs(&self) -> &[Blob] {
        &self.blobs_table.entries
    }

    #[must_use]
    pub fn packs(&self) -> &[Pack] {
        &self.packs_table.entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<DependencyManifest xmlns:xsd="http://www.w3.org/2001/XMLSchema" BaseUrl="https://cdn.unrealengine.com/dependencies">
  <Files>
    <File Name="Engine/Source/Runtime/OodleDataCompression/Sdks/2.9.16/lib/Win64/oo2core_win64.lib" Hash="1111111111111111111111111111111111111111" />
    <File Name="Engine/Source/Runtime/OodleDataCompression/Sdks/2.9.16/lib/Linux/liboo2corelinux64.a" Hash="2222222222222222222222222222222222222222" />
  </Files>
  <Blobs>
    <Blob Hash="1111111111111111111111111111111111111111" Size="174074" PackHash="aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" PackOffset="34060" />
  </Blobs>
  <Packs>
    <Pack Hash="aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" Size="2005632" CompressedSize="361451" RemotePath="UnrealEngine-25328963" />
  </Packs>
</DependencyManifest>
"#;

    #[test]
    fn manifest_parse_reads_files_blobs_and_packs() {
        let manifest = Manifest::parse(MANIFEST).expect("fixture parses");

        assert_eq!(
            manifest.base_url,
            "https://cdn.unrealengine.com/dependencies"
        );
        assert_eq!(manifest.files().len(), 2);
        assert_eq!(
            manifest.files()[1].name,
            "Engine/Source/Runtime/OodleDataCompression/Sdks/2.9.16/lib/Linux/liboo2corelinux64.a"
        );
        assert_eq!(
            manifest.files()[0].hash,
            "1111111111111111111111111111111111111111"
        );

        assert_eq!(manifest.blobs().len(), 1);
        let blob = &manifest.blobs()[0];
        assert_eq!(blob.size, 174_074);
        assert_eq!(blob.pack_offset, 34_060);
        assert_eq!(blob.pack_hash, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");

        assert_eq!(manifest.packs().len(), 1);
        let pack = &manifest.packs()[0];
        assert_eq!(pack.remote_path, "UnrealEngine-25328963");
        assert_eq!(pack.compressed_size, 361_451);
    }
}
