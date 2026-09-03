//! Pure `GameData` manager dependency resolution.
//!
//! The graph resolver is intentionally catalog-local and IO-free. Project-host
//! gathers table, family, and explicit manager descriptors for one workspace, then
//! this module resolves provider dependencies and validates topology before the
//! asset processor plans work.

use std::collections::{BTreeMap, BTreeSet};

use crate::manager::{
    GameDataManagerInput, GameDataManagerShape, GameDataManagerShapeError, ProviderTarget,
    TableFamilyDescriptor, TableInputDescriptor,
};

/// Stable resolved manager node identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ManagerNodeId {
    /// Explicit manager that does not provide a table/family target.
    Explicit(&'static str),
    /// Provider node for a table/family target.
    Provider(ProviderTarget),
}

impl ManagerNodeId {
    /// User-facing node label for diagnostics.
    #[inline]
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Explicit(name) => name,
            Self::Provider(target) => target.name(),
        }
    }
}

/// Resolved manager implementation kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagerNode {
    /// Authored explicit manager descriptor.
    Explicit(GameDataManagerShape),
    /// Synthesized read-only projection for a table or family.
    DefaultProvider(ProviderTarget),
}

/// One resolved manager node and its manager-to-manager dependencies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedManagerNode {
    id: ManagerNodeId,
    node: ManagerNode,
    dependencies: Vec<ManagerNodeId>,
}

impl ResolvedManagerNode {
    #[inline]
    #[must_use]
    pub const fn id(&self) -> ManagerNodeId {
        self.id
    }

    #[inline]
    #[must_use]
    pub const fn node(&self) -> ManagerNode {
        self.node
    }

    #[inline]
    #[must_use]
    pub fn dependencies(&self) -> &[ManagerNodeId] {
        &self.dependencies
    }
}

/// Resolved manager dependency graph in dependency-first order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedManagerGraph {
    nodes: Vec<ResolvedManagerNode>,
    topological_order: Vec<ManagerNodeId>,
}

impl ResolvedManagerGraph {
    #[inline]
    #[must_use]
    pub fn nodes(&self) -> &[ResolvedManagerNode] {
        &self.nodes
    }

    #[inline]
    #[must_use]
    pub fn topological_order(&self) -> &[ManagerNodeId] {
        &self.topological_order
    }

    #[must_use]
    pub fn node(&self, id: ManagerNodeId) -> Option<&ResolvedManagerNode> {
        self.nodes.iter().find(|node| node.id == id)
    }
}

/// Manager graph resolution failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ManagerGraphError {
    /// A manager shape failed local descriptor validation.
    #[error("GameData manager `{manager}` descriptor is invalid: {source}")]
    InvalidShape {
        manager: &'static str,
        source: GameDataManagerShapeError,
    },
    /// Two explicit manager shapes share one manager name.
    #[error("GameData manager `{manager}` is declared more than once")]
    DuplicateManagerName { manager: &'static str },
    /// Two explicit managers claim the same table/family provider target.
    #[error("GameData provider target `{target}` is claimed by both `{first}` and `{second}`")]
    DuplicateProviderClaim {
        target: &'static str,
        first: &'static str,
        second: &'static str,
    },
    /// A named manager dependency did not resolve to an explicit manager.
    #[error("GameData manager `{from}` depends on unknown manager `{manager}`")]
    UnknownNamedManager {
        from: &'static str,
        manager: &'static str,
    },
    /// A provider dependency targets a table/family that is not in the catalog.
    #[error("GameData manager `{from}` depends on unknown provider target `{target}`")]
    DanglingProviderTarget {
        from: &'static str,
        target: &'static str,
    },
    /// An explicit manager claims a provider target that is not in the catalog.
    #[error("GameData manager `{manager}` claims unknown provider target `{target}`")]
    DanglingProviderClaim {
        manager: &'static str,
        target: &'static str,
    },
    /// Manager dependencies contain a cycle.
    #[error("GameData manager dependency cycle: {members:?}")]
    Cycle { members: Vec<ManagerNodeId> },
}

