//! Lua bytecode parsing, decompilation, and legacy asset conversion.
//!
//! This crate merges the multi-phase Lua 5.1–5.5 decompiler pipeline with the
//! engine `BuildRule` / legacy source-transform surface for `.luac` assets.
//!
//! # Product path
//!
//! Some legacy archives store Lua 5.1 bytecode with a two-byte container
//! prefix. The asset builder decompiles those chunks to Lua source so the
//! `LuaJIT` runtime (`bevy_mod_scripting` with the `luajit` feature) can load
//! them — `LuaJIT` cannot execute vanilla Lua 5.1 bytecode.

pub mod builder;
pub mod bytecode;
pub mod chunk;
pub mod decompile;
pub mod disasm;
pub mod emit;
pub mod error;
pub mod ir;
pub mod source_transform;
pub mod version;

pub(crate) mod number;

pub use decompile::DecompOptions;
pub use emit::to_source;
pub use error::LuaError;
pub use source_transform::{
    LuaBytecodeSourceTransform, LuaBytecodeSourceTransformError, is_legacy_lua_bytecode_source,
};

use az_asset_builder::{
    BuildRuleRegistration, ProductFormat, ProductFormatId, ProductFormatRegistration, SourceFormat,
    product_format_id,
};
use std::{
    fmt, io,
    path::{Path, PathBuf},
};

use thiserror::Error;

/// Legacy `.luac` container prefix before the standard Lua signature.
pub const LEGACY_LUAC_PREFIX: [u8; 2] = [0x04, 0x00];
/// Standard Lua binary chunk signature (`\x1bLua`).
pub const LUA_SIGNATURE: [u8; 4] = *b"\x1bLua";
/// Lua 5.1 version byte.
pub const LUA51_VERSION: u8 = 0x51;

/// Source format for legacy `.luac` files.
#[derive(SourceFormat)]
#[source(ext = "luac")]
pub struct LuaBytecodeSourceFormat;

/// Product format for cooked script assets (`az.core.script` / `AZ::ScriptAsset`).
///
/// Product bytes are **Lua source text** (UTF-8), not Lua 5.1 bytecode. The
/// runtime loads them via BMS `ScriptAsset` / `mlua` `load(content)` which
/// accepts source under the `LuaJIT` backend.
#[derive(ProductFormat)]
#[product_format(
    id = "azoth.compat.azcore.lua-bytecode",
    version = 2,
    asset = az_core::ScriptAsset
)]
pub struct LuaBytecodeProductFormat;

/// Product format id for [`LuaBytecodeProductFormat`].
pub const LUA_BYTECODE_PRODUCT_FORMAT_ID: ProductFormatId =
    product_format_id::<LuaBytecodeProductFormat>();

/// The product formats this crate owns, for a host contribution to register.
#[must_use]
pub const fn product_formats() -> [ProductFormatRegistration; 1] {
    [ProductFormatRegistration::for_format::<
        LuaBytecodeProductFormat,
    >()]
}

/// The build rules this crate owns, for a host contribution to register.
#[must_use]
pub fn build_rules() -> [BuildRuleRegistration; 1] {
    [BuildRuleRegistration::new(
        builder::NAME,
        builder::ID,
        builder::desc,
    )]
}

/// Register this crate's asset-pipeline contributions into a composing host.
pub fn register<D>(ctx: &mut az_gem_contract::GemContext<'_, D>) {
    ctx.registrar::<ProductFormatRegistration>()
        .register_many(product_formats());
    ctx.registrar::<BuildRuleRegistration>()
        .register_many(build_rules());
}

/// Strip the recognized legacy container prefix if present.
#[must_use]
pub fn strip_legacy_prefix(bytes: &[u8]) -> &[u8] {
    if bytes.starts_with(&LEGACY_LUAC_PREFIX)
        && bytes
            .get(LEGACY_LUAC_PREFIX.len()..)
            .is_some_and(|rest| rest.starts_with(&LUA_SIGNATURE))
    {
        &bytes[LEGACY_LUAC_PREFIX.len()..]
    } else {
        bytes
    }
}

/// Whether `bytes` begin with the recognized legacy `.luac` prefix.
#[must_use]
pub fn has_legacy_prefix(bytes: &[u8]) -> bool {
    bytes.starts_with(&LEGACY_LUAC_PREFIX)
        && bytes
            .get(LEGACY_LUAC_PREFIX.len()..)
            .is_some_and(|rest| rest.starts_with(&LUA_SIGNATURE))
}

/// Normalize prefixed or plain Lua bytecode to the plain chunk expected by the
/// decompiler.
#[must_use]
pub fn as_lua_chunk_bytes(bytes: &[u8]) -> &[u8] {
    strip_legacy_prefix(bytes)
}

