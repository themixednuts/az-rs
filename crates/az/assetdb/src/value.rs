//! Domain values stored directly by the `AssetDB` schema.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;

use drizzle::error::DrizzleError;
use drizzle::sqlite::prelude::*;
use drizzle::sqlite::traits::DrizzleSQLiteColumn;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

/// A BLAKE3 digest stored as exactly 32 `SQLite` BLOB bytes.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Digest([u8; blake3::OUT_LEN]);

impl Digest {
    pub const BYTE_LENGTH: usize = blake3::OUT_LEN;

    #[must_use]
    pub const fn from_bytes(bytes: [u8; blake3::OUT_LEN]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; blake3::OUT_LEN] {
        &self.0
    }

    #[must_use]
    pub fn to_hex(self) -> String {
        blake3::Hash::from_bytes(self.0).to_hex().to_string()
    }

    fn parse_hex(value: &str) -> Result<Self, InvalidDigest> {
        if value.len() != blake3::OUT_LEN * 2 {
            return Err(InvalidDigest::Length { got: value.len() });
        }

        let mut bytes = [0_u8; blake3::OUT_LEN];
        for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
            let high = hex_nibble(chunk[0]).ok_or(InvalidDigest::Character { index: index * 2 })?;
            let low = hex_nibble(chunk[1]).ok_or(InvalidDigest::Character {
                index: index * 2 + 1,
            })?;
            bytes[index] = (high << 4) | low;
        }
        Ok(Self(bytes))
    }
}

impl FromStr for Digest {
    type Err = InvalidDigest;

    /// Decode the canonical lowercase or uppercase hexadecimal projection
    /// used at `AssetDB` protocol and migration boundaries.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse_hex(value)
    }
}

impl Serialize for Digest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Digest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

impl From<blake3::Hash> for Digest {
    fn from(value: blake3::Hash) -> Self {
        Self(*value.as_bytes())
    }
}

impl From<Digest> for [u8; blake3::OUT_LEN] {
    fn from(value: Digest) -> Self {
        value.0
    }
}

impl fmt::Debug for Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("Digest")
            .field(&self.to_hex())
            .finish()
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_hex())
    }
}

impl DrizzleSQLiteColumn for Digest {
    type SQLType = drizzle::sqlite::types::Blob;

    fn decode(value: SQLiteValueRef<'_>) -> Result<Self, DrizzleError> {
        let SQLiteValueRef::Blob(value) = value else {
            return Err(DrizzleError::ConversionError(
                "AssetDB digest must be stored as BLOB".into(),
            ));
        };
        let bytes = value.try_into().map_err(|_| {
            DrizzleError::ConversionError(
                format!(
                    "AssetDB digest must contain exactly {} bytes, got {}",
                    blake3::OUT_LEN,
                    value.len()
                )
                .into(),
            )
        })?;
        Ok(Self(bytes))
    }

