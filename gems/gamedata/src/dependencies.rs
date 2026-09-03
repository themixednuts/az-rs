//! Build-time aggregation of per-table dependency sections.
//!
//! Runtime providers use `assetcatalog.bin` as the inventory and validate table
//! headers directly; this module is for transformer diagnostics and dependency
//! parity checks.

use std::collections::{HashMap, HashSet};
use std::hash::BuildHasher;

use az_asset::AssetId;

use crate::GameDataError;
use crate::release::{GameDataDependency, GameDataDependencyKind, SchemaHash};
use crate::table::{DEPENDENCY_KIND_FOREIGN_KEY, TableDependency};

/// Fold per-table `DependencyIndex` payloads into deduplicated dependency edges.
///
/// # Errors
///
/// Returns any error [`collect_table_dependencies_with_schema`] returns; with
/// no schema list supplied that is only the unknown-target-table case.
pub fn collect_table_dependencies<S: BuildHasher>(
    name_crc_to_asset: &HashMap<u32, AssetId, S>,
    tables: &[(AssetId, Vec<TableDependency>)],
) -> Result<Vec<GameDataDependency>, GameDataError> {
    collect_table_dependencies_with_schema(
        name_crc_to_asset,
        &HashMap::<u32, SchemaHash>::new(),
        tables,
    )
}

/// Like [`collect_table_dependencies`] but also verifies each edge's
/// `target_schema_hash` against the generated table schema list when provided.
///
/// # Errors
///
/// Returns [`GameDataError::Decode`] when a foreign-key edge names a target
/// table CRC that `name_crc_to_asset` does not resolve, or
/// [`GameDataError::SchemaMismatch`] when `schema_hash_by_name_crc` records a
/// different schema hash for that target than the edge carries.
pub fn collect_table_dependencies_with_schema<S, T>(
    name_crc_to_asset: &HashMap<u32, AssetId, S>,
    schema_hash_by_name_crc: &HashMap<u32, SchemaHash, T>,
    tables: &[(AssetId, Vec<TableDependency>)],
) -> Result<Vec<GameDataDependency>, GameDataError>
where
    S: BuildHasher,
    T: BuildHasher,
{
    let mut edges = HashSet::new();
    for (from, dependencies) in tables {
        for dependency in dependencies {
            if dependency.kind != DEPENDENCY_KIND_FOREIGN_KEY {
                continue;
            }
            let Some(to) = name_crc_to_asset.get(&dependency.target_table_name_crc) else {
                return Err(GameDataError::Decode(format!(
                    "dependency from asset {from} references unknown target table name_crc 0x{:08x}",
                    dependency.target_table_name_crc
                )));
            };
            if let Some(expected) = schema_hash_by_name_crc.get(&dependency.target_table_name_crc)
                && *expected != dependency.target_schema_hash
            {
                return Err(GameDataError::SchemaMismatch {
                    asset_id: *to,
                    expected: *expected,
                    actual: dependency.target_schema_hash,
                });
            }
            edges.insert((*from, *to));
        }
    }
    let mut edges = edges
        .into_iter()
        .map(|(from, to)| GameDataDependency {
            from,
            to,
            kind: GameDataDependencyKind::ForeignKey,
        })
        .collect::<Vec<_>>();
    edges.sort_by_key(|edge| (edge.from, edge.to));
    Ok(edges)
}

/// `(from_asset, to_asset)` keys for comparing table dependency graphs.
#[must_use]
pub fn dependency_edge_set(deps: &[GameDataDependency]) -> HashSet<(AssetId, AssetId)> {
    deps.iter().map(|dep| (dep.from, dep.to)).collect()
}

/// Build/import audit: table-asset `DependencyIndex` sections must agree with
/// schema inference. Fails the transform when table assets and schemas drift.
///
/// # Errors
///
/// Returns [`GameDataError::Decode`] when the two edge sets differ, naming
/// both counts and up to three example edges unique to each side.
pub fn verify_dependency_parity(
    from_tables: &[GameDataDependency],
    from_schema: &[GameDataDependency],
) -> Result<(), GameDataError> {
    let tables = dependency_edge_set(from_tables);
    let schema = dependency_edge_set(from_schema);
    if tables == schema {
        return Ok(());
    }
    let only_tables: Vec<_> = tables.difference(&schema).take(3).copied().collect();
    let only_schema: Vec<_> = schema.difference(&tables).take(3).copied().collect();
    Err(GameDataError::Decode(format!(
        "table dependency parity failed: {} table-asset edges vs {} schema-inferred edges \
         (table-asset-only examples: {only_tables:?}; schema-only examples: {only_schema:?})",
        tables.len(),
        schema.len()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn asset_id(seed: u8) -> AssetId {
        AssetId::new(Uuid::from_u128(u128::from(seed)), 0)
    }

    #[test]
    fn collect_deduplicates_parallel_edges() {
        let from = asset_id(1);
        let to = asset_id(2);
        let name_crc_to_asset = HashMap::from([(0xaaa_bbbb, to)]);
        let dependency = TableDependency {
            column_crc: 10,
            target_table_name_crc: 0xaaa_bbbb,
            target_schema_hash: SchemaHash(1),
            kind: DEPENDENCY_KIND_FOREIGN_KEY,
        };
        let tables = vec![
            (from, vec![dependency, dependency]),
            (from, vec![dependency]),
        ];
        let edges = collect_table_dependencies(&name_crc_to_asset, &tables).expect("collect edges");
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].from, from);
        assert_eq!(edges[0].to, to);
    }

    #[test]
    fn verify_dependency_parity_rejects_drift() {
        let from = asset_id(1);
        let to = asset_id(2);
        let other = asset_id(3);
        let tables = vec![GameDataDependency {
            from,
            to,
            kind: GameDataDependencyKind::ForeignKey,
        }];
        let schema = vec![GameDataDependency {
            from,
            to: other,
            kind: GameDataDependencyKind::ForeignKey,
        }];
        let err = verify_dependency_parity(&tables, &schema).expect_err("drift");
        assert!(err.to_string().contains("parity failed"), "{err}");
    }
}