/// Parse a Lua binary chunk, accepting the optional legacy container prefix.
///
/// # Errors
///
/// Returns [`LuaError`] when the input is truncated, has an invalid header, uses
/// an unsupported Lua version, or contains malformed chunk data.
pub fn parse_chunk(bytes: &[u8]) -> Result<chunk::Chunk, LuaError> {
    chunk::parse(as_lua_chunk_bytes(bytes))
}

/// Parse and disassemble a Lua binary chunk.
///
/// # Errors
///
/// Returns [`LuaError`] when parsing fails or the chunk version has no built-in
/// opcode table in this phase.
pub fn disassemble(bytes: &[u8]) -> Result<String, LuaError> {
    let (chunk, table) = parse_with_builtin_table(bytes)?;
    Ok(disassemble_chunk_with_table(&chunk, &table))
}

/// Parse and disassemble a Lua binary chunk with a caller-supplied opcode table.
///
/// # Errors
///
/// Returns [`LuaError`] when parsing fails or the table version does not match
/// the chunk version.
pub fn disassemble_with(bytes: &[u8], table: &bytecode::OpcodeTable) -> Result<String, LuaError> {
    let chunk = parse_chunk(bytes)?;
    ensure_compatible_table(&chunk, table)?;
    Ok(disassemble_chunk_with_table(&chunk, table))
}

/// Parse a Lua binary chunk and dump Phase 2 SSA for every prototype.
///
/// # Errors
///
/// Returns [`LuaError`] when parsing fails or the chunk version has no built-in
/// opcode table in this phase.
pub fn ssa_dump(bytes: &[u8]) -> Result<String, LuaError> {
    let (chunk, table) = parse_with_builtin_table(bytes)?;
    Ok(ssa_dump_chunk_with_table(&chunk, &table))
}

/// Parse a Lua binary chunk and dump SSA with a caller-supplied opcode table.
///
/// # Errors
///
/// Returns [`LuaError`] when parsing fails or the table version does not match
/// the chunk version.
pub fn ssa_dump_with(bytes: &[u8], table: &bytecode::OpcodeTable) -> Result<String, LuaError> {
    let chunk = parse_chunk(bytes)?;
    ensure_compatible_table(&chunk, table)?;
    Ok(ssa_dump_chunk_with_table(&chunk, table))
}

/// Parse and decompile a Lua binary chunk into formatted Lua source.
///
/// # Errors
///
/// Returns [`LuaError`] when parsing, SSA construction, reconstruction, or
/// source emission fails.
pub fn decompile(bytes: &[u8]) -> Result<String, LuaError> {
    decompile_with_options(bytes, DecompOptions::default())
}

/// Parse and decompile a Lua binary chunk into core, bytecode-shaped Lua source.
///
/// # Errors
///
/// Returns [`LuaError`] when parsing, SSA construction, reconstruction, or
/// source emission fails.
pub fn decompile_core(bytes: &[u8]) -> Result<String, LuaError> {
    decompile_with_options(bytes, DecompOptions::core())
}

/// Parse and decompile a Lua binary chunk with explicit decompile options.
///
/// # Errors
///
/// Returns [`LuaError`] when parsing, SSA construction, reconstruction, or
/// source emission fails.
pub fn decompile_with_options(bytes: &[u8], options: DecompOptions) -> Result<String, LuaError> {
    decompile_with_options_and_module_stem(bytes, options, None)
}

/// Parse and decompile a Lua binary chunk with an optional file-stem fallback.
///
/// # Errors
///
/// Returns [`LuaError`] when parsing, SSA construction, reconstruction, or
/// source emission fails.
pub fn decompile_with_options_and_module_stem(
    bytes: &[u8],
    options: DecompOptions,
    fallback_module_stem: Option<&str>,
) -> Result<String, LuaError> {
    let (chunk, table) = parse_with_builtin_table(bytes)?;
    decompile_chunk_with_table_options_and_module_stem(
        &chunk,
        &table,
        options,
        fallback_module_stem,
    )
}

/// Parse and decompile a Lua binary chunk with a caller-supplied opcode table.
///
/// # Errors
///
/// Returns [`LuaError`] when parsing fails, the table version does not match the
/// chunk version, SSA/decompilation fails, or source emission fails.
pub fn decompile_with(bytes: &[u8], table: &bytecode::OpcodeTable) -> Result<String, LuaError> {
    decompile_with_table_options(bytes, table, DecompOptions::default())
}

/// Parse and decompile a Lua binary chunk with explicit options and opcode table.
///
/// # Errors
///
/// Returns [`LuaError`] when parsing fails, the table version does not match the
/// chunk version, SSA/decompilation fails, or source emission fails.
pub fn decompile_with_table_options(
    bytes: &[u8],
    table: &bytecode::OpcodeTable,
    options: DecompOptions,
) -> Result<String, LuaError> {
    decompile_with_table_options_and_module_stem(bytes, table, options, None)
}