    fn encode(&self) -> SQLiteValue<'_> {
        SQLiteValue::Blob(Cow::Borrowed(&self.0))
    }

    fn encode_owned(self) -> OwnedSQLiteValue {
        OwnedSQLiteValue::Blob(Box::new(self.0))
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum InvalidDigest {
    #[error("digest must contain 64 hexadecimal characters, got {got}")]
    Length { got: usize },
    #[error("digest contains a non-hexadecimal character at byte {index}")]
    Character { index: usize },
}

const fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

/// Normalized source paths shadowed by explicit project overrides.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Exclusions(BTreeSet<String>);

impl Exclusions {
    #[must_use]
    pub const fn new(paths: BTreeSet<String>) -> Self {
        Self(paths)
    }

    #[must_use]
    pub const fn as_set(&self) -> &BTreeSet<String> {
        &self.0
    }

    #[must_use]
    pub fn into_set(self) -> BTreeSet<String> {
        self.0
    }
}

impl From<BTreeSet<String>> for Exclusions {
    fn from(value: BTreeSet<String>) -> Self {
        Self(value)
    }
}

/// Additional catalog lookup paths for one built product.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Aliases(Vec<String>);

impl Aliases {
    #[must_use]
    pub const fn new(paths: Vec<String>) -> Self {
        Self(paths)
    }

    #[must_use]
    pub fn as_slice(&self) -> &[String] {
        &self.0
    }

    #[must_use]
    pub fn into_vec(self) -> Vec<String> {
        self.0
    }
}

impl From<Vec<String>> for Aliases {
    fn from(value: Vec<String>) -> Self {
        Self(value)
    }
}

/// The two durable scheduler lanes. Planner work has no builder identity;
/// build work always does.
#[derive(SQLiteEnum, Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(i64)]
pub enum Work {
    #[default]
    Plan = 0,
    Build = 1,
}

impl Work {
    #[must_use]
    pub const fn as_i64(self) -> i64 {
        self as i64
    }
}

/// Encoding of an editable payload stored in the merged payload table.
#[derive(
    SQLiteEnum,
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
)]
#[repr(i64)]
pub enum Encoding {
    #[default]
    Ron = 0,
    Bytes = 1,
}

impl Encoding {
    #[must_use]
    pub const fn as_i64(self) -> i64 {
        self as i64
    }
}

/// Authored dependency intent. Resolution to an `AssetDB` row is stored beside
/// this value and never replaces it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Target {
    Guid(Uuid),
    Path(TargetPath),
}

/// Canonical project-relative path used by an authored dependency target.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TargetPath(String);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("AssetDB dependency path is not canonical: {path}")]
pub struct InvalidTargetPath {
    path: String,
}

impl TargetPath {
    /// # Errors
    ///
    /// Returns [`InvalidTargetPath`] if `path` is empty, starts or ends with
    /// `/`, contains a backslash, or has an empty, `.` or `..` component.
    pub fn new(path: impl Into<String>) -> Result<Self, InvalidTargetPath> {
        let path = path.into();
        if path.is_empty()
            || path.starts_with('/')
            || path.ends_with('/')
            || path.contains('\\')
            || path
                .split('/')
                .any(|component| component.is_empty() || component == "." || component == "..")
        {
            return Err(InvalidTargetPath { path });
        }
        Ok(Self(path))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Target {
    const GUID_TAG: u8 = 0;
    const PATH_TAG: u8 = 1;

    /// # Errors
    ///
    /// Returns any error [`TargetPath::new`] returns for `path`.
    pub fn path(path: impl Into<String>) -> Result<Self, InvalidTargetPath> {
        TargetPath::new(path).map(Self::Path)
    }

    fn to_bytes(&self) -> Vec<u8> {
        match self {
            Self::Guid(guid) => {
                let mut encoded = Vec::with_capacity(17);
                encoded.push(Self::GUID_TAG);
                encoded.extend_from_slice(guid.as_bytes());
                encoded
            }
            Self::Path(path) => {
                let mut encoded = Vec::with_capacity(path.as_str().len() + 1);
                encoded.push(Self::PATH_TAG);
                encoded.extend_from_slice(path.as_str().as_bytes());
                encoded
            }
        }
    }
}

impl DrizzleSQLiteColumn for Target {
    type SQLType = drizzle::sqlite::types::Blob;

    fn decode(value: SQLiteValueRef<'_>) -> Result<Self, DrizzleError> {
        let SQLiteValueRef::Blob(encoded) = value else {
            return Err(DrizzleError::ConversionError(
                "AssetDB dependency target must be stored as BLOB".into(),
            ));
        };
        let Some((&tag, payload)) = encoded.split_first() else {
            return Err(DrizzleError::ConversionError(
                "AssetDB dependency target cannot be empty".into(),
            ));
        };
        match tag {
            Self::GUID_TAG if payload.len() == 16 => {
                let guid = Uuid::from_slice(payload).map_err(|error| {
                    DrizzleError::ConversionError(
                        format!("invalid AssetDB dependency GUID: {error}").into(),
                    )
                })?;
                Ok(Self::Guid(guid))
            }
            Self::GUID_TAG => Err(DrizzleError::ConversionError(
                format!(
                    "AssetDB GUID dependency target must contain 16 bytes, got {}",
                    payload.len()
                )
                .into(),
            )),
            Self::PATH_TAG => {
                let path = std::str::from_utf8(payload).map_err(|error| {
                    DrizzleError::ConversionError(
                        format!("invalid UTF-8 AssetDB dependency path: {error}").into(),
                    )
                })?;
                Self::path(path)
                    .map_err(|error| DrizzleError::ConversionError(error.to_string().into()))
            }
            _ => Err(DrizzleError::ConversionError(
                format!("unknown AssetDB dependency target tag {tag}").into(),
            )),
        }
    }

    fn encode(&self) -> SQLiteValue<'_> {
        SQLiteValue::Blob(Cow::Owned(self.to_bytes()))
    }

    fn encode_owned(self) -> OwnedSQLiteValue {
        OwnedSQLiteValue::Blob(self.to_bytes().into_boxed_slice())
    }
}

#[derive(SQLiteEnum, Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(i64)]
pub enum Status {
    #[default]
    Queued = 0,
    Leased = 1,
    Succeeded = 2,
    Failed = 3,
    Abandoned = 4,
}

impl Status {
    #[must_use]
    pub const fn as_i64(self) -> i64 {
        self as i64
    }

