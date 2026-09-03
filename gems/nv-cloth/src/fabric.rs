use std::sync::Arc;

use az_core::AssetId;
use az_nv_cloth::{ClothFabricAsset, FabricPhaseType};
use bevy::prelude::Resource;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DistanceConstraint {
    pub particles: [u32; 2],
    pub rest_length: f32,
    pub stiffness: f32,
    pub phase_type: FabricPhaseType,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TetherConstraint {
    pub particle: u32,
    pub anchor: u32,
    pub length: f32,
}

#[derive(Debug)]
pub struct SharedFabric {
    pub constraints: Box<[DistanceConstraint]>,
    pub tethers: Box<[TetherConstraint]>,
    pub triangles: Box<[[u32; 3]]>,
    self_collision_exclusions: Box<[u64]>,
}

impl SharedFabric {
    #[must_use]
    pub fn from_asset(asset: &ClothFabricAsset) -> Self {
        let fabric = asset.fabric();
        let cooked = &fabric.cooked;
        let particle_count = fabric.mesh.vertices.len();
        let mut constraints = Vec::with_capacity(cooked.rest_values.len());
        for (phase_slot, (&set_index, &phase_type)) in cooked
            .phase_indices
            .iter()
            .zip(&cooked.phase_types)
            .enumerate()
        {
            let set_index = set_index as usize;
            let first_constraint = set_index
                .checked_sub(1)
                .and_then(|index| cooked.sets.get(index))
                .copied()
                .unwrap_or(0);
            let start = first_constraint as usize;
            let end = cooked
                .sets
                .get(set_index)
                .copied()
                .unwrap_or(first_constraint) as usize;
            for constraint in start..end.min(cooked.rest_values.len()) {
                let index = constraint * 2;
                let Some(particles) = cooked.constraint_indices.get(index..index + 2) else {
                    break;
                };
                constraints.push(DistanceConstraint {
                    particles: [particles[0], particles[1]],
                    rest_length: cooked.rest_values[constraint],
                    stiffness: cooked
                        .stiffness_values
                        .get(constraint)
                        .copied()
                        .unwrap_or(1.0),
                    phase_type,
                });
            }
            debug_assert!(phase_slot < cooked.phase_types.len());
        }

        let tethers = cooked
            .anchors
            .iter()
            .copied()
            .zip(cooked.tether_lengths.iter().copied())
            .enumerate()
            .map(|(tether, (anchor, length))| TetherConstraint {
                // A particle index is a `u32` in the fabric format, so the
                // remainder is in range for every fabric the loader accepted.
                particle: u32::try_from(tether % particle_count).unwrap_or(u32::MAX),
                anchor,
                length,
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let triangles = cooked
            .triangles
            .chunks_exact(3)
            .map(|triangle| [triangle[0], triangle[1], triangle[2]])
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let mut self_collision_exclusions = Vec::with_capacity(
            constraints
                .len()
                .saturating_add(triangles.len().saturating_mul(3)),
        );
        self_collision_exclusions.extend(
            constraints.iter().map(|constraint| {
                particle_pair_key(constraint.particles[0], constraint.particles[1])
            }),
        );
        for triangle in &triangles {
            self_collision_exclusions.extend([
                particle_pair_key(triangle[0], triangle[1]),
                particle_pair_key(triangle[1], triangle[2]),
                particle_pair_key(triangle[2], triangle[0]),
            ]);
        }
        self_collision_exclusions.sort_unstable();
        self_collision_exclusions.dedup();
        Self {
            constraints: constraints.into_boxed_slice(),
            tethers,
            triangles,
            self_collision_exclusions: self_collision_exclusions.into_boxed_slice(),
        }
    }

    #[must_use]
    pub fn excludes_self_collision(&self, left: u32, right: u32) -> bool {
        self.self_collision_exclusions
            .binary_search(&particle_pair_key(left, right))
            .is_ok()
    }
}

const fn particle_pair_key(left: u32, right: u32) -> u64 {
    let (lower, upper) = if left <= right {
        (left, right)
    } else {
        (right, left)
    };
    (lower as u64) << 32 | upper as u64
}

#[derive(Resource, Default)]
pub struct SharedFabricCache {
    fabrics: std::collections::HashMap<AssetId, Arc<SharedFabric>>,
}

impl SharedFabricCache {
    pub fn get_or_insert(
        &mut self,
        asset_id: AssetId,
        asset: &ClothFabricAsset,
    ) -> Arc<SharedFabric> {
        self.fabrics
            .entry(asset_id)
            .or_insert_with(|| Arc::new(SharedFabric::from_asset(asset)))
            .clone()
    }

    pub fn remove(&mut self, asset_id: AssetId) {
        self.fabrics.remove(&asset_id);
    }
}
