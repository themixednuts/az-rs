use std::collections::HashMap;

use az_core::crc::Crc32;
use bevy_ecs::resource::Resource;
use bevy_reflect::Reflect;
use serde::{Deserialize, Serialize};

use crate::{CollisionFilter, SurfaceIndex};

/// Solver-independent Cry/RockNRoll surface properties resolved by authored
/// material name, surface type, or native surface index.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Reflect)]
pub struct PhysicsMaterial {
    pub surface_index: SurfaceIndex,
    pub surface_type_crc: Crc32,
    pub pierceability: u8,
    pub friction: f32,
    pub restitution: f32,
    pub density: f32,
    pub traversable: bool,
}

impl Default for PhysicsMaterial {
    fn default() -> Self {
        Self {
            surface_index: SurfaceIndex(0),
            surface_type_crc: Crc32::ZERO,
            pierceability: 0,
            friction: 0.5,
            restitution: 0.0,
            density: 1.0,
            traversable: true,
        }
    }
}

/// Runtime material table matching the native `Physics::MaterialSet` indices.
///
/// The post-load event builds three independent maps: index to
/// material, case-folded name CRC to index, and nonzero case-folded surface
/// type CRC to index. The default material is not assigned an authored index.
#[derive(Resource, Debug, Clone, Default)]
pub struct PhysicsMaterialRegistry {
    default: PhysicsMaterial,
    by_index: HashMap<SurfaceIndex, PhysicsMaterial>,
    name_to_index: HashMap<Crc32, SurfaceIndex>,
    surface_to_index: HashMap<Crc32, SurfaceIndex>,
}

impl PhysicsMaterialRegistry {
    #[must_use]
    pub fn with_default(default: PhysicsMaterial) -> Self {
        Self {
            default,
            by_index: HashMap::new(),
            name_to_index: HashMap::new(),
            surface_to_index: HashMap::new(),
        }
    }

    pub fn insert(&mut self, name: impl AsRef<str>, material: PhysicsMaterial) {
        self.insert_crc(Crc32::from_str_lower(name.as_ref()), material);
    }

    pub fn insert_crc(&mut self, name: Crc32, material: PhysicsMaterial) {
        self.name_to_index.insert(name, material.surface_index);
        if material.surface_type_crc != Crc32::ZERO {
            self.surface_to_index
                .insert(material.surface_type_crc, material.surface_index);
        }
        self.by_index.insert(material.surface_index, material);
    }

    #[must_use]
    pub fn resolve(&self, name: Option<&str>) -> Option<PhysicsMaterial> {
        name.filter(|name| !name.is_empty())
            .map_or(Some(self.default), |name| {
                self.resolve_crc(Crc32::from_str_lower(name))
            })
    }

    #[must_use]
    pub fn resolve_crc(&self, name_or_surface: Crc32) -> Option<PhysicsMaterial> {
        self.resolve_index_crc(name_or_surface)
            .and_then(|index| self.material(index))
    }

    #[must_use]
    pub fn resolve_index(&self, name_or_surface: &str) -> Option<SurfaceIndex> {
        self.resolve_index_crc(Crc32::from_str_lower(name_or_surface))
    }

    #[must_use]
    pub fn resolve_index_crc(&self, name_or_surface: Crc32) -> Option<SurfaceIndex> {
        self.name_to_index
            .get(&name_or_surface)
            .or_else(|| self.surface_to_index.get(&name_or_surface))
            .copied()
    }

    #[must_use]
    pub fn material(&self, index: SurfaceIndex) -> Option<PhysicsMaterial> {
        self.by_index.get(&index).copied()
    }

    #[must_use]
    pub const fn default_material(&self) -> PhysicsMaterial {
        self.default
    }

    pub fn replace(
        &mut self,
        default: PhysicsMaterial,
        materials: impl IntoIterator<Item = (Crc32, PhysicsMaterial)>,
    ) {
        self.default = default;
        self.by_index.clear();
        self.name_to_index.clear();
        self.surface_to_index.clear();
        for (name, material) in materials {
            self.insert_crc(name, material);
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.by_index.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_index.is_empty()
    }
}

/// Immutable compiled `RockNRoll` collision filters resolved by authored name.
#[derive(Resource, Debug, Clone, Default)]
pub struct CollisionFilterRegistry(HashMap<Crc32, CollisionFilter>);

impl CollisionFilterRegistry {
    pub fn insert(&mut self, name: impl AsRef<str>, filter: CollisionFilter) {
        self.insert_crc(Crc32::from_str_lower(name.as_ref()), filter);
    }

    pub fn insert_crc(&mut self, name: Crc32, filter: CollisionFilter) {
        self.0.insert(name, filter);
    }

    #[must_use]
    pub fn resolve(&self, name: &str) -> Option<CollisionFilter> {
        self.resolve_crc(Crc32::from_str_lower(name))
    }

    #[must_use]
    pub fn resolve_crc(&self, name: Crc32) -> Option<CollisionFilter> {
        self.0.get(&name).copied()
    }

    pub fn replace<N>(&mut self, filters: impl IntoIterator<Item = (N, CollisionFilter)>)
    where
        N: AsRef<str>,
    {
        self.0 = filters
            .into_iter()
            .map(|(name, filter)| (Crc32::from_str_lower(name.as_ref()), filter))
            .collect();
    }

    pub fn replace_crc(&mut self, filters: impl IntoIterator<Item = (Crc32, CollisionFilter)>) {
        self.0 = filters.into_iter().collect();
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_tables_match_cry_names_case_insensitively() {
        let mut filters = CollisionFilterRegistry::default();
        let structure = CollisionFilter::default();
        filters.insert("Structure", structure);
        assert_eq!(filters.resolve("structure"), Some(structure));
        assert_eq!(filters.resolve(" structure "), None);

        let mut materials = PhysicsMaterialRegistry::default();
        assert_eq!(materials.resolve(None), Some(PhysicsMaterial::default()));
        assert_eq!(materials.resolve(Some("missing")), None);

        let wood = PhysicsMaterial {
            surface_index: SurfaceIndex(7),
            surface_type_crc: Crc32::from_str_lower("mat_wood"),
            ..PhysicsMaterial::default()
        };
        materials.insert("Wood", wood);
        assert_eq!(materials.resolve(Some("wood")), Some(wood));
        assert_eq!(materials.resolve(Some("mat_WOOD")), Some(wood));
        assert_eq!(materials.resolve(Some(" wood ")), None);
        assert_eq!(materials.material(SurfaceIndex(7)), Some(wood));
    }
}
