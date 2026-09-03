use uuid::Uuid;

/// Lumberyard `LmbrCentral::LightComponent` AZ component UUID.
pub const LIGHT_COMPONENT_TYPE_ID: &str = "6B9AB512-CA8A-4D2B-B570-DF128EA7CE6A";
pub const LIGHT_COMPONENT_TYPE_UUID: Uuid = Uuid::from_u128(0x6B9AB512_CA8A_4D2B_B570_DF128EA7CE6A);

/// Lumberyard `LmbrCentral::LightConfiguration` type UUID.
pub const LIGHT_CONFIGURATION_TYPE_ID: &str = "F4CC7BB4-C541-480C-88FC-C5A8F37CC67F";
pub const LIGHT_CONFIGURATION_TYPE_UUID: Uuid =
    Uuid::from_u128(0xF4CC7BB4_C541_480C_88FC_C5A8F37CC67F);
