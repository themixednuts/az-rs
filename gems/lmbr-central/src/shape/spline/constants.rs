use uuid::Uuid;

/// Lumberyard `LmbrCentral::SplineCommon` type UUID.
pub const SPLINE_COMMON_TYPE_ID: &str = "91A31D7E-F63A-4AA8-BC50-909B37F0AD8B";
pub const SPLINE_COMMON_TYPE_UUID: Uuid = Uuid::from_u128(0x91A31D7E_F63A_4AA8_BC50_909B37F0AD8B);

/// Lumberyard `LmbrCentral::SplineComponent` AZ component UUID.
pub const SPLINE_COMPONENT_TYPE_ID: &str = "F0905297-1E24-4044-BFDA-BDE3583F1E57";
pub const SPLINE_COMPONENT_TYPE_UUID: Uuid =
    Uuid::from_u128(0xF0905297_1E24_4044_BFDA_BDE3583F1E57);

/// Lumberyard `AZ::Spline` type UUID.
pub const SPLINE_TYPE_ID: &str = "6E2D31AF-5CB0-4A50-BD68-B00E2D2FD0A4";
pub const SPLINE_TYPE_UUID: Uuid = Uuid::from_u128(0x6E2D31AF_5CB0_4A50_BD68_B00E2D2FD0A4);

/// Lumberyard `AZStd::shared_ptr<AZ::Spline>` reflected field wrapper.
///
/// Lumberyard fixture reference: `dev/StarterGame/slices/Roads.slice`.
pub const SPLINE_SHARED_PTR_TYPE_ID: &str = "E13859C4-1F24-5C44-A133-F17B4B050D7C";
pub const SPLINE_SHARED_PTR_TYPE_UUID: Uuid =
    Uuid::from_u128(0xE13859C4_1F24_5C44_A133_F17B4B050D7C);

/// Lumberyard `AZ::LinearSpline` type UUID.
pub const LINEAR_SPLINE_TYPE_ID: &str = "DD80E118-12C9-4F69-848B-4EA5DAA2E0EC";
pub const LINEAR_SPLINE_TYPE_UUID: Uuid = Uuid::from_u128(0xDD80E118_12C9_4F69_848B_4EA5DAA2E0EC);

/// Lumberyard `AZ::BezierSpline` type UUID.
pub const BEZIER_SPLINE_TYPE_ID: &str = "C1A48956-5CBC-4124-AB49-61FFEEE9139A";
pub const BEZIER_SPLINE_TYPE_UUID: Uuid = Uuid::from_u128(0xC1A48956_5CBC_4124_AB49_61FFEEE9139A);

/// Lumberyard `AZ::BezierSpline::BezierData` type UUID.
pub const BEZIER_DATA_TYPE_ID: &str = "6C34069E-AEA2-44A2-877F-BED9CE07DA6B";
pub const BEZIER_DATA_TYPE_UUID: Uuid = Uuid::from_u128(0x6C34069E_AEA2_44A2_877F_BED9CE07DA6B);

/// Lumberyard `AZ::CatmullRomSpline` type UUID.
pub const CATMULL_ROM_SPLINE_TYPE_ID: &str = "B4AD0E71-92D8-4888-AB89-5C3B4A30759A";
pub const CATMULL_ROM_SPLINE_TYPE_UUID: Uuid =
    Uuid::from_u128(0xB4AD0E71_92D8_4888_AB89_5C3B4A30759A);

/// Lumberyard `AZ::VertexContainer<AZ::Vector2>` type UUID.
pub const VERTEX_CONTAINER_VEC2_TYPE_ID: &str = "EBE98B36-0783-5226-9739-064BD41EBB52";
pub const VERTEX_CONTAINER_VEC2_TYPE_UUID: Uuid =
    Uuid::from_u128(0xEBE98B36_0783_5226_9739_064BD41EBB52);

/// Lumberyard `AZ::VertexContainer<AZ::Vector3>` type UUID.
pub const VERTEX_CONTAINER_VEC3_TYPE_ID: &str = "A6F50685-C884-50C6-AD08-123028C77954";
pub const VERTEX_CONTAINER_VEC3_TYPE_UUID: Uuid =
    Uuid::from_u128(0xA6F50685_C884_50C6_AD08_123028C77954);

/// Minimum editor granularity used by curved spline types.
///
/// O3DE reference: `Code/Framework/AzCore/AzCore/Math/Spline.cpp:27`.
pub const MIN_SPLINE_GRANULARITY: u16 = 2;

/// Maximum editor granularity used by curved spline types.
///
/// O3DE reference: `Code/Framework/AzCore/AzCore/Math/Spline.cpp:28`.
pub const MAX_SPLINE_GRANULARITY: u16 = 64;