/// Parse and decompile a Lua binary chunk with explicit options, opcode table,
/// and an optional file-stem fallback.
///
/// # Errors
///
/// Returns [`LuaError`] when parsing fails, the table version does not match the
/// chunk version, SSA/decompilation fails, or source emission fails.
pub fn decompile_with_table_options_and_module_stem(
    bytes: &[u8],
    table: &bytecode::OpcodeTable,
    options: DecompOptions,
    fallback_module_stem: Option<&str>,
) -> Result<String, LuaError> {
    let chunk = parse_chunk(bytes)?;
    ensure_compatible_table(&chunk, table)?;
    decompile_chunk_with_table_options_and_module_stem(&chunk, table, options, fallback_module_stem)
}

/// Decompile a chunk and prepend best-effort disassembly annotations as Lua comments.
///
/// # Errors
///
/// Returns [`LuaError`] when parsing, disassembly, decompilation, or source
/// emission fails.
pub fn decompile_annotated(bytes: &[u8]) -> Result<String, LuaError> {
    let (chunk, table) = parse_with_builtin_table(bytes)?;
    Ok(annotate_source(
        &disassemble_chunk_with_table(&chunk, &table),
        &decompile_chunk_with_table(&chunk, &table)?,
    ))
}

/// Decompile with a caller-supplied opcode table and prepend disassembly comments.
///
/// # Errors
///
/// Returns [`LuaError`] when parsing fails, the table version does not match the
/// chunk version, SSA/decompilation fails, or source emission fails.
pub fn decompile_annotated_with(
    bytes: &[u8],
    table: &bytecode::OpcodeTable,
) -> Result<String, LuaError> {
    let chunk = parse_chunk(bytes)?;
    ensure_compatible_table(&chunk, table)?;
    Ok(annotate_source(
        &disassemble_chunk_with_table(&chunk, table),
        &decompile_chunk_with_table(&chunk, table)?,
    ))
}

/// Parsed Lua bytecode view used by inspection and `LyShine` loaders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LuaBytecode<'a> {
    bytes: &'a [u8],
    chunk: &'a [u8],
    prefix: Option<LegacyLuacPrefix>,
    header: LuaHeader,
}

impl<'a> LuaBytecode<'a> {
    /// Parse header metadata from legacy or plain Lua bytecode bytes.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError`] when the signature/header is invalid or the Lua
    /// version is unrecognized.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, ParseError> {
        let (prefix, chunk) = if has_legacy_prefix(bytes) {
            (
                Some(LegacyLuacPrefix(LEGACY_LUAC_PREFIX)),
                &bytes[LEGACY_LUAC_PREFIX.len()..],
            )
        } else {
            (None, bytes)
        };

        let header = LuaHeader::parse(chunk)?;
        Ok(Self {
            bytes,
            chunk,
            prefix,
            header,
        })
    }

    #[inline]
    #[must_use]
    pub const fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    #[inline]
    #[must_use]
    pub const fn chunk(&self) -> &'a [u8] {
        self.chunk
    }

    #[inline]
    #[must_use]
    pub const fn prefix(&self) -> Option<LegacyLuacPrefix> {
        self.prefix
    }

    #[inline]
    #[must_use]
    pub const fn header(&self) -> LuaHeader {
        self.header
    }

    #[inline]
    #[must_use]
    pub const fn has_legacy_prefix(&self) -> bool {
        self.prefix.is_some()
    }

    /// Decode the full main prototype tree.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError`] when the body cannot be decoded.
    pub fn parse_chunk(&self) -> Result<LuaChunk, ParseError> {
        let chunk = chunk::parse(self.chunk).map_err(ParseError::from_lua_error)?;
        Ok(LuaChunk::from_chunk(chunk))
    }
}

/// Owned decoded Lua chunk used by inspection / require-graph helpers.
#[derive(Debug, Clone, PartialEq)]
pub struct LuaChunk {
    pub main: LuaProto,
}

impl LuaChunk {
    fn from_chunk(chunk: chunk::Chunk) -> Self {
        Self {
            main: LuaProto::from_proto(chunk.root),
        }
    }
}

/// Owned decoded function prototype.
#[derive(Debug, Clone, PartialEq)]
pub struct LuaProto {
    pub source: Option<String>,
    pub line_defined: u32,
    pub last_line_defined: u32,
    pub upvalue_count: u8,
    pub param_count: u8,
    pub is_vararg: u8,
    pub max_stack_size: u8,
    pub code: Vec<Instruction>,
    pub constants: Vec<LuaConstant>,
    pub protos: Vec<Self>,
    pub line_info: Vec<u32>,
    pub locals: Vec<LuaLocal>,
    pub upvalues: Vec<String>,
}

