use uuid::Uuid;

/// `LmbrCentral::ParticleComponent` AZ component UUID.
pub const PARTICLE_COMPONENT_TYPE_ID: &str = "65BC817A-ABF6-440F-AD4F-581C40F92795";
pub const PARTICLE_COMPONENT_TYPE_UUID: Uuid =
    Uuid::from_u128(0x65BC817A_ABF6_440F_AD4F_581C40F92795);

/// Lumberyard `LmbrCentral::ParticleEmitterSettings` type UUID.
pub const PARTICLE_EMITTER_SETTINGS_TYPE_ID: &str = "A1E34557-30DB-4716-B4CE-39D52A113D0C";
pub const PARTICLE_EMITTER_SETTINGS_TYPE_UUID: Uuid =
    Uuid::from_u128(0xA1E34557_30DB_4716_B4CE_39D52A113D0C);

/// `LmbrCentral::ParticleEmitBoneLayer` type UUID.
pub const PARTICLE_EMIT_BONE_LAYER_TYPE_UUID: Uuid =
    Uuid::from_u128(0xD29E0CF9_8F02_4E61_BBDE_7BEB76D13FE5);
