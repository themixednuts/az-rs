//! Compile-time identity for merged `GameData` row schemas.

use az_core::crc::Crc32;

/// Compile-time row identity for one `GameData` table row type.
pub trait Row: Send + Sync + 'static {
    const NAME: &'static str;
    const CRC: u32 = Crc32::from_str_lower(Self::NAME).value();
}

impl Row for () {
    const NAME: &'static str = "";
    const CRC: u32 = 0;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
    struct AchievementData;

    impl Row for AchievementData {
        const NAME: &'static str = "AchievementData";
    }

    #[test]
    fn row_crc_derives_from_name() {
        assert_eq!(
            AchievementData::CRC,
            Crc32::from_str_lower("AchievementData").value()
        );
    }
}
