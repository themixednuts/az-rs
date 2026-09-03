//! Cooked animation-event database data keyed by canonical product identity.

use std::{
    collections::BTreeMap,
    io::{self, Write},
    path::Path,
    sync::Arc,
};

use az_asset_builder::{
    BuildProduct, BuildRule, BuilderId, CreateJobsRequest, CreateJobsResponse, JobContext,
    JobDescriptor, ProcessJobRequest, ProcessJobResponse, ProcessJobResult, ProductDependency,
    ProductFormat, SourceFormat, SourceMetaError, TypedBuildProduct, resolve_referenced_product_id,
};
use az_core::{AssetData, AssetId, AzRtti, AzTypeInfo};
use az_filesystem::{engine_path_with_extension_key, normalize_source_path};
use bevy_asset::Asset;
use bevy_math::Vec3;
use bevy_reflect::{
    Reflect, ReflectDeserialize, ReflectSerialize, TypePath, std_traits::ReflectDefault,
};
use ron::ser::PrettyConfig;
use serde::{Deserialize, Serialize};
use uuid::{Uuid, uuid};

pub const ANIMATION_EVENT_LIST_SOURCE_SCHEMA_NAME: &str =
    "azoth.animation.AnimationEventListSource";
pub const ANIMATION_EVENT_LIST_BUILDER_NAME: &str = "azoth.animation.event-database";
pub const ANIMATION_EVENT_LIST_BUILDER_ID: BuilderId =
    BuilderId::new(uuid!("26b72712-e523-43cd-92b3-9f476a874ee4"));
pub const ANIMATION_EVENT_LIST_VERSION: u32 = 1;
const PRIMARY_PRODUCT_SUB_ID: u32 = 0;

#[derive(SourceFormat)]
#[source(
    schema = "azoth.animation.AnimationEventListSource",
    ext = "animevents.ron"
)]
pub struct AnimationEventListSourceFormat;

#[derive(Debug, Clone, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct AnimationEventListSource {
    pub source_path: String,
    pub animations: Vec<AnimationEventsSource>,
}

impl AnimationEventListSource {
    /// Serializes the source list to pretty RON with a trailing newline.
    ///
    /// # Errors
    ///
    /// Returns the [`ron::Error`] raised while serializing this list.
    pub fn to_ron_bytes(&self) -> Result<Vec<u8>, ron::Error> {
        let ron = ron::ser::to_string_pretty(self, PrettyConfig::default())?;
        Ok(format!("{ron}\n").into_bytes())
    }