/// Resolves explicit manager shapes and synthesized provider fallbacks into a
/// dependency graph.
///
/// Provider dependencies target data provisions, not concrete manager names. If
/// no explicit shape claims a provider target, the resolver synthesizes a
/// read-only default provider node for that target.
///
/// # Errors
///
/// Returns graph diagnostics for invalid shapes, duplicate names, duplicate
/// explicit provider claims, unknown named dependencies, dangling provider
/// targets, or dependency cycles.
pub fn build_manager_graph(
    shapes: &[GameDataManagerShape],
    tables: &[TableInputDescriptor],
    families: &[TableFamilyDescriptor],
) -> Result<ResolvedManagerGraph, ManagerGraphError> {
    let known_targets = known_provider_targets(tables, families);
    let mut manager_ids_by_name = BTreeMap::new();
    let mut provider_claims = BTreeMap::new();
    let mut nodes = Vec::new();
    let mut indexes_by_id = BTreeMap::new();

    for shape in shapes {
        shape
            .validate()
            .map_err(|source| ManagerGraphError::InvalidShape {
                manager: shape.name(),
                source,
            })?;
        let id = shape_node_id(shape);
        if manager_ids_by_name.insert(shape.name(), id).is_some() {
            return Err(ManagerGraphError::DuplicateManagerName {
                manager: shape.name(),
            });
        }
        if let Some(target) = shape.provides() {
            if !known_targets.contains(&target) {
                return Err(ManagerGraphError::DanglingProviderClaim {
                    manager: shape.name(),
                    target: target.name(),
                });
            }
            if let Some(first) = provider_claims.insert(target, shape.name()) {
                return Err(ManagerGraphError::DuplicateProviderClaim {
                    target: target.name(),
                    first,
                    second: shape.name(),
                });
            }
        }
        insert_node(
            &mut nodes,
            &mut indexes_by_id,
            ResolvedManagerNode {
                id,
                node: ManagerNode::Explicit(*shape),
                dependencies: Vec::new(),
            },
        );
    }

    for shape in shapes {
        let from_id = shape_node_id(shape);
        let from_index = indexes_by_id[&from_id];
        let mut dependencies = BTreeSet::new();

        for input in shape.inputs() {
            match *input {
                GameDataManagerInput::Manager(manager) => {
                    let Some(id) = manager_ids_by_name.get(manager.manager()).copied() else {
                        return Err(ManagerGraphError::UnknownNamedManager {
                            from: shape.name(),
                            manager: manager.manager(),
                        });
                    };
                    dependencies.insert(id);
                }
                GameDataManagerInput::Provider(provider) => {
                    let target = provider.target();
                    if !known_targets.contains(&target) {
                        return Err(ManagerGraphError::DanglingProviderTarget {
                            from: shape.name(),
                            target: target.name(),
                        });
                    }
                    let id = ManagerNodeId::Provider(target);
                    if !indexes_by_id.contains_key(&id) {
                        insert_node(
                            &mut nodes,
                            &mut indexes_by_id,
                            ResolvedManagerNode {
                                id,
                                node: ManagerNode::DefaultProvider(target),
                                dependencies: Vec::new(),
                            },
                        );
                    }
                    dependencies.insert(id);
                }
                GameDataManagerInput::Table(_)
                | GameDataManagerInput::TableFamily(_)
                | GameDataManagerInput::Product(_)
                | GameDataManagerInput::SourceSchema(_) => {}
            }
        }

        nodes[from_index].dependencies = dependencies.into_iter().collect();
    }

    let topological_order = topological_order(&nodes, &indexes_by_id)?;
    Ok(ResolvedManagerGraph {
        nodes,
        topological_order,
    })
}

fn known_provider_targets(
    tables: &[TableInputDescriptor],
    families: &[TableFamilyDescriptor],
) -> BTreeSet<ProviderTarget> {
    let mut targets = BTreeSet::new();
    targets.extend(tables.iter().copied().map(ProviderTarget::table));
    targets.extend(families.iter().copied().map(ProviderTarget::family));
    targets
}

fn shape_node_id(shape: &GameDataManagerShape) -> ManagerNodeId {
    shape.provides().map_or_else(
        || ManagerNodeId::Explicit(shape.name()),
        ManagerNodeId::Provider,
    )
}

fn insert_node(
    nodes: &mut Vec<ResolvedManagerNode>,
    indexes_by_id: &mut BTreeMap<ManagerNodeId, usize>,
    node: ResolvedManagerNode,
) {
    indexes_by_id.insert(node.id, nodes.len());
    nodes.push(node);
}

fn topological_order(
    nodes: &[ResolvedManagerNode],
    indexes_by_id: &BTreeMap<ManagerNodeId, usize>,
) -> Result<Vec<ManagerNodeId>, ManagerGraphError> {
    let mut state = vec![VisitState::Unvisited; nodes.len()];
    let mut stack = Vec::new();
    let mut order = Vec::with_capacity(nodes.len());

    for index in 0..nodes.len() {
        visit(
            index,
            nodes,
            indexes_by_id,
            &mut state,
            &mut stack,
            &mut order,
        )?;
    }

    Ok(order)
}