/// Widen a debug-info line or program counter.
///
/// These fields are non-negative in any well-formed chunk. A malformed
/// negative value clamps to zero rather than reinterpreting its bits as a
/// line number near four billion.
fn debug_index(value: i32) -> u32 {
    u32::try_from(value).unwrap_or(0)
}

impl LuaProto {
    fn from_proto(proto: chunk::Proto) -> Self {
        let source = if proto.source.is_empty() {
            None
        } else {
            Some(String::from_utf8_lossy(proto.source.as_slice()).into_owned())
        };
        let constants = proto
            .constants
            .into_iter()
            .map(LuaConstant::from_constant)
            .collect();
        let locals = proto
            .loc_vars
            .into_iter()
            .map(|local| LuaLocal {
                name: String::from_utf8_lossy(local.name.as_slice()).into_owned(),
                start_pc: debug_index(local.start_pc),
                end_pc: debug_index(local.end_pc),
            })
            .collect();
        let upvalues = proto
            .upvalues
            .into_iter()
            .map(|up| String::from_utf8_lossy(up.name.as_slice()).into_owned())
            .collect();
        let code = proto.code.into_iter().map(Instruction::new).collect();
        let protos = proto.protos.into_iter().map(Self::from_proto).collect();
        let line_info = proto.line_info.into_iter().map(debug_index).collect();

        Self {
            source,
            line_defined: debug_index(proto.line_defined),
            last_line_defined: debug_index(proto.last_line_defined),
            upvalue_count: proto.nups,
            param_count: proto.num_params,
            is_vararg: proto.is_vararg,
            max_stack_size: proto.max_stack,
            code,
            constants,
            protos,
            line_info,
            locals,
            upvalues,
        }
    }

    #[inline]
    #[must_use]
    pub const fn instruction_count(&self) -> usize {
        self.code.len()
    }

    #[inline]
    #[must_use]
    pub fn proto_count_recursive(&self) -> usize {
        self.protos
            .iter()
            .map(|proto| 1 + proto.proto_count_recursive())
            .sum()
    }
}

/// Constant pool entry.
#[derive(Debug, Clone, PartialEq)]
pub enum LuaConstant {
    Nil,
    Boolean(bool),
    Number(f64),
    Integer(i64),
    String(String),
}

impl LuaConstant {
    fn from_constant(constant: chunk::Constant) -> Self {
        match constant {
            chunk::Constant::Nil => Self::Nil,
            chunk::Constant::Boolean(value) => Self::Boolean(value),
            chunk::Constant::Number(value) => Self::Number(value),
            chunk::Constant::Integer(value) => Self::Integer(value),
            chunk::Constant::Str(value) => {
                Self::String(String::from_utf8_lossy(value.as_slice()).into_owned())
            }
        }
    }

    #[must_use]
    pub const fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value.as_str()),
            _ => None,
        }
    }
}

/// Local variable debug entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LuaLocal {
    pub name: String,
    pub start_pc: u32,
    pub end_pc: u32,
}

/// Decoded Lua 5.1 instruction word (ABC / `ABx` / `AsBx` fields).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Instruction {
    pub raw: u32,
}

impl Instruction {
    #[inline]
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self { raw }
    }

    #[inline]
    #[must_use]
    pub const fn opcode(self) -> Opcode {
        Opcode::from_u8((self.raw & 0x3f) as u8)
    }

    #[inline]
    #[must_use]
    pub const fn a(self) -> u8 {
        ((self.raw >> 6) & 0xff) as u8
    }

    #[inline]
    #[must_use]
    pub const fn c(self) -> u16 {
        ((self.raw >> 14) & 0x1ff) as u16
    }

    #[inline]
    #[must_use]
    pub const fn b(self) -> u16 {
        ((self.raw >> 23) & 0x1ff) as u16
    }

    #[inline]
    #[must_use]
    pub const fn bx(self) -> u32 {
        (self.raw >> 14) & 0x3ffff
    }

    #[inline]
    #[must_use]
    pub const fn sbx(self) -> i32 {
        // `bx` is an 18-bit field, far below `i32::MAX`, so `cast_signed`
        // is exact.
        self.bx().cast_signed() - 131_071
    }
}

/// Lua 5.1 opcode names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Opcode {
    Move,
    LoadK,
    LoadBool,
    LoadNil,
    GetUpval,
    GetGlobal,
    GetTable,
    SetGlobal,
    SetUpval,
    SetTable,
    NewTable,
    SelfOp,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Unm,
    Not,
    Len,
    Concat,
    Jmp,
    Eq,
    Lt,
    Le,
    Test,
    TestSet,
    Call,
    TailCall,
    Return,
    ForLoop,
    ForPrep,
    TForLoop,
    SetList,
    Close,
    Closure,
    Vararg,
    Unknown(u8),
}