    /// Parses a source list from its RON representation.
    ///
    /// # Errors
    ///
    /// Returns a [`ron::error::SpannedError`] when `bytes` is not valid RON for
    /// this type.
    pub fn from_ron_bytes(bytes: &[u8]) -> Result<Self, ron::error::SpannedError> {
        ron::de::from_bytes(bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct AnimationEventsSource {
    pub animation: String,
    pub events: Vec<AnimationEventSource>,
}

#[derive(Debug, Clone, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct AnimationEventSource {
    pub name: String,
    pub time: f32,
    pub end_time: f32,
    pub parameter: String,
    pub bone: String,
    pub second_bone: String,
    pub offset: AnimationEventVec3,
    pub direction: AnimationEventVec3,
    pub model: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AnimationEventVec3(pub [f32; 3]);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnimationEventData {
    pub name: Arc<str>,
    pub time: f32,
    pub end_time: f32,
    pub parameter: Arc<str>,
    pub bone_name_1: Arc<str>,
    pub bone_name_2: Arc<str>,
    pub offset: Vec3,
    pub direction: Vec3,
    pub model: Arc<str>,
}

impl AnimationEventData {
    #[must_use]
    pub fn duration(&self) -> f32 {
        (self.end_time - self.time).max(0.0)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnimationEventTrack {
    pub motion: AssetId,
    pub animation_name: Arc<str>,
    pub events: Vec<AnimationEventData>,
}

#[derive(Asset, TypePath, Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AnimationEventDatabaseAsset {
    tracks: Vec<AnimationEventTrack>,
}

impl AnimationEventDatabaseAsset {
    #[must_use]
    pub fn new(mut tracks: Vec<AnimationEventTrack>) -> Self {
        tracks.sort_unstable_by_key(|track| track.motion);
        for track in &mut tracks {
            track.events.sort_by(|left, right| {
                left.time
                    .total_cmp(&right.time)
                    .then_with(|| left.end_time.total_cmp(&right.end_time))
            });
        }
        Self { tracks }
    }

    #[must_use]
    pub fn tracks(&self) -> &[AnimationEventTrack] {
        &self.tracks
    }

    #[must_use]
    pub fn track(&self, motion: AssetId) -> Option<&AnimationEventTrack> {
        self.tracks
            .binary_search_by_key(&motion, |track| track.motion)
            .ok()
            .map(|index| &self.tracks[index])
    }

    pub fn referenced_asset_ids(&self) -> impl Iterator<Item = AssetId> + '_ {
        self.tracks.iter().map(|track| track.motion)
    }
}

impl AzTypeInfo for AnimationEventDatabaseAsset {
    const NAME: &'static str = "Azoth::AnimationEventDatabaseAsset";
    const TYPE_ID: Uuid = uuid!("e45cbf56-df80-4d7d-b69e-b9d951b517d6");
}

impl AzRtti for AnimationEventDatabaseAsset {}

impl AssetData for AnimationEventDatabaseAsset {
    const STABLE_NAME: &'static str = "azoth.animation.event-database";
}

#[derive(ProductFormat)]
#[product_format(
    id = "azoth.animation.event-database",
    version = 1,
    asset = AnimationEventDatabaseAsset
)]
pub struct AnimationEventDatabaseProductFormat;

/// Encodes a cooked animation-event database and writes it to `writer`.
///
/// # Errors
///
/// Returns [`AnimationEventDatabaseCodecError::Codec`] when postcard fails to
/// serialize `asset`, and [`AnimationEventDatabaseCodecError::Write`] when the
/// writer rejects the encoded bytes.
pub fn write_animation_event_database(
    asset: &AnimationEventDatabaseAsset,
    writer: &mut impl Write,
) -> Result<(), AnimationEventDatabaseCodecError> {
    writer.write_all(&postcard::to_allocvec(asset)?)?;
    Ok(())
}

/// Decodes a cooked animation-event database from its postcard representation.
///
/// # Errors
///
/// Returns [`AnimationEventDatabaseCodecError::Codec`] when `bytes` is not a
/// valid postcard encoding of an [`AnimationEventDatabaseAsset`].
pub fn read_animation_event_database(
    bytes: &[u8],
) -> Result<AnimationEventDatabaseAsset, AnimationEventDatabaseCodecError> {
    Ok(postcard::from_bytes(bytes)?)
}

#[derive(Debug, thiserror::Error)]
pub enum AnimationEventDatabaseCodecError {
    #[error("encode or decode animation-event database: {0}")]
    Codec(#[from] postcard::Error),
    #[error("write animation-event database: {0}")]
    Write(#[from] io::Error),
}

#[must_use]
pub fn animation_event_list_build_rule(_: &JobContext<'_>) -> BuildRule {
    BuildRule::for_source::<AnimationEventListSourceFormat>()
        .named(ANIMATION_EVENT_LIST_BUILDER_NAME)
        .id(ANIMATION_EVENT_LIST_BUILDER_ID)
        .version(ANIMATION_EVENT_LIST_VERSION)
        .produces::<AnimationEventDatabaseProductFormat>()
        .create_jobs(create_jobs)
        .process(animation_event_list_process_job)
}

fn create_jobs(request: &CreateJobsRequest<'_>) -> CreateJobsResponse {
    CreateJobsResponse {
        jobs: request
            .platforms
            .iter()
            .copied()
            .map(JobDescriptor::default_for_platform)
            .collect(),
        ..CreateJobsResponse::default()
    }
}

fn animation_event_list_process_job(request: &ProcessJobRequest<'_>) -> ProcessJobResponse {
    match transform_animation_event_list_product(
        request.source_root,
        &request.source_path,
        request.source_bytes,
    ) {
        Ok(build) => ProcessJobResponse {
            products: vec![build.product],
            product_dependencies: build
                .dependencies
                .into_iter()
                .map(|dependency| (PRIMARY_PRODUCT_SUB_ID as usize, dependency))
                .collect(),
            result: ProcessJobResult::Success,
        },
        Err(error) => {
            tracing::warn!(%error, "animation-event product failed");
            ProcessJobResponse {
                result: ProcessJobResult::Failed,
                ..ProcessJobResponse::default()
            }
        }
    }
}

pub struct AnimationEventListBuild {
    pub product: BuildProduct,
    pub dependencies: Vec<ProductDependency>,
}

/// Cooks one authored animation-event list into its build product.
///
/// # Errors
///
/// Returns [`AnimationEventListProductError::ParseSource`] when `source_bytes`
/// is not valid RON, [`AnimationEventListProductError::UnsupportedMotion`] for
/// a referenced motion that is not a motion or blend-space authoring source,
/// [`AnimationEventListProductError::ConflictingMotion`] when two sources
/// resolve to the same asset id,
/// [`AnimationEventListProductError::NonFiniteEvent`] for an event carrying a
/// non-finite time, offset, or direction, and
/// [`AnimationEventListProductError::Serialize`] when the cooked database
/// cannot be encoded.
pub fn transform_animation_event_list_product(
    source_root: &Path,
    source_path: &str,
    source_bytes: &[u8],
) -> Result<AnimationEventListBuild, AnimationEventListProductError> {
    let source = AnimationEventListSource::from_ron_bytes(source_bytes)?;
    let mut tracks = BTreeMap::<AssetId, (String, Vec<AnimationEventData>)>::new();

    for animation in source.animations {
        let animation_name = normalize_source_path(&animation.animation);
        let product_source = animation_event_motion_source(&animation_name).ok_or_else(|| {
            AnimationEventListProductError::UnsupportedMotion {
                motion: animation_name.clone(),
            }
        })?;
        let motion =
            resolve_referenced_product_id(source_root, &product_source, PRIMARY_PRODUCT_SUB_ID)?;
        let track = tracks
            .entry(motion)
            .or_insert_with(|| (animation_name.clone(), Vec::new()));
        if track.0 != animation_name {
            return Err(AnimationEventListProductError::ConflictingMotion {
                asset_id: motion,
                first: track.0.clone(),
                second: animation_name,
            });
        }

        for event in animation.events {
            if !event.time.is_finite()
                || !event.end_time.is_finite()
                || event.offset.0.iter().any(|value| !value.is_finite())
                || event.direction.0.iter().any(|value| !value.is_finite())
            {
                return Err(AnimationEventListProductError::NonFiniteEvent {
                    motion: track.0.clone(),
                    event: event.name,
                });
            }
            track.1.push(AnimationEventData {
                name: event.name.into(),
                time: event.time,
                end_time: event.end_time,
                parameter: event.parameter.into(),
                bone_name_1: event.bone.into(),
                bone_name_2: event.second_bone.into(),
                offset: Vec3::from_array(event.offset.0),
                direction: Vec3::from_array(event.direction.0),
                model: event.model.into(),
            });
        }
    }

    let asset = AnimationEventDatabaseAsset::new(
        tracks
            .into_iter()
            .map(|(motion, (animation_name, events))| AnimationEventTrack {
                motion,
                animation_name: animation_name.into(),
                events,
            })
            .collect(),
    );
    let dependencies = asset
        .referenced_asset_ids()
        .map(ProductDependency::new)
        .collect();
    let mut bytes = Vec::new();
    write_animation_event_database(&asset, &mut bytes)?;
    let product = TypedBuildProduct::<AnimationEventDatabaseProductFormat>::from_trusted_path(
        animation_event_list_product_path(source_path),
        PRIMARY_PRODUCT_SUB_ID,
        bytes,
    )
    .erase();

    Ok(AnimationEventListBuild {
        product,
        dependencies,
    })
}

fn animation_event_motion_source(path: &str) -> Option<String> {
    if path.ends_with(".anim.glb") || path.ends_with(".bspace.ron") || path.ends_with(".comb.ron") {
        return Some(path.to_owned());
    }
    path.strip_suffix(".caf")
        .map(|stem| format!("{stem}.anim.glb"))
        .or_else(|| {
            path.strip_suffix(".bspace")
                .map(|stem| format!("{stem}.bspace.ron"))
        })
        .or_else(|| {
            path.strip_suffix(".comb")
                .map(|stem| format!("{stem}.comb.ron"))
        })
}

#[must_use]
pub fn animation_event_list_product_path(source_path: &str) -> String {
    engine_path_with_extension_key(
        source_path,
        "",
        "animation.events.bin",
        Some("animevents.ron"),
    )
}

#[derive(Debug, thiserror::Error)]
pub enum AnimationEventListProductError {
    #[error("parse animation-event source RON: {0}")]
    ParseSource(#[from] ron::error::SpannedError),
    #[error("animation-event motion `{motion}` is not a motion or blend-space authoring source")]
    UnsupportedMotion { motion: String },
    #[error(
        "animation-event sources `{first}` and `{second}` resolve to the same asset id {asset_id}"
    )]
    ConflictingMotion {
        asset_id: AssetId,
        first: String,
        second: String,
    },
    #[error("animation-event `{event}` for `{motion}` contains a non-finite value")]
    NonFiniteEvent { motion: String, event: String },
    #[error("serialize animation-event product: {0}")]
    Serialize(#[from] AnimationEventDatabaseCodecError),
    #[error("read referenced source metadata: {0}")]
    SourceMeta(#[from] SourceMetaError),
}

#[cfg(test)]
mod tests {
    use az_asset_builder::{SourceFormat, source_asset_id};

    use super::*;

    #[test]
    fn event_product_resolves_direct_and_parametric_motion_identities() {
        let source = AnimationEventListSource {
            source_path: "animations/events/hero.animevents".to_owned(),
            animations: vec![
                animation_events("Animations/Hero/Attack.caf", "hit", 0.4),
                animation_events("Animations/Hero/Locomotion.bspace", "step", 0.2),
                animation_events("Animations/Hero/Travel.comb", "stride", 0.7),
            ],
        };
        let source_bytes = source.to_ron_bytes().expect("serialize event source");
        let build = transform_animation_event_list_product(
            Path::new("."),
            "animations/events/hero.animevents.ron",
            &source_bytes,
        )
        .expect("build event product");
        let database =
            read_animation_event_database(&build.product.bytes).expect("decode event product");
        let mut expected = [
            ("animations/hero/attack.anim.glb", "hit"),
            ("animations/hero/locomotion.bspace.ron", "step"),
            ("animations/hero/travel.comb.ron", "stride"),
        ]
        .map(|(path, event)| (source_asset_id(path, 0), event));
        expected.sort_by_key(|(asset_id, _)| *asset_id);

        assert_eq!(
            build.product.product_path.as_str(),
            "animations/events/hero.animation.events.bin"
        );
        assert_eq!(
            build
                .dependencies
                .iter()
                .map(|dependency| dependency.dependency_id)
                .collect::<Vec<_>>(),
            expected
                .iter()
                .map(|(asset_id, _)| *asset_id)
                .collect::<Vec<_>>()
        );
        for (asset_id, event_name) in expected {
            let track = database.track(asset_id).expect("motion event track");
            assert_eq!(track.events.len(), 1);
            assert_eq!(track.events[0].name.as_ref(), event_name);
        }
        assert_eq!(
            AnimationEventListSourceFormat::SCHEMA
                .expect("event source schema")
                .as_str(),
            ANIMATION_EVENT_LIST_SOURCE_SCHEMA_NAME
        );
    }

    fn animation_events(path: &str, event: &str, time: f32) -> AnimationEventsSource {
        AnimationEventsSource {
            animation: path.to_owned(),
            events: vec![AnimationEventSource {
                name: event.to_owned(),
                time,
                end_time: time,
                parameter: String::new(),
                bone: String::new(),
                second_bone: String::new(),
                offset: AnimationEventVec3([0.0; 3]),
                direction: AnimationEventVec3([0.0; 3]),
                model: String::new(),
            }],
        }
    }
}