    #[must_use]
    pub const fn can_complete_from_worker(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed)
    }
}

#[derive(SQLiteEnum, Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(i64)]
pub enum Diff {
    #[default]
    Clean = 0,
    Added = 1,
    Modified = 2,
    Deleted = 3,
    Conflicted = 4,
}

impl Diff {
    #[must_use]
    pub const fn as_i64(self) -> i64 {
        self as i64
    }
}

#[derive(SQLiteEnum, Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(i64)]
pub enum Relation {
    #[default]
    SourceToSource = 0,
    JobToJob = 1,
    SourceLikeMatch = 2,
}

impl Relation {
    #[must_use]
    pub const fn as_i64(self) -> i64 {
        self as i64
    }
}

#[derive(SQLiteEnum, Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(i64)]
pub enum Coupling {
    #[default]
    Order = 0,
    Fingerprint = 1,
    OrderOnly = 2,
}

impl Coupling {
    #[must_use]
    pub const fn as_i64(self) -> i64 {
        self as i64
    }
}

#[derive(SQLiteEnum, Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(i64)]
pub enum Registration {
    #[default]
    Registered = 0,
    AssetIdOnly = 1,
}

impl Registration {
    #[must_use]
    pub const fn as_i64(self) -> i64 {
        self as i64
    }
}

#[cfg(test)]
mod tests {
    use drizzle::sqlite::traits::DrizzleSQLiteColumn;

    use super::*;

    #[test]
    fn digest_hex_and_blob_round_trip() {
        let expected = Digest::from(blake3::hash(b"typed AssetDB digest"));
        let hex = expected.to_hex();

        assert_eq!(hex.parse::<Digest>(), Ok(expected));
        assert_eq!(
            Digest::decode(SQLiteValueRef::Blob(expected.as_bytes())).unwrap(),
            expected
        );
        assert_eq!(
            expected.encode(),
            SQLiteValue::Blob(Cow::Borrowed(expected.as_bytes()))
        );
    }

    #[test]
    fn digest_json_is_canonical_hex() {
        let expected = Digest::from(blake3::hash(b"recovery manifest digest"));
        let encoded = serde_json::to_string(&expected).unwrap();
        assert_eq!(encoded, format!("\"{}\"", expected.to_hex()));
        assert_eq!(serde_json::from_str::<Digest>(&encoded).unwrap(), expected);
        assert!(serde_json::from_str::<Digest>("[0,1]").is_err());
    }

