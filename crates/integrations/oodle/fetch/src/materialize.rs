//! Turning a plan into files on disk: pack transport, gzip extraction, SHA-1
//! verification, writing, and the header copy out of the Unreal checkout.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::select::{Plan, Product};

/// Where pack bytes come from.
///
/// Packs arrive gzip-compressed exactly as the CDN stores them, so a test can
/// serve one it built in memory and the real transport stays out of the
/// extraction logic.
pub trait PackSource {
    /// Fetch the compressed bytes of one pack.
    ///
    /// # Errors
    ///
    /// Returns [`PackFetchError`] when the pack cannot be retrieved.
    fn pack(&self, remote_path: &str, hash: &str) -> Result<Vec<u8>, PackFetchError>;
}

#[derive(Debug, Error)]
pub enum PackFetchError {
    #[error("pack {hash} is not available under {remote_path}")]
    Missing { remote_path: String, hash: String },
    #[error("fetching pack {hash} from {remote_path} failed: {reason}")]
    Transport {
        remote_path: String,
        hash: String,
        reason: String,
    },
}

#[derive(Debug, Error)]
pub enum MaterializeError {
    #[error(transparent)]
    Fetch(#[from] PackFetchError),
    #[error("pack {hash} is not valid gzip: {source}")]
    Decompress {
        hash: String,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "pack {hash} decompressed to {actual} bytes, too short for {name} at offset {offset}+{size}"
    )]
    ShortPack {
        hash: String,
        name: String,
        offset: u64,
        size: u64,
        actual: usize,
    },
    #[error("{name} hashes to {actual}, but the manifest says {expected}")]
    HashMismatch {
        name: String,
        expected: String,
        actual: String,
    },
    #[error("could not create {}: {source}", path.display())]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not write {}: {source}", path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Fetch, verify, and write every file in `plan` under `destination`.
///
/// Each pack is fetched once and every blob taken from it is verified against
/// the manifest's SHA-1 before anything reaches the filesystem, so a corrupt
/// download cannot leave a plausible-looking library behind.
///
/// # Errors
///
/// Returns [`MaterializeError`] on a failed fetch, a truncated or non-gzip
/// pack, a hash mismatch, or a filesystem failure.
pub fn materialize(
    plan: &Plan,
    source: &dyn PackSource,
    destination: &Path,
) -> Result<Vec<PathBuf>, MaterializeError> {
    let mut written = Vec::with_capacity(plan.files.len());
    let mut packs: HashMap<&str, Vec<u8>> = HashMap::new();

    for file in &plan.files {
        if !packs.contains_key(file.pack.hash.as_str()) {
            let compressed = source.pack(&file.pack.remote_path, &file.pack.hash)?;
            let raw = decompress(&compressed).map_err(|error| MaterializeError::Decompress {
                hash: file.pack.hash.clone(),
                source: error,
            })?;
            packs.insert(file.pack.hash.as_str(), raw);
        }
        let raw = &packs[file.pack.hash.as_str()];

        let start = usize::try_from(file.blob.pack_offset).unwrap_or(usize::MAX);
        let length = usize::try_from(file.blob.size).unwrap_or(usize::MAX);
        let Some(bytes) = start
            .checked_add(length)
            .and_then(|end| raw.get(start..end))
        else {
            return Err(MaterializeError::ShortPack {
                hash: file.pack.hash.clone(),
                name: file.source.clone(),
                offset: file.blob.pack_offset,
                size: file.blob.size,
                actual: raw.len(),
            });
        };

        let actual = sha1_hex(bytes);
        if !actual.eq_ignore_ascii_case(&file.hash) {
            return Err(MaterializeError::HashMismatch {
                name: file.source.clone(),
                expected: file.hash.clone(),
                actual,
            });
        }

        let path = destination.join(&file.destination);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| MaterializeError::CreateDirectory {
                path: parent.to_path_buf(),
                source: error,
            })?;
        }
        std::fs::write(&path, bytes).map_err(|error| MaterializeError::Write {
            path: path.clone(),
            source: error,
        })?;
        written.push(path);
    }

    Ok(written)
}

fn decompress(compressed: &[u8]) -> std::io::Result<Vec<u8>> {
    use std::io::Read as _;

    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(compressed).read_to_end(&mut raw)?;
    Ok(raw)
}