fn visit(
    index: usize,
    nodes: &[ResolvedManagerNode],
    indexes_by_id: &BTreeMap<ManagerNodeId, usize>,
    state: &mut [VisitState],
    stack: &mut Vec<ManagerNodeId>,
    order: &mut Vec<ManagerNodeId>,
) -> Result<(), ManagerGraphError> {
    match state[index] {
        VisitState::Done => return Ok(()),
        VisitState::Visiting => {
            return Err(ManagerGraphError::Cycle {
                members: stack.clone(),
            });
        }
        VisitState::Unvisited => {}
    }

    state[index] = VisitState::Visiting;
    stack.push(nodes[index].id);
    for dependency in &nodes[index].dependencies {
        let dependency_index = indexes_by_id[dependency];
        if state[dependency_index] == VisitState::Visiting {
            let cycle_start = stack
                .iter()
                .position(|member| member == dependency)
                .unwrap_or(0);
            let mut members = stack[cycle_start..].to_vec();
            members.push(*dependency);
            return Err(ManagerGraphError::Cycle { members });
        }
        visit(dependency_index, nodes, indexes_by_id, state, stack, order)?;
    }
    stack.pop();
    state[index] = VisitState::Done;
    order.push(nodes[index].id);
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VisitState {
    Unvisited,
    Visiting,
    Done,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::{
        DuplicateKeyPolicy, ManagerDependencyDescriptor, ManagerShapeKind,
        ProviderDependencyDescriptor,
    };

    const ITEM_TABLE: TableInputDescriptor = TableInputDescriptor::new("ItemTable", "ItemRow");
    const NPC_TABLE: TableInputDescriptor = TableInputDescriptor::new("NpcTable", "NpcRow");
    const TABLES: &[TableInputDescriptor] = &[ITEM_TABLE, NPC_TABLE];
    const ITEM_TARGET: ProviderTarget = ProviderTarget::table(ITEM_TABLE);
    const NPC_TARGET: ProviderTarget = ProviderTarget::table(NPC_TABLE);
    const EMPTY_FAMILIES: &[TableFamilyDescriptor] = &[];

    const ITEM_PROVIDER_INPUTS: &[GameDataManagerInput] =
        &[GameDataManagerInput::Table(ITEM_TABLE)];
    const ITEM_PROVIDER: GameDataManagerShape = GameDataManagerShape::new(
        "ItemDataManager",
        ManagerShapeKind::SingleTableIndex,
        ITEM_PROVIDER_INPUTS,
    )
    .with_provides(ITEM_TARGET)
    .with_duplicate_key_policy(DuplicateKeyPolicy::FirstWins);

    const DEPENDS_ON_ITEM_PROVIDER: &[GameDataManagerInput] = &[GameDataManagerInput::Provider(
        ProviderDependencyDescriptor::new(ITEM_TARGET),
    )];
    const DEPENDENT: GameDataManagerShape = GameDataManagerShape::new(
        "DependentManager",
        ManagerShapeKind::ComposedResource,
        DEPENDS_ON_ITEM_PROVIDER,
    );

    #[test]
    fn provider_dependency_synthesizes_default_provider() {
        let graph = build_manager_graph(&[DEPENDENT], TABLES, EMPTY_FAMILIES)
            .expect("manager graph should resolve");
        let provider_id = ManagerNodeId::Provider(ITEM_TARGET);
        let dependent_id = ManagerNodeId::Explicit("DependentManager");

        let provider = graph.node(provider_id).expect("default provider");
        assert_eq!(provider.node(), ManagerNode::DefaultProvider(ITEM_TARGET));
        assert_eq!(
            graph.node(dependent_id).unwrap().dependencies(),
            &[provider_id]
        );
        assert_order_before(&graph, provider_id, dependent_id);
    }

    #[test]
    fn explicit_provider_overrides_synthesized_provider_without_changing_edges() {
        let auto_graph = build_manager_graph(&[DEPENDENT], TABLES, EMPTY_FAMILIES)
            .expect("auto graph should resolve");
        let explicit_graph =
            build_manager_graph(&[ITEM_PROVIDER, DEPENDENT], TABLES, EMPTY_FAMILIES)
                .expect("explicit graph should resolve");
        let provider_id = ManagerNodeId::Provider(ITEM_TARGET);
        let dependent_id = ManagerNodeId::Explicit("DependentManager");

        assert_eq!(
            auto_graph.node(dependent_id).unwrap().dependencies(),
            explicit_graph.node(dependent_id).unwrap().dependencies()
        );
        assert!(matches!(
            explicit_graph.node(provider_id).unwrap().node(),
            ManagerNode::Explicit(shape) if shape.name() == "ItemDataManager"
        ));
        assert_order_before(&explicit_graph, provider_id, dependent_id);
    }

    #[test]
    fn dangling_provider_target_is_rejected() {
        const MISSING_TARGET: ProviderTarget = ProviderTarget::Table {
            table_name: "MissingTable",
            row_name: "MissingRow",
        };
        const INPUTS: &[GameDataManagerInput] = &[GameDataManagerInput::Provider(
            ProviderDependencyDescriptor::new(MISSING_TARGET),
        )];
        const SHAPE: GameDataManagerShape =
            GameDataManagerShape::new("Broken", ManagerShapeKind::ComposedResource, INPUTS);

        assert!(matches!(
            build_manager_graph(&[SHAPE], TABLES, EMPTY_FAMILIES),
            Err(ManagerGraphError::DanglingProviderTarget {
                from: "Broken",
                target: "MissingTable",
            })
        ));
    }

    #[test]
    fn unknown_named_manager_is_rejected() {
        const INPUTS: &[GameDataManagerInput] = &[GameDataManagerInput::Manager(
            ManagerDependencyDescriptor::new("MissingManager"),
        )];
        const SHAPE: GameDataManagerShape =
            GameDataManagerShape::new("Broken", ManagerShapeKind::ComposedResource, INPUTS);

        assert!(matches!(
            build_manager_graph(&[SHAPE], TABLES, EMPTY_FAMILIES),
            Err(ManagerGraphError::UnknownNamedManager {
                from: "Broken",
                manager: "MissingManager",
            })
        ));
    }

    #[test]
    fn duplicate_explicit_provider_claim_is_rejected() {
        const OTHER_INPUTS: &[GameDataManagerInput] = &[GameDataManagerInput::Table(ITEM_TABLE)];
        const OTHER_PROVIDER: GameDataManagerShape = GameDataManagerShape::new(
            "OtherItemManager",
            ManagerShapeKind::SingleTableIndex,
            OTHER_INPUTS,
        )
        .with_provides(ITEM_TARGET);

        assert!(matches!(
            build_manager_graph(&[ITEM_PROVIDER, OTHER_PROVIDER], TABLES, EMPTY_FAMILIES),
            Err(ManagerGraphError::DuplicateProviderClaim {
                target: "ItemTable",
                first: "ItemDataManager",
                second: "OtherItemManager",
            })
        ));
    }

    #[test]
    fn explicit_provider_claim_must_target_known_catalog_data() {
        const MISSING_TARGET: ProviderTarget = ProviderTarget::Table {
            table_name: "MissingTable",
            row_name: "MissingRow",
        };
        const INPUTS: &[GameDataManagerInput] = &[GameDataManagerInput::Table(ITEM_TABLE)];
        const SHAPE: GameDataManagerShape =
            GameDataManagerShape::new("BrokenProvider", ManagerShapeKind::SingleTableIndex, INPUTS)
                .with_provides(MISSING_TARGET);

        assert!(matches!(
            build_manager_graph(&[SHAPE], TABLES, EMPTY_FAMILIES),
            Err(ManagerGraphError::DanglingProviderClaim {
                manager: "BrokenProvider",
                target: "MissingTable",
            })
        ));
    }

    #[test]
    fn cycle_across_named_and_provider_edges_is_rejected() {
        const A_INPUTS: &[GameDataManagerInput] = &[GameDataManagerInput::Provider(
            ProviderDependencyDescriptor::new(NPC_TARGET),
        )];
        const A: GameDataManagerShape =
            GameDataManagerShape::new("A", ManagerShapeKind::ComposedResource, A_INPUTS);
        const B_INPUTS: &[GameDataManagerInput] = &[GameDataManagerInput::Manager(
            ManagerDependencyDescriptor::new("A"),
        )];
        const B: GameDataManagerShape =
            GameDataManagerShape::new("B", ManagerShapeKind::ComposedResource, B_INPUTS)
                .with_provides(NPC_TARGET);

        assert!(matches!(
            build_manager_graph(&[A, B], TABLES, EMPTY_FAMILIES),
            Err(ManagerGraphError::Cycle { .. })
        ));
    }

    fn assert_order_before(
        graph: &ResolvedManagerGraph,
        before: ManagerNodeId,
        after: ManagerNodeId,
    ) {
        let before_index = graph
            .topological_order()
            .iter()
            .position(|id| *id == before)
            .expect("before node should be ordered");
        let after_index = graph
            .topological_order()
            .iter()
            .position(|id| *id == after)
            .expect("after node should be ordered");
        assert!(
            before_index < after_index,
            "{before:?} should appear before {after:?}"
        );
    }
}
