//! `GradientSignal` reflected type identifiers.

use uuid::{Uuid, uuid};

/// `VegetationGradientSampler` AZ type UUID.
pub const VEGETATION_GRADIENT_SAMPLER_TYPE_ID: Uuid = uuid!("00BD6356-F371-475F-A2E2-0A0C3638BD86");
/// `VegetationConstantGradientConfig` AZ type UUID.
pub const VEGETATION_CONSTANT_GRADIENT_CONFIG_TYPE_ID: Uuid =
    uuid!("7CC6479B-CAA6-4288-8E04-9EBF48CE5C41");
/// `VegetationConstantGradientComponent` AZ type UUID.
pub const VEGETATION_CONSTANT_GRADIENT_COMPONENT_TYPE_ID: Uuid =
    uuid!("12FAFFDD-CD31-4D16-92D2-91F3C9434A7B");
/// `VegetationThresholdGradientConfig` AZ type UUID.
pub const VEGETATION_THRESHOLD_GRADIENT_CONFIG_TYPE_ID: Uuid =
    uuid!("2F68B383-747B-499D-8CDB-C4E4C63B9445");
/// `VegetationThresholdGradientComponent` AZ type UUID.
pub const VEGETATION_THRESHOLD_GRADIENT_COMPONENT_TYPE_ID: Uuid =
    uuid!("1F3FD6EF-0BF0-4E1A-98C4-D7176EB4484C");
/// `VegetationInvertGradientConfig` AZ type UUID.
pub const VEGETATION_INVERT_GRADIENT_CONFIG_TYPE_ID: Uuid =
    uuid!("000D4B39-CAB8-4950-9C50-692584073526");
/// `VegetationInvertGradientComponent` AZ type UUID.
pub const VEGETATION_INVERT_GRADIENT_COMPONENT_TYPE_ID: Uuid =
    uuid!("305E36B8-2BFE-4B80-8831-DFA3BC6E7AE0");
/// `VegetationLevelsGradientConfig` AZ type UUID.
pub const VEGETATION_LEVELS_GRADIENT_CONFIG_TYPE_ID: Uuid =
    uuid!("A2BF0FE8-2AAC-45F4-8CDD-2FECED927BF5");
/// `VegetationLevelsGradientComponent` AZ type UUID.
pub const VEGETATION_LEVELS_GRADIENT_COMPONENT_TYPE_ID: Uuid =
    uuid!("2EB048B1-2623-4F17-8A86-1CF8075FFD1E");
/// `VegetationPerlinGradientConfig` AZ type UUID.
pub const VEGETATION_PERLIN_GRADIENT_CONFIG_TYPE_ID: Uuid =
    uuid!("AC02AF00-B9F2-46D1-9EAC-7DB918269B81");
/// `VegetationPerlinGradientComponent` AZ type UUID.
pub const VEGETATION_PERLIN_GRADIENT_COMPONENT_TYPE_ID: Uuid =
    uuid!("8C95DACD-84CC-42C5-A49E-5E7A94DBA0EE");
/// `VegetationRandomGradientConfig` AZ type UUID.
pub const VEGETATION_RANDOM_GRADIENT_CONFIG_TYPE_ID: Uuid =
    uuid!("705D70DA-EF33-4CE3-903E-3B61C9C3B085");
/// `VegetationRandomGradientComponent` AZ type UUID.
pub const VEGETATION_RANDOM_GRADIENT_COMPONENT_TYPE_ID: Uuid =
    uuid!("2DFECFD9-7623-49AC-9BCA-704972E6B24B");
/// `VegetationGradientTransformConfig` AZ type UUID.
pub const VEGETATION_GRADIENT_TRANSFORM_CONFIG_TYPE_ID: Uuid =
    uuid!("4FEE431F-3BAC-456B-9C5D-226422A9B2AD");
/// `VegetationGradientTransformComponent` AZ type UUID.
pub const VEGETATION_GRADIENT_TRANSFORM_COMPONENT_TYPE_ID: Uuid =
    uuid!("9CB66205-301C-430B-8339-957534CAEFDF");

/// Lumberyard `GradientSignal::GradientSampler` AZ RTTI UUID.
pub const GRADIENT_SAMPLER_LUMBERYARD_TYPE_ID: Uuid = uuid!("3768D3A6-BF70-4ABC-B4EC-73C75A886916");
