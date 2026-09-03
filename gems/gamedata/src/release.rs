use std::fmt;

use az_asset::AssetId;
use az_core::BuildIdentity;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Identity for one immutable game-data release.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(transparent)]
pub struct GameDataReleaseId(Uuid);

impl GameDataReleaseId {
    #[inline]
    #[must_use]
    pub const fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    #[inline]
    #[must_use]
    pub const fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl fmt::Display for GameDataReleaseId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Hash of generated projection data derived from `GameData` tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(transparent)]
pub struct ProjectionHash(pub [u8; 32]);

impl ProjectionHash {
    /// Lower-case hex encoding persisted on STDB rows and heartbeats.
    #[must_use]
    pub fn to_hex_string(self) -> String {
        use std::fmt::Write as _;

        self.0
            .iter()
            .fold(String::with_capacity(64), |mut hex, byte| {
                let _ = write!(hex, "{byte:02x}");
                hex
            })
    }
}

/// Hash of reflected type metadata or a generated table schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(transparent)]
pub struct SchemaHash(pub u64);

impl fmt::Display for SchemaHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

/// Release metadata that auth, world, tools, and `GameData` service agree on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameDataRelease {
    pub id: GameDataReleaseId,
    pub game_build: BuildIdentity,
    pub table_set_hash: String,
    pub projection_hash: ProjectionHash,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameDataDependency {
    pub from: AssetId,
    pub to: AssetId,
    pub kind: GameDataDependencyKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GameDataDependencyKind {
    ForeignKey,
    GeneratedProjection,
    RuntimeOverlay,
}
