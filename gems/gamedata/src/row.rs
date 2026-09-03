use std::hash::Hash;
use std::num::{NonZeroI8, NonZeroI16, NonZeroI32, NonZeroU8, NonZeroU16, NonZeroU32};

use az_core::crc::Crc32;

use crate::GameDataError;
use crate::Row;
use crate::descriptor::RowSchemaDescriptor;
use crate::game_system::SchemaRowRef;

/// Merged row-schema structs implement this through `#[derive(GameDataRow)]`.
pub trait GameDataRow: Row {
    const ROW_NAME: &'static str = <Self as Row>::NAME;
    const KEY_FIELD_NAMES: &'static [&'static str];
}

/// A merged row schema that can materialize itself from any compatible
/// physical `GameData` table.
pub trait GameDataSchemaRow: GameDataRow + Send + Sync + Sized + 'static {
    const SCHEMA: RowSchemaDescriptor;

    /// Materializes one row of this schema from a physical table row.
    ///
    /// # Errors
    ///
    /// Returns [`GameDataError::Decode`] for the first field the row cannot
    /// supply: a required field the physical table omits, a cell whose stored
    /// type does not match the schema's declared cell type, or a string or
    /// list cell that points outside the table-asset bytes. The generated
    /// implementation delegates each field to [`SchemaRowRef::get`] or
    /// [`SchemaRowRef::require`].
    fn decode(row: SchemaRowRef<'_, '_, Self>) -> Result<Self, GameDataError>;
}

/// Authored primary-key value that can be converted to the compiled row key CRC.
pub trait GameDataKeyValue: Clone + Eq + Hash + 'static {
    #[must_use]
    fn key_crc(&self) -> u32;
}

impl GameDataKeyValue for String {
    #[inline]
    fn key_crc(&self) -> u32 {
        Crc32::from_str_lower(self).value()
    }
}

impl GameDataKeyValue for Crc32 {
    #[inline]
    fn key_crc(&self) -> u32 {
        self.value()
    }
}

impl GameDataKeyValue for crate::RowIndex {
    #[inline]
    fn key_crc(&self) -> u32 {
        self.one_based().get()
    }
}

macro_rules! unsigned_key_value {
    ($($ty:ty),* $(,)?) => {
        $(
            impl GameDataKeyValue for $ty {
                #[inline]
                fn key_crc(&self) -> u32 {
                    u32::from(*self)
                }
            }
        )*
    };
}

macro_rules! signed_key_value {
    ($($ty:ty),* $(,)?) => {
        $(
            impl GameDataKeyValue for $ty {
                #[inline]
                fn key_crc(&self) -> u32 {
                    u32::from_ne_bytes(i32::from(*self).to_ne_bytes())
                }
            }
        )*
    };
}

unsigned_key_value!(u8, u16, u32);
signed_key_value!(i8, i16, i32);

impl GameDataKeyValue for NonZeroU8 {
    #[inline]
    fn key_crc(&self) -> u32 {
        u32::from(self.get())
    }
}

impl GameDataKeyValue for NonZeroU16 {
    #[inline]
    fn key_crc(&self) -> u32 {
        u32::from(self.get())
    }
}

impl GameDataKeyValue for NonZeroU32 {
    #[inline]
    fn key_crc(&self) -> u32 {
        self.get()
    }
}

impl GameDataKeyValue for NonZeroI8 {
    #[inline]
    fn key_crc(&self) -> u32 {
        u32::from_ne_bytes(i32::from(self.get()).to_ne_bytes())
    }
}

impl GameDataKeyValue for NonZeroI16 {
    #[inline]
    fn key_crc(&self) -> u32 {
        u32::from_ne_bytes(i32::from(self.get()).to_ne_bytes())
    }
}

impl GameDataKeyValue for NonZeroI32 {
    #[inline]
    fn key_crc(&self) -> u32 {
        u32::from_ne_bytes(self.get().to_ne_bytes())
    }
}

#[cfg(all(test, feature = "derive"))]
mod tests {
    use super::*;

    #[allow(non_camel_case_types)]
    #[derive(gamedata_derive::GameDataRow)]
    #[expect(
        dead_code,
        reason = "these fields exist for the GameDataRow derive to read at expansion time; the tests assert on what it generates, never on the values"
    )]
    struct damage_data {
        #[key]
        damage_id: String,
        #[key]
        house_item_id: u32,
    }

    #[derive(gamedata_derive::GameDataRow)]
    #[row(name = "Special.DAT")]
    #[expect(
        dead_code,
        reason = "this field exists for the GameDataRow derive to read at expansion time; the tests assert on what it generates, never on the value"
    )]
    struct SpecialData {
        #[key]
        id: String,
    }

    #[test]
    fn derive_infers_row_name_with_heck_and_marks_key_candidates() {
        assert_eq!(<damage_data as Row>::NAME, "DamageData");
        assert_eq!(
            <damage_data as GameDataRow>::KEY_FIELD_NAMES,
            &["damage_id", "house_item_id"]
        );
    }

    #[test]
    fn row_name_override_is_reserved_for_non_heck_names() {
        assert_eq!(<SpecialData as Row>::NAME, "Special.DAT");
    }
}
