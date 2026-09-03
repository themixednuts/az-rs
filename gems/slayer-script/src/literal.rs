//! Generic `SlayerScript` catalog keys and authoring names.

use az_core::{crc::Crc32, name::AzName};
use az_derive::AzRtti;
use bevy::prelude::Reflect;
use gridmate::Marshaler;
use serde::{Deserialize, Serialize};

/// Cooked case-insensitive key used by the native template factory registry.
#[derive(
    AzRtti,
    Debug,
    Default,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Marshaler,
    Serialize,
    Deserialize,
    Reflect,
)]
#[az_rtti(name = "SlayerScriptLiteral", "F4F725DD-D22B-4DC1-8CC2-FD99E7B4CD66")]
pub struct SlayerScriptLiteral {
    /// Native `m_crc` field and catalog lookup word.
    #[serde(rename = "m_crc", default)]
    pub crc: u32,
}

impl SlayerScriptLiteral {
    pub const EMPTY: Self = Self { crc: 0 };

    #[must_use]
    pub const fn new(crc: Crc32) -> Self {
        Self { crc: crc.value() }
    }

    #[must_use]
    pub const fn crc32(self) -> Crc32 {
        Crc32::from_u32(self.crc)
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.crc == 0
    }
}

impl From<Crc32> for SlayerScriptLiteral {
    fn from(value: Crc32) -> Self {
        Self::new(value)
    }
}

impl From<&str> for SlayerScriptLiteral {
    fn from(value: &str) -> Self {
        Self::new(Crc32::from_str_lower(value))
    }
}

/// Editable generic `SlayerScript` name plus its cooked catalog key.
#[derive(
    AzRtti, Debug, Default, Clone, PartialEq, Eq, Hash, Marshaler, Serialize, Deserialize, Reflect,
)]
#[az_rtti(
    name = "SlayerScriptEditLiteral",
    "4CAC7A1B-5D32-4AEF-9722-7E2F5CB38635"
)]
pub struct SlayerScriptEditLiteral {
    /// Native `m_string` field.
    #[serde(rename = "m_string", default)]
    pub string: String,
    /// Native `m_crc` field.
    #[serde(rename = "m_crc", default)]
    pub crc: u32,
}

impl SlayerScriptEditLiteral {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        let string = value.into();
        let crc = Crc32::from_str_lower(&string).value();
        Self { string, crc }
    }

    #[must_use]
    pub const fn literal(&self) -> SlayerScriptLiteral {
        SlayerScriptLiteral { crc: self.crc }
    }
}

/// Authoring-side generic name that cooks to a catalog literal.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize, Reflect)]
#[serde(transparent)]
pub struct SlayerScriptName(AzName);

impl SlayerScriptName {
    #[must_use]
    pub fn cook(&self) -> SlayerScriptLiteral {
        SlayerScriptLiteral::from(self.0.as_str())
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl AsRef<str> for SlayerScriptName {
    fn as_ref(&self) -> &str {
        self.0.as_str()
    }
}

impl std::borrow::Borrow<str> for SlayerScriptName {
    fn borrow(&self) -> &str {
        self.0.as_str()
    }
}

impl From<&str> for SlayerScriptName {
    fn from(value: &str) -> Self {
        Self(AzName::from(value))
    }
}

impl From<String> for SlayerScriptName {
    fn from(value: String) -> Self {
        Self(AzName::from(value))
    }
}

#[cfg(test)]
mod tests {
    use az_core::AzTypeInfo;

    use super::*;

    #[test]
    fn names_cook_to_case_insensitive_literals() {
        let name = SlayerScriptName::from("EncounterStart");
        let editable = SlayerScriptEditLiteral::new("ENCOUNTERSTART");

        assert_eq!(name.cook(), editable.literal());
        assert_eq!(name.as_ref(), "EncounterStart");
        assert!(!name.is_empty());
        assert!(SlayerScriptLiteral::EMPTY.is_empty());
    }

    #[test]
    fn native_type_ids_match_the_generic_module_contract() {
        assert_eq!(
            SlayerScriptLiteral::TYPE_ID,
            uuid::uuid!("F4F725DD-D22B-4DC1-8CC2-FD99E7B4CD66")
        );
        assert_eq!(
            SlayerScriptEditLiteral::TYPE_ID,
            uuid::uuid!("4CAC7A1B-5D32-4AEF-9722-7E2F5CB38635")
        );
    }
}
