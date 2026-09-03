use az_asset::AssetId;
use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use super::variants::{BezierSpline, CatmullRomSpline, LinearSpline, Spline, SplineType};

/// Spline component configuration.
///
/// O3DE reference: `Gems/LmbrCentral/Code/Source/Shape/SplineComponent.cpp:61`.
#[derive(Debug, Clone, Default, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct SplineCommon {
    pub spline: Spline,
}

impl SplineCommon {
    #[must_use]
    pub const fn spline_type(&self) -> SplineType {
        self.spline.spline_type()
    }

    pub fn change_spline_type(&mut self, spline_type: SplineType) {
        if self.spline_type() == spline_type {
            return;
        }

        let spline_data = self.spline.data().clone();
        self.spline = match spline_type {
            SplineType::Linear => Spline::Linear(LinearSpline {
                spline: spline_data,
            }),
            SplineType::Bezier => Spline::Bezier(BezierSpline {
                spline: spline_data,
                ..Default::default()
            }),
            SplineType::CatmullRom => Spline::CatmullRom(CatmullRomSpline {
                spline: spline_data,
                ..Default::default()
            }),
        };
    }
}

/// Runtime spline component.
///
/// O3DE reference: `Gems/LmbrCentral/Code/Source/Shape/SplineComponent.cpp:162`.
#[derive(Component, Debug, Clone, PartialEq, Reflect, Serialize, Deserialize, Prefab)]
#[reflect(Component, Default, Serialize, Deserialize, Prefab)]
// Azoth prefab-format versioning starts at 1 and only bumps with real migration
// steps once documents ship. Independent of ObjectStream SERIALIZE_VERSION.
#[prefab(tag = "azoth.lmbr_central.SplineComponent", version = 1)]
pub struct SplineComponent {
    pub configuration: SplineCommon,
    #[reflect(ignore)]
    #[serde(default, skip)]
    pub spline_shape_asset_id: AssetId,
}

impl Default for SplineComponent {
    fn default() -> Self {
        Self {
            configuration: SplineCommon::default(),
            spline_shape_asset_id: AssetId::nil(),
        }
    }
}