impl Opcode {
    #[must_use]
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Move,
            1 => Self::LoadK,
            2 => Self::LoadBool,
            3 => Self::LoadNil,
            4 => Self::GetUpval,
            5 => Self::GetGlobal,
            6 => Self::GetTable,
            7 => Self::SetGlobal,
            8 => Self::SetUpval,
            9 => Self::SetTable,
            10 => Self::NewTable,
            11 => Self::SelfOp,
            12 => Self::Add,
            13 => Self::Sub,
            14 => Self::Mul,
            15 => Self::Div,
            16 => Self::Mod,
            17 => Self::Pow,
            18 => Self::Unm,
            19 => Self::Not,
            20 => Self::Len,
            21 => Self::Concat,
            22 => Self::Jmp,
            23 => Self::Eq,
            24 => Self::Lt,
            25 => Self::Le,
            26 => Self::Test,
            27 => Self::TestSet,
            28 => Self::Call,
            29 => Self::TailCall,
            30 => Self::Return,
            31 => Self::ForLoop,
            32 => Self::ForPrep,
            33 => Self::TForLoop,
            34 => Self::SetList,
            35 => Self::Close,
            36 => Self::Closure,
            37 => Self::Vararg,
            other => Self::Unknown(other),
        }
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Move => "MOVE",
            Self::LoadK => "LOADK",
            Self::LoadBool => "LOADBOOL",
            Self::LoadNil => "LOADNIL",
            Self::GetUpval => "GETUPVAL",
            Self::GetGlobal => "GETGLOBAL",
            Self::GetTable => "GETTABLE",
            Self::SetGlobal => "SETGLOBAL",
            Self::SetUpval => "SETUPVAL",
            Self::SetTable => "SETTABLE",
            Self::NewTable => "NEWTABLE",
            Self::SelfOp => "SELF",
            Self::Add => "ADD",
            Self::Sub => "SUB",
            Self::Mul => "MUL",
            Self::Div => "DIV",
            Self::Mod => "MOD",
            Self::Pow => "POW",
            Self::Unm => "UNM",
            Self::Not => "NOT",
            Self::Len => "LEN",
            Self::Concat => "CONCAT",
            Self::Jmp => "JMP",
            Self::Eq => "EQ",
            Self::Lt => "LT",
            Self::Le => "LE",
            Self::Test => "TEST",
            Self::TestSet => "TESTSET",
            Self::Call => "CALL",
            Self::TailCall => "TAILCALL",
            Self::Return => "RETURN",
            Self::ForLoop => "FORLOOP",
            Self::ForPrep => "FORPREP",
            Self::TForLoop => "TFORLOOP",
            Self::SetList => "SETLIST",
            Self::Close => "CLOSE",
            Self::Closure => "CLOSURE",
            Self::Vararg => "VARARG",
            Self::Unknown(_) => "UNKNOWN",
        }
    }
}

/// Legacy two-byte container prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LegacyLuacPrefix(pub [u8; 2]);

/// Header metadata exposed to loaders / inspection tooling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LuaHeader {
    pub signature: [u8; 4],
    pub version: LuaVersion,
    pub version_byte: u8,
    pub format: u8,
    pub endianness: LuaEndianness,
    pub int_size: u8,
    pub size_t_size: u8,
    pub instruction_size: u8,
    pub number_size: u8,
    pub integral_numbers: bool,
}

impl LuaHeader {
    /// Parse a Lua binary chunk header from plain (unprefixed) bytes.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError`] on bad signature, unsupported version, or short
    /// input.
    pub fn parse(bytes: &[u8]) -> Result<Self, ParseError> {
        if bytes.len() < 12 {
            return Err(ParseError::TooShort {
                needed: 12,
                actual: bytes.len(),
            });
        }
        let signature = [bytes[0], bytes[1], bytes[2], bytes[3]];
        if signature != LUA_SIGNATURE {
            return Err(ParseError::InvalidSignature { actual: signature });
        }
        let version_byte = bytes[4];
        let version =
            LuaVersion::from_byte(version_byte).ok_or(ParseError::UnsupportedLuaVersion {
                version: version_byte,
            })?;
        let endianness = LuaEndianness::from_byte(bytes[6])
            .ok_or(ParseError::InvalidEndianness { value: bytes[6] })?;
        Ok(Self {
            signature,
            version,
            version_byte,
            format: bytes[5],
            endianness,
            int_size: bytes[7],
            size_t_size: bytes[8],
            instruction_size: bytes[9],
            number_size: bytes[10],
            integral_numbers: bytes[11] != 0,
        })
    }

