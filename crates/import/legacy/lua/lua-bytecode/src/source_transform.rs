//! Legacy Lua bytecode source classification and authoring import.

use az_asset_builder::{
    LegacySourceInput, LegacySourceOutput, LegacySourceTransform, SourceSchemaType,
    normalize_source_path,
};

use crate::{DecompOptions, ParseError, decompile_with_options_and_module_stem};

/// Schema id for decompiled Lua authoring sources.
pub const LUA_AUTHORING_SOURCE_SCHEMA: SourceSchemaType =
    SourceSchemaType::__from_static("azoth.compat.lua.LuaAuthoringSource");

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LuaBytecodeSourceTransform;

impl LegacySourceTransform for LuaBytecodeSourceTransform {
    type Error = LuaBytecodeSourceTransformError;

    fn transform(&self, input: LegacySourceInput<'_>) -> Result<LegacySourceOutput, Self::Error> {
        let source_path = input.source_path.to_string();
        if !is_legacy_lua_bytecode_source(&source_path) {
            return Err(LuaBytecodeSourceTransformError::UnsupportedPath { path: source_path });
        }

        let stem = std::path::Path::new(&source_path)
            .file_stem()
            .and_then(|stem| stem.to_str());
        let source =
            decompile_with_options_and_module_stem(input.bytes, DecompOptions::default(), stem)
                .map_err(|source| LuaBytecodeSourceTransformError::Decompile {
                    path: source_path.clone(),
                    source,
                })?;

        let authoring_path = authoring_lua_path(&source_path);
        Ok(LegacySourceOutput::authoring_source(
            authoring_path,
            LUA_AUTHORING_SOURCE_SCHEMA,
            source.into_bytes(),
        ))
    }
}

#[must_use]
pub fn is_legacy_lua_bytecode_source(source_path: &str) -> bool {
    // `normalize_source_path` lower-cases, so one comparison covers every
    // spelling of the extension.
    normalize_source_path(source_path)
        .rsplit_once('.')
        .is_some_and(|(_, extension)| extension.eq_ignore_ascii_case("luac"))
}

fn authoring_lua_path(source_path: &str) -> String {
    let normalized = normalize_source_path(source_path);
    let stem = normalized.strip_suffix(".luac").unwrap_or(&normalized);
    format!("{stem}.lua")
}

#[derive(Debug, thiserror::Error)]
pub enum LuaBytecodeSourceTransformError {
    #[error("unsupported Lua bytecode source path {path}")]
    UnsupportedPath { path: String },
    #[error("parse Lua bytecode source: {0}")]
    Parse(#[from] ParseError),
    #[error("decompile Lua bytecode source {path}: {source}")]
    Decompile {
        path: String,
        #[source]
        source: crate::LuaError,
    },
}

#[cfg(test)]
mod tests {
    use az_asset_builder::{LegacySourceInput, LegacySourceOutput, LegacySourceTransform};

    use super::*;
    use crate::LEGACY_LUAC_PREFIX;

    #[test]
    fn routes_lua_bytecode_paths() {
        assert!(is_legacy_lua_bytecode_source("LyShineUI/Shared.luac"));
        assert!(is_legacy_lua_bytecode_source("scripts/player.LUAC"));
        assert!(!is_legacy_lua_bytecode_source("lyshineui/shared.lua"));
        assert!(!is_legacy_lua_bytecode_source("lyshineui/shared.luac.ron"));
    }

    #[test]
    fn decompiles_valid_lua_bytecode_to_authoring_source() {
        let bytes = prefixed_minimal_lua51_chunk();
        let output = LuaBytecodeSourceTransform
            .transform(LegacySourceInput::new("LyShineUI/Shared.luac", &bytes))
            .unwrap();

        let artifact = output.artifact().expect("authoring artifact");
        assert_eq!(artifact.path, "lyshineui/shared.lua");
        assert_eq!(artifact.schema, LUA_AUTHORING_SOURCE_SCHEMA);
        assert!(!artifact.bytes.starts_with(b"\x1bLua"));
        match output {
            LegacySourceOutput::AuthoringSource(_) => {}
            other => panic!("expected authoring source, got {other:?}"),
        }
    }

    fn prefixed_minimal_lua51_chunk() -> Vec<u8> {
        let mut bytes = Vec::from(LEGACY_LUAC_PREFIX);
        bytes.extend_from_slice(&minimal_lua51_chunk());
        bytes
    }

    fn minimal_lua51_chunk() -> Vec<u8> {
        let mut bytes = lua51_header();
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

    fn lua51_header() -> Vec<u8> {
        vec![0x1b, b'L', b'u', b'a', 0x51, 0, 1, 4, 8, 4, 8, 0]
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