fn sha1_hex(bytes: &[u8]) -> String {
    use sha1::Digest as _;
    use std::fmt::Write as _;

    sha1::Sha1::digest(bytes)
        .iter()
        .fold(String::with_capacity(40), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

#[derive(Debug, Error)]
pub enum HeaderError {
    #[error("no Oodle {product} headers at {}", path.display())]
    MissingDirectory { product: Product, path: PathBuf },
    #[error("could not read {}: {source}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not copy {} to {}: {source}", from.display(), to.display())]
    Copy {
        from: PathBuf,
        to: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Copy each product's `*.h` out of the Unreal checkout, which is never
/// modified.
///
/// # Errors
///
/// Returns [`HeaderError`] when a product's `include` directory is absent or
/// cannot be read, or when a header cannot be copied.
pub fn copy_headers(
    unreal_root: &Path,
    version: &str,
    products: &[Product],
    destination: &Path,
) -> Result<Vec<PathBuf>, HeaderError> {
    let mut copied = Vec::new();
    for &product in products {
        let source = product
            .manifest_root()
            .split('/')
            .fold(unreal_root.to_path_buf(), |path, segment| {
                path.join(segment)
            })
            .join(version)
            .join("include");
        if !source.is_dir() {
            return Err(HeaderError::MissingDirectory {
                product,
                path: source,
            });
        }
        let target = destination.join(product.directory()).join("include");
        std::fs::create_dir_all(&target).map_err(|error| HeaderError::Read {
            path: target.clone(),
            source: error,
        })?;

        let entries = std::fs::read_dir(&source).map_err(|error| HeaderError::Read {
            path: source.clone(),
            source: error,
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| HeaderError::Read {
                path: source.clone(),
                source: error,
            })?;
            let path = entry.path();
            if path.extension().is_none_or(|extension| extension != "h") {
                continue;
            }
            let Some(name) = path.file_name() else {
                continue;
            };
            let copy = target.join(name);
            std::fs::copy(&path, &copy).map_err(|error| HeaderError::Copy {
                from: path.clone(),
                to: copy.clone(),
                source: error,
            })?;
            copied.push(copy);
        }
    }
    Ok(copied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Manifest;
    use crate::platform::Platform;
    use crate::select::select;

    /// A pack the test built: `filler || payload`, gzip compressed, served
    /// under one remote path.
    struct MemoryPacks {
        packs: HashMap<String, Vec<u8>>,
    }

    impl MemoryPacks {
        fn new(hash: &str, raw: &[u8]) -> Self {
            use std::io::Write as _;

            let mut encoder =
                flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            encoder.write_all(raw).expect("in-memory gzip write");
            let compressed = encoder.finish().expect("in-memory gzip finish");
            Self {
                packs: HashMap::from([(hash.to_owned(), compressed)]),
            }
        }
    }

    impl PackSource for MemoryPacks {
        fn pack(&self, remote_path: &str, hash: &str) -> Result<Vec<u8>, PackFetchError> {
            self.packs
                .get(hash)
                .cloned()
                .ok_or_else(|| PackFetchError::Missing {
                    remote_path: remote_path.to_owned(),
                    hash: hash.to_owned(),
                })
        }
    }

    const PAYLOAD: &[u8] = b"oodle data library bytes";
    const PACK_OFFSET: usize = 7;

    fn pack_bytes() -> Vec<u8> {
        let mut raw = vec![0xAB; PACK_OFFSET];
        raw.extend_from_slice(PAYLOAD);
        raw.extend_from_slice(b"trailing");
        raw
    }

    fn extraction_manifest(file_hash: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<DependencyManifest BaseUrl="https://cdn.unrealengine.com/dependencies">
  <Files>
    <File Name="Engine/Source/Runtime/OodleDataCompression/Sdks/2.9.16/lib/Win64/oo2core_win64.lib" Hash="{file_hash}" />
  </Files>
  <Blobs>
    <Blob Hash="{file_hash}" Size="{size}" PackHash="p0" PackOffset="{PACK_OFFSET}" />
  </Blobs>
  <Packs>
    <Pack Hash="p0" Size="39" CompressedSize="31" RemotePath="UnrealEngine-1" />
  </Packs>
</DependencyManifest>
"#,
            size = PAYLOAD.len()
        )
    }

    #[test]
    fn materialize_writes_verified_blobs_to_their_destinations() {
        let document = extraction_manifest(&sha1_hex(PAYLOAD));
        let manifest = Manifest::parse(&document).expect("fixture parses");
        let plan = select(&manifest, "2.9.16", &[Product::Data], &[Platform::Win64])
            .expect("data/win64 resolves");
        let destination = tempfile::tempdir().expect("temp destination");

        let written = materialize(
            &plan,
            &MemoryPacks::new("p0", &pack_bytes()),
            destination.path(),
        )
        .expect("the pack extracts");

        let expected = destination
            .path()
            .join("data")
            .join("lib")
            .join("oo2core_win64.lib");
        assert_eq!(written, [expected.as_path()]);
        assert_eq!(std::fs::read(&expected).expect("written file"), PAYLOAD);
    }

    #[test]
    fn materialize_rejects_a_blob_that_does_not_match_its_hash() {
        let document = extraction_manifest("0000000000000000000000000000000000000000");
        let manifest = Manifest::parse(&document).expect("fixture parses");
        let plan = select(&manifest, "2.9.16", &[Product::Data], &[Platform::Win64])
            .expect("data/win64 resolves");
        let destination = tempfile::tempdir().expect("temp destination");

        let error = materialize(
            &plan,
            &MemoryPacks::new("p0", &pack_bytes()),
            destination.path(),
        )
        .expect_err("a mismatched hash is an error");

        assert!(
            matches!(error, MaterializeError::HashMismatch { .. }),
            "unexpected error: {error}"
        );
        assert!(
            !destination
                .path()
                .join("data")
                .join("lib")
                .join("oo2core_win64.lib")
                .exists(),
            "a file that failed verification was written"
        );
    }
}