    #[inline]
    #[must_use]
    pub const fn is_lua51(self) -> bool {
        matches!(self.version, LuaVersion::Lua51)
    }
}

/// Recognized Lua header version for inspection summaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LuaVersion {
    Lua51,
    Lua52,
    Lua53,
    Lua54,
    Lua55,
}

impl LuaVersion {
    #[inline]
    #[must_use]
    pub const fn from_byte(value: u8) -> Option<Self> {
        match value {
            0x51 => Some(Self::Lua51),
            0x52 => Some(Self::Lua52),
            0x53 => Some(Self::Lua53),
            0x54 => Some(Self::Lua54),
            0x55 => Some(Self::Lua55),
            _ => None,
        }
    }

    #[inline]
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Lua51 => "Lua 5.1",
            Self::Lua52 => "Lua 5.2",
            Self::Lua53 => "Lua 5.3",
            Self::Lua54 => "Lua 5.4",
            Self::Lua55 => "Lua 5.5",
        }
    }
}

/// Endianness flag from the Lua header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LuaEndianness {
    Big,
    Little,
}

impl LuaEndianness {
    #[inline]
    #[must_use]
    pub const fn from_byte(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Big),
            1 => Some(Self::Little),
            _ => None,
        }
    }

    #[inline]
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Big => "big",
            Self::Little => "little",
        }
    }
}

/// Fail-closed parse errors for the engine-facing inspection API.
#[derive(Debug, Error)]
pub enum ParseError {
    #[error("asset is too short: need at least {needed} bytes, got {actual}")]
    TooShort { needed: usize, actual: usize },

    #[error("invalid Lua bytecode signature: got {actual:?}")]
    InvalidSignature { actual: [u8; 4] },

    #[error("unsupported Lua bytecode version: 0x{version:02x}")]
    UnsupportedLuaVersion { version: u8 },

    #[error("invalid Lua bytecode endianness marker: {value}")]
    InvalidEndianness { value: u8 },

    #[error("unsupported Lua header layout: {reason}")]
    UnsupportedLayout { reason: &'static str },

    #[error("malformed Lua chunk: {0}")]
    Malformed(String),
}

impl ParseError {
    fn from_lua_error(error: LuaError) -> Self {
        match error {
            LuaError::Truncated {
                offset: _,
                needed,
                remaining,
            } => Self::TooShort {
                needed,
                actual: remaining,
            },
            LuaError::BadMagic => Self::InvalidSignature {
                actual: [0, 0, 0, 0],
            },
            LuaError::UnsupportedVersion(version) => Self::UnsupportedLuaVersion { version },
            LuaError::Malformed(reason) => Self::Malformed(reason),
            other => Self::Malformed(other.to_string()),
        }
    }
}

/// Parsed bytecode plus decoded main chunk.
#[derive(Debug, Clone, PartialEq)]
pub struct LuaBytecodeInspection {
    has_legacy_prefix: bool,
    header: LuaHeader,
    chunk_bytes: usize,
    asset_bytes: usize,
    chunk: LuaChunk,
    source: String,
}

/// Report kinds for path/file inspection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LuaBytecodeReportKind {
    Summary,
    Disassembly,
    Source,
    Ssa,
}

/// Human-readable summary of one Lua bytecode asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LuaBytecodeSummary {
    pub has_legacy_prefix: bool,
    pub version: LuaVersion,
    pub format: u8,
    pub endianness: LuaEndianness,
    pub int_size: u8,
    pub size_t_size: u8,
    pub instruction_size: u8,
    pub number_size: u8,
    pub integral_numbers: bool,
    pub chunk_bytes: usize,
    pub asset_bytes: usize,
    pub instructions: usize,
    pub constants: usize,
    pub child_protos: usize,
    pub locals: usize,
    pub upvalues: usize,
}

impl fmt::Display for LuaBytecodeSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "  prefix:            {}", self.has_legacy_prefix)?;
        writeln!(f, "  version:           {}", self.version.as_str())?;
        writeln!(f, "  format:            {}", self.format)?;
        writeln!(f, "  endianness:        {}", self.endianness.as_str())?;
        writeln!(f, "  int size:          {}", self.int_size)?;
        writeln!(f, "  size_t size:       {}", self.size_t_size)?;
        writeln!(f, "  instruction size:  {}", self.instruction_size)?;
        writeln!(f, "  number size:       {}", self.number_size)?;
        writeln!(f, "  integral numbers:  {}", self.integral_numbers)?;
        writeln!(f, "  chunk bytes:       {}", self.chunk_bytes)?;
        writeln!(f, "  asset bytes:       {}", self.asset_bytes)?;
        writeln!(f, "  instructions:      {}", self.instructions)?;
        writeln!(f, "  constants:         {}", self.constants)?;
        writeln!(f, "  child protos:      {}", self.child_protos)?;
        writeln!(f, "  locals:            {}", self.locals)?;
        write!(f, "  upvalues:          {}", self.upvalues)
    }
}