    #[test]
    fn digest_rejects_invalid_width_and_storage_class() {
        assert_eq!(
            "00".parse::<Digest>(),
            Err(InvalidDigest::Length { got: 2 })
        );
        assert!(Digest::decode(SQLiteValueRef::Blob(&[0; 31])).is_err());
        assert!(Digest::decode(SQLiteValueRef::Text("00")).is_err());
    }

    #[test]
    fn json_collection_values_round_trip() {
        let exclusions = Exclusions::from(BTreeSet::from([
            "project/overridden.asset".to_string(),
            "project/replaced.asset".to_string(),
        ]));
        let aliases = Aliases::from(vec!["alias/a".to_string(), "alias/b".to_string()]);

        assert_eq!(
            serde_json::from_str::<Exclusions>(&serde_json::to_string(&exclusions).unwrap())
                .unwrap(),
            exclusions
        );
        assert_eq!(
            serde_json::from_str::<Aliases>(&serde_json::to_string(&aliases).unwrap()).unwrap(),
            aliases
        );
    }

    #[test]
    fn sqlite_enums_have_stable_integer_discriminants() {
        assert_eq!(Status::Queued.as_i64(), 0);
        assert_eq!(Status::Abandoned.as_i64(), 4);
        assert_eq!(Diff::Clean.as_i64(), 0);
        assert_eq!(Diff::Conflicted.as_i64(), 4);
        assert_eq!(Relation::SourceToSource.as_i64(), 0);
        assert_eq!(Relation::SourceLikeMatch.as_i64(), 2);
        assert_eq!(Coupling::Order.as_i64(), 0);
        assert_eq!(Coupling::OrderOnly.as_i64(), 2);
        assert_eq!(Registration::Registered.as_i64(), 0);
        assert_eq!(Registration::AssetIdOnly.as_i64(), 1);
        assert_eq!(Work::Plan.as_i64(), 0);
        assert_eq!(Work::Build.as_i64(), 1);
        assert_eq!(Encoding::Ron.as_i64(), 0);
        assert_eq!(Encoding::Bytes.as_i64(), 1);

        assert_eq!(Status::try_from(2).unwrap(), Status::Succeeded);
        assert_eq!(Diff::try_from(2).unwrap(), Diff::Modified);
        assert_eq!(Relation::try_from(1).unwrap(), Relation::JobToJob);
        assert_eq!(Coupling::try_from(1).unwrap(), Coupling::Fingerprint);
        assert_eq!(
            Registration::try_from(1).unwrap(),
            Registration::AssetIdOnly
        );
        assert!(Status::try_from(5).is_err());
    }

    #[test]
    fn dependency_targets_have_one_canonical_blob_encoding() {
        let guid = Target::Guid(Uuid::from_u128(0x0011_2233_4455_6677_8899_aabb_ccdd_eeff));
        let path = Target::path("prefabs/example.prefab.ron").unwrap();

        for expected in [guid, path] {
            let encoded = expected.to_bytes();
            assert_eq!(
                Target::decode(SQLiteValueRef::Blob(&encoded)).unwrap(),
                expected
            );
        }
        assert!(Target::decode(SQLiteValueRef::Blob(&[])).is_err());
        assert!(Target::decode(SQLiteValueRef::Blob(&[Target::GUID_TAG, 1])).is_err());
        assert!(Target::decode(SQLiteValueRef::Blob(&[Target::PATH_TAG])).is_err());
        assert!(
            Target::decode(SQLiteValueRef::Blob(&[
                Target::PATH_TAG,
                b'.',
                b'.',
                b'/',
                b'a'
            ]))
            .is_err()
        );
        assert!(Target::decode(SQLiteValueRef::Blob(&[2, 0])).is_err());
        assert!(Target::decode(SQLiteValueRef::Text("uuid:legacy")).is_err());
        for invalid in ["", "/rooted", "trailing/", "a\\b", "a//b", "a/../b"] {
            assert!(Target::path(invalid).is_err(), "{invalid}");
        }
    }
}
