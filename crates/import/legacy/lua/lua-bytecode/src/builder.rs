//! `AssetBuilder` adapter for Lua bytecode sources.
//!
//! Product path: `.luac` (Lua 5.1 bytecode, optionally legacy-prefixed)
//! is decompiled to Lua source so the `LuaJIT` runtime can load it. Vanilla
//! Lua 5.1 bytecode is not loadable by `LuaJIT`; source is.

use az_asset_builder::{
    BuildProduct, BuildRule, BuilderId, CreateJobsRequest, CreateJobsResponse, JobContext,
    JobDescriptor, ProcessJobRequest, ProcessJobResponse, ProcessJobResult, TypedBuildProduct,
    normalize_source_path,
};
use uuid::uuid;

use crate::{LuaBytecodeProductFormat, LuaBytecodeSourceFormat};

pub const NAME: &str = "az.lua-script";
pub const ID: BuilderId = BuilderId::new(uuid!("e4f8c9da-b6d6-4722-3a93-08157b6c7433"));
/// Bumped when the product payload changed from raw Lua 5.1 bytecode to
/// decompiled Lua source for `LuaJIT`.
pub const VERSION: u32 = 2;

#[must_use]
pub fn desc(_: &JobContext<'_>) -> BuildRule {
    BuildRule::for_source::<LuaBytecodeSourceFormat>()
        .named(NAME)
        .id(ID)
        .version(VERSION)
        .produces::<LuaBytecodeProductFormat>()
        .create_jobs(create_jobs)
        .process(process_job)
}

fn create_jobs(req: &CreateJobsRequest<'_>) -> CreateJobsResponse {
    let jobs = req
        .platforms
        .iter()
        .copied()
        .map(JobDescriptor::default_for_platform)
        .collect();
    CreateJobsResponse {
        jobs,
        ..CreateJobsResponse::default()
    }
}

fn process_job(req: &ProcessJobRequest<'_>) -> ProcessJobResponse {
    match transform_product(&req.source_path, req.source_bytes) {
        Ok(product) => ProcessJobResponse {
            products: vec![product],
            result: ProcessJobResult::Success,
            ..ProcessJobResponse::default()
        },
        Err(err) => {
            tracing::error!(
                source = %req.source_path,
                error = %err,
                "luac decompile failed; refusing product (fail-closed)"
            );
            ProcessJobResponse {
                result: ProcessJobResult::Failed,
                ..ProcessJobResponse::default()
            }
        }
    }
}

/// Decompile a `.luac` source into a Lua-source product.
///
/// # Errors
///
/// Returns [`LuaScriptProductError`] when the chunk cannot be parsed or
/// decompiled (unknown version, malformed body, decompiler failure).
pub fn transform_product(
    source_path: &str,
    source_bytes: &[u8],
) -> Result<BuildProduct, LuaScriptProductError> {
    let stem = std::path::Path::new(source_path)
        .file_stem()
        .and_then(|stem| stem.to_str());
    let source = crate::decompile_with_options_and_module_stem(
        source_bytes,
        crate::DecompOptions::default(),
        stem,
    )
    .map_err(|source| LuaScriptProductError::Decompile {
        path: source_path.to_string(),
        source,
    })?;

    let product_path = lua_product_path(source_path);
    Ok(
        TypedBuildProduct::<LuaBytecodeProductFormat>::from_trusted_path(
            product_path,
            0,
            source.into_bytes(),
        )
        .erase(),
    )
}

fn lua_product_path(source_path: &str) -> String {
    let normalized = normalize_source_path(source_path);
    let stem = normalized.strip_suffix(".luac").unwrap_or(&normalized);
    format!("{stem}.lua")
}

/// Fail-closed product transform error.
#[derive(Debug, thiserror::Error)]
pub enum LuaScriptProductError {
    #[error("decompile Lua bytecode {path}: {source}")]
    Decompile {
        path: String,
        #[source]
        source: crate::LuaError,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LEGACY_LUAC_PREFIX;

    #[test]
    fn descriptor_claims_luac_sources() {
        let registries = az_gem_contract::Registries::new();
        let desc = desc(&JobContext::new(&registries));

        assert_eq!(desc.name, NAME);
        assert_eq!(desc.id, ID);
        assert_eq!(desc.version, VERSION);
        assert!(desc.matches("lyshineui/main menu/landing.luac"));
    }

    #[test]
    fn transform_decompiles_prefixed_chunk_to_lua_source() {
        let mut bytes = Vec::from(LEGACY_LUAC_PREFIX);
        bytes.extend_from_slice(&minimal_lua51_chunk());

        let product = transform_product("UI/Scripts/Bootstrap.luac", &bytes).unwrap();

        assert_eq!(product.product_path, "ui/scripts/bootstrap.lua");
        let text = std::str::from_utf8(&product.bytes).unwrap();
        // Product must be source text, not a Lua bytecode signature.
        assert!(!text.as_bytes().starts_with(b"\x1bLua"));
        assert!(!product.bytes.starts_with(&LEGACY_LUAC_PREFIX));
    }

    fn minimal_lua51_chunk() -> Vec<u8> {
        let mut bytes = vec![0x1b, b'L', b'u', b'a', 0x51, 0, 1, 4, 8, 4, 8, 0];
        push_string(&mut bytes, None);
        push_u32(&mut bytes, 0);
        push_u32(&mut bytes, 0);
        bytes.extend_from_slice(&[0, 0, 0, 2]);
        push_u32(&mut bytes, 1);
        push_u32(&mut bytes, 30);
        push_u32(&mut bytes, 0);
        push_u32(&mut bytes, 0);
        push_u32(&mut bytes, 0);
        push_u32(&mut bytes, 0);
        push_u32(&mut bytes, 0);
        bytes
    }

    fn push_string(bytes: &mut Vec<u8>, value: Option<&[u8]>) {
        match value {
            Some(value) => {
                push_u64(bytes, (value.len() + 1) as u64);
                bytes.extend_from_slice(value);
                bytes.push(0);
            }
            None => push_u64(bytes, 0),
        }
    }

    fn push_u32(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn push_u64(bytes: &mut Vec<u8>, value: u64) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
}