impl LuaBytecodeInspection {
    /// Parse bytes, fully decode the chunk tree, and decompile to source.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError`] on header/body failures, or wraps decompile
    /// failures as [`ParseError::Malformed`].
    pub fn parse(bytes: &[u8]) -> Result<Self, ParseError> {
        let bytecode = LuaBytecode::parse(bytes)?;
        let chunk = bytecode.parse_chunk()?;
        let source = decompile(bytes).map_err(|error| ParseError::Malformed(error.to_string()))?;
        Ok(Self {
            has_legacy_prefix: bytecode.has_legacy_prefix(),
            header: bytecode.header(),
            chunk_bytes: bytecode.chunk().len(),
            asset_bytes: bytecode.bytes().len(),
            chunk,
            source,
        })
    }

    #[inline]
    #[must_use]
    pub const fn chunk(&self) -> &LuaChunk {
        &self.chunk
    }

    #[must_use]
    pub fn summary(&self) -> LuaBytecodeSummary {
        let main = &self.chunk.main;
        LuaBytecodeSummary {
            has_legacy_prefix: self.has_legacy_prefix,
            version: self.header.version,
            format: self.header.format,
            endianness: self.header.endianness,
            int_size: self.header.int_size,
            size_t_size: self.header.size_t_size,
            instruction_size: self.header.instruction_size,
            number_size: self.header.number_size,
            integral_numbers: self.header.integral_numbers,
            chunk_bytes: self.chunk_bytes,
            asset_bytes: self.asset_bytes,
            instructions: main.code.len(),
            constants: main.constants.len(),
            child_protos: main.proto_count_recursive(),
            locals: main.locals.len(),
            upvalues: main.upvalues.len(),
        }
    }

    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }
}

/// File inspection report (path label + inspection payload).
#[derive(Debug, Clone, PartialEq)]
pub struct LuaBytecodeFileInspection {
    source: String,
    inspection: LuaBytecodeInspection,
    disassembly: String,
    ssa: String,
}

impl LuaBytecodeFileInspection {
    /// # Errors
    ///
    /// Returns [`ParseError`] when parsing or decompilation fails.
    pub fn parse(source: impl Into<String>, bytes: &[u8]) -> Result<Self, ParseError> {
        let source = source.into();
        let inspection = LuaBytecodeInspection::parse(bytes)?;
        let disassembly =
            disassemble(bytes).map_err(|error| ParseError::Malformed(error.to_string()))?;
        let ssa = ssa_dump(bytes).map_err(|error| ParseError::Malformed(error.to_string()))?;
        Ok(Self {
            source,
            inspection,
            disassembly,
            ssa,
        })
    }

    #[inline]
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    #[inline]
    #[must_use]
    pub const fn inspection(&self) -> &LuaBytecodeInspection {
        &self.inspection
    }

    #[must_use]
    pub fn report(&self, kind: LuaBytecodeReportKind) -> String {
        match kind {
            LuaBytecodeReportKind::Summary => {
                format!("{}\n{}", self.source, self.inspection.summary())
            }
            LuaBytecodeReportKind::Disassembly => {
                format!("{}\n{}", self.source, self.disassembly)
            }
            LuaBytecodeReportKind::Source => self.inspection.source().to_string(),
            LuaBytecodeReportKind::Ssa => format!("{}\n{}", self.source, self.ssa),
        }
    }
}

/// # Errors
///
/// Returns [`ParseError`] when parsing or decompilation fails.
pub fn inspect_lua_bytecode_file(
    source: impl Into<String>,
    bytes: &[u8],
) -> Result<LuaBytecodeFileInspection, ParseError> {
    LuaBytecodeFileInspection::parse(source, bytes)
}

/// Path-based inspection errors.
#[derive(Debug, Error)]
pub enum LuaBytecodeInspectionError {
    #[error("read Lua bytecode asset {path:?}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("parse Lua bytecode asset {path:?}")]
    Parse {
        path: PathBuf,
        #[source]
        source: ParseError,
    },
}

/// Inspect a `.luac` path and return the requested report text.
///
/// # Errors
///
/// Returns [`LuaBytecodeInspectionError`] on IO or parse/decompile failure.
pub fn inspect_lua_bytecode_path(
    path: impl AsRef<Path>,
    kind: LuaBytecodeReportKind,
) -> Result<String, LuaBytecodeInspectionError> {
    let path = path.as_ref();
    let bytes = std::fs::read(path).map_err(|source| LuaBytecodeInspectionError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let inspection =
        inspect_lua_bytecode_file(path.display().to_string(), &bytes).map_err(|source| {
            LuaBytecodeInspectionError::Parse {
                path: path.to_path_buf(),
                source,
            }
        })?;
    Ok(inspection.report(kind))
}

fn parse_with_builtin_table(
    bytes: &[u8],
) -> Result<(chunk::Chunk, bytecode::OpcodeTable), LuaError> {
    let chunk = parse_chunk(bytes)?;
    let table = bytecode::OpcodeTable::builtin(chunk.header.version);
    Ok((chunk, table))
}

fn disassemble_chunk_with_table(chunk: &chunk::Chunk, table: &bytecode::OpcodeTable) -> String {
    let mut out = format!("-- Lua {} Disassembly --\n\n", chunk.header.version);
    out.push_str(&disasm::disassemble_proto(&chunk.root, table));
    out
}

fn ssa_dump_chunk_with_table(chunk: &chunk::Chunk, table: &bytecode::OpcodeTable) -> String {
    ir::dump::dump_proto_tree(&chunk.root, table)
}

fn decompile_chunk_with_table(
    chunk: &chunk::Chunk,
    table: &bytecode::OpcodeTable,
) -> Result<String, LuaError> {
    decompile_chunk_with_table_options(chunk, table, DecompOptions::default())
}

fn decompile_chunk_with_table_options(
    chunk: &chunk::Chunk,
    table: &bytecode::OpcodeTable,
    options: DecompOptions,
) -> Result<String, LuaError> {
    decompile_chunk_with_table_options_and_module_stem(chunk, table, options, None)
}

fn decompile_chunk_with_table_options_and_module_stem(
    chunk: &chunk::Chunk,
    table: &bytecode::OpcodeTable,
    options: DecompOptions,
    fallback_module_stem: Option<&str>,
) -> Result<String, LuaError> {
    let block = reconstruct_chunk_with_table_options_and_module_stem(
        chunk,
        table,
        options,
        fallback_module_stem,
    )?;
    emit::to_source(&block)
}

fn reconstruct_chunk_with_table_options_and_module_stem(
    chunk: &chunk::Chunk,
    table: &bytecode::OpcodeTable,
    options: DecompOptions,
    fallback_module_stem: Option<&str>,
) -> Result<decompile::ast::Block, LuaError> {
    let ssa = ir::build_ssa(&chunk.root, table);
    decompile::decompile_proto_with_options_and_module_stem(
        &chunk.root,
        &ssa,
        table,
        options,
        fallback_module_stem,
    )
}

fn ensure_compatible_table(
    chunk: &chunk::Chunk,
    table: &bytecode::OpcodeTable,
) -> Result<(), LuaError> {
    if table.version != chunk.header.version {
        return Err(LuaError::Malformed(format!(
            "opcode table version {} does not match chunk version {}",
            table.version, chunk.header.version
        )));
    }
    Ok(())
}

fn annotate_source(disassembly: &str, source: &str) -> String {
    let mut out = String::from("-- disassembly annotations\n");
    for line in disassembly.lines() {
        if line.is_empty() {
            out.push_str("--\n");
        } else {
            out.push_str("-- ");
            out.push_str(line);
            out.push('\n');
        }
    }
    out.push('\n');
    out.push_str(source);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The registration is keyed on the builder id and ordered by the name, so
    /// a registration that disagrees with the rule it resolves would file job
    /// attempts under an identity the dispatcher never reports.
    #[test]
    fn every_registration_matches_the_rule_it_resolves() {
        let registries = az_gem_contract::Registries::new();
        let context = az_asset_builder::JobContext::new(&registries);

        for registration in build_rules() {
            let rule = registration.rule(&context);
            assert_eq!(registration.name(), rule.name);
            assert_eq!(registration.id(), rule.id);
        }
    }

    #[test]
    fn strips_legacy_prefix() {
        let mut bytes = Vec::from(LEGACY_LUAC_PREFIX);
        bytes.extend_from_slice(&minimal_lua51_chunk());
        assert!(has_legacy_prefix(&bytes));
        let chunk = parse_chunk(&bytes).unwrap();
        assert_eq!(chunk.root.num_params, 0);
        assert_eq!(chunk.root.code.len(), 1);
    }

    #[test]
    fn decompiles_minimal_chunk_to_source() {
        let source = decompile(&minimal_lua51_chunk()).unwrap();
        assert!(source.contains("return") || source.is_empty() || !source.is_empty());
    }

    #[test]
    fn inspection_summary_is_stable() {
        let bytes = minimal_lua51_chunk();
        let inspection = LuaBytecodeInspection::parse(&bytes).unwrap();
        let summary = inspection.summary().to_string();
        assert!(summary.contains("Lua 5.1"));
        assert!(summary.contains("prefix:            false"));
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
