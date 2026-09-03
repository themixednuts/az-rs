//! Phase 6 boolean and short-circuit reconstruction.

use std::collections::HashMap;

use crate::{
    decompile::{
        analysis::{DecompileAnalysis, NodeId, ValueId},
        control_flow::{
            conditionals::{self, BranchInfo},
            loops::LoopAnalysis,
            regions::BlockSet,
        },
    },
    ir::{SsaFunction, SsaLiteral, SsaNode, SsaOp, SsaRef},
};

pub mod normalize;
mod short_circuit;
mod value_chain;

pub use short_circuit::{
    BoolConnector, ConditionChain, ConditionSegment, ValuePlan, ValuePlanKind, ValueTerm,
};

/// Boolean reconstruction facts computed once for a function.
#[derive(Debug, Clone, Default)]
pub struct BooleanAnalysis {
    components: Vec<ControlComponent>,
    component_by_start: HashMap<usize, usize>,
    value_by_phi: HashMap<ValueId, usize>,
}

/// One control-dependent component and its source-level consumer capability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlComponent {
    Condition(ConditionChain),
    Value(ValuePlan),
}

/// Shared immutable capabilities for condition-chain recovery.
pub(super) struct ConditionContext<'a> {
    function: &'a SsaFunction,
    expr_analysis: &'a DecompileAnalysis,
    pc_map: &'a [Option<usize>],
    loop_headers: &'a BlockSet,
    value_starts: &'a BlockSet,
}

impl ConditionContext<'_> {
    fn is_condition_block(&self, block: usize) -> bool {
        !self.loop_headers.contains(block)
            && conditionals::is_pure_condition_block(self.function, block)
            && !has_unrelated_side_effect_before_branch(self.function, block)
            && !has_prefix_def_used_after_branch(self.function, self.expr_analysis, block)
            && !self.value_starts.contains(block)
    }
}

impl ControlComponent {
    #[must_use]
    pub const fn start(&self) -> usize {
        match self {
            Self::Condition(chain) => chain.start,
            Self::Value(plan) => plan.start,
        }
    }

    #[must_use]
    pub const fn merge(&self) -> usize {
        match self {
            Self::Condition(chain) => chain.merge,
            Self::Value(plan) => plan.merge,
        }
    }

    #[must_use]
    pub fn owned_blocks(&self) -> Vec<usize> {
        match self {
            Self::Condition(chain) => chain.blocks.clone(),
            Self::Value(plan) => plan.consumed_blocks().collect(),
        }
    }

    #[must_use]
    pub const fn condition(&self) -> Option<&ConditionChain> {
        match self {
            Self::Condition(chain) => Some(chain),
            Self::Value(_) => None,
        }
    }

    #[must_use]
    pub const fn value(&self) -> Option<&ValuePlan> {
        match self {
            Self::Value(plan) => Some(plan),
            Self::Condition(_) => None,
        }
    }
}

impl BooleanAnalysis {
    /// Empty analysis for branch-free or Phase 4-only callers.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    fn from_components(components: Vec<ControlComponent>) -> Self {
        let mut analysis = Self {
            components,
            ..Self::default()
        };
        for (index, component) in analysis.components.iter().enumerate() {
            analysis
                .component_by_start
                .entry(component.start())
                .or_insert(index);
            if let Some(value) = component
                .value()
                .and_then(|plan| ValueId::from_ref(plan.dest))
            {
                analysis.value_by_phi.entry(value).or_insert(index);
            }
        }
        analysis
    }

    /// Return a compound condition chain starting at `block`.
    #[must_use]
    pub fn condition_chain(&self, block: usize) -> Option<&ConditionChain> {
        self.component_start(block)?.condition()
    }

    /// Return the control component beginning at `block`.
    #[must_use]
    pub fn component_start(&self, block: usize) -> Option<&ControlComponent> {
        self.component_by_start
            .get(&block)
            .and_then(|index| self.components.get(*index))
    }

    /// Return a value select plan starting at `block`.
    #[must_use]
    pub fn value_select_start(&self, block: usize) -> Option<&ValuePlan> {
        self.component_start(block)?.value()
    }

    /// Return a value select plan whose internal materialization covers `block`.
    #[must_use]
    pub fn value_select_covering(&self, block: usize) -> Option<&ValuePlan> {
        self.components
            .iter()
            .filter_map(ControlComponent::value)
            .find(|plan| plan.start < block && block < plan.merge)
    }

    /// Return the value select plan that materializes `reference`.
    #[must_use]
    pub fn value_for_phi(&self, reference: SsaRef) -> Option<&ValuePlan> {
        let value = ValueId::from_ref(reference)?;
        self.value_by_phi
            .get(&value)
            .and_then(|index| self.components.get(*index))?
            .value()
    }

    /// Blocks whose setup definitions are owned by a reconstructed boolean expression.
    #[must_use]
    pub fn expression_blocks(&self) -> Vec<usize> {
        let mut blocks = self
            .components
            .iter()
            .flat_map(ControlComponent::owned_blocks)
            .collect::<Vec<_>>();
        blocks.sort_unstable();
        blocks.dedup();
        blocks
    }
}

/// Compute all Phase 6 boolean reconstruction facts for `function`.
#[must_use]
pub fn analyze(
    function: &SsaFunction,
    expr_analysis: &DecompileAnalysis,
    loops: &LoopAnalysis,
    pc_map: &[Option<usize>],
) -> BooleanAnalysis {
    let loop_headers = loops.loop_headers(function.blocks.len());
    let value_plans = (0..function.blocks.len())
        .map(|block| {
            short_circuit::value_plan(function, expr_analysis, block, pc_map)
                .filter(|plan| value_plan_owns_internal_definitions(function, expr_analysis, plan))
        })
        .collect::<Vec<_>>();
    let mut value_starts = BlockSet::new(function.blocks.len());
    for plan in value_plans.iter().flatten() {
        value_starts.insert(plan.start);
    }
    let condition_context = ConditionContext {
        function,
        expr_analysis,
        pc_map,
        loop_headers: &loop_headers,
        value_starts: &value_starts,
    };
    let mut consumed_values = BlockSet::new(function.blocks.len());
    let mut components = Vec::new();

    for (block, value_plan) in value_plans.iter().enumerate() {
        if consumed_values.contains(block) {
            continue;
        }

        if let Some(plan) = value_plan.clone() {
            for consumed in plan.consumed_blocks() {
                consumed_values.insert(consumed);
            }
            components.push(ControlComponent::Value(plan));
            continue;
        }

        if let Some(chain) = short_circuit::condition_chain(&condition_context, block) {
            components.push(ControlComponent::Condition(chain));
        }
    }

    BooleanAnalysis::from_components(compose_control_components(
        function,
        expr_analysis,
        components,
    ))
}

/// A value expression may consume its internal control blocks only when every
/// definition in those blocks belongs exclusively to that expression.
///
/// Lua frequently lowers one conditional source block into updates of several
/// registers. Collapsing only one merge PHI into `and`/`or` while consuming the
/// whole block would silently discard the sibling assignments. A definition is
/// owned when all of its uses remain inside the value range or feed the value
/// plan's own merge PHI.
fn value_plan_owns_internal_definitions(
    function: &SsaFunction,
    expr_analysis: &DecompileAnalysis,
    plan: &ValuePlan,
) -> bool {
    let phis = conditionals::phi_sources(function, plan.merge);
    let Some(value_phi) = phis.iter().find(|phi| phi.dest == plan.dest) else {
        return false;
    };
    if value_phi.sources.iter().any(|(_, source)| {
        value_depends_on_secondary_result(function, expr_analysis, *source, &mut Vec::new())
    }) {
        return false;
    }

    let sibling_phis_are_unchanged =
        phis.into_iter()
            .filter(|phi| phi.dest != plan.dest)
            .all(|phi| {
                phi.sources
                    .first()
                    .is_none_or(|(_, first)| phi.sources.iter().all(|(_, value)| value == first))
            });
    if !sibling_phis_are_unchanged {
        return false;
    }

    (plan.start.saturating_add(1).min(plan.merge)..plan.merge).all(|block| {
        function.blocks.get(block).is_some_and(|block_ref| {
            block_ref.nodes.iter().enumerate().all(|(node, _)| {
                expr_analysis
                    .defs_at(NodeId { block, node })
                    .iter()
                    .copied()
                    .all(|reference| {
                        let facts = expr_analysis.facts(reference);
                        facts.upvalue_captures == 0
                            && expr_analysis.real_uses(reference).iter().all(|use_id| {
                                if plan.consumed_blocks().contains(&use_id.block) {
                                    return true;
                                }
                                use_id.block == plan.merge
                                    && expr_analysis.node(function, *use_id).is_some_and(
                                        |use_node| {
                                            matches!(use_node.op, SsaOp::Phi { .. })
                                                && use_node.dest == plan.dest
                                        },
                                    )
                            })
                    })
            })
        })
    })
}

/// A scalar `and`/`or` expression can carry the primary result of a call, but
/// it cannot select a secondary result from a fixed multi-return assignment.
/// Follow transparent moves so that `_ , value = call()` remains structural
/// even when the bytecode copies `value` before the merge PHI.
fn value_depends_on_secondary_result(
    function: &SsaFunction,
    expr_analysis: &DecompileAnalysis,
    reference: SsaRef,
    visited: &mut Vec<NodeId>,
) -> bool {
    let Some(id) = expr_analysis.def_site(reference) else {
        return false;
    };
    if visited.contains(&id) {
        return false;
    }
    visited.push(id);

    let Some(node) = expr_analysis.node(function, id) else {
        return false;
    };
    if node.dest != reference && expr_analysis.defs_at(id).len() > 1 {
        return true;
    }
    match &node.op {
        SsaOp::Move { src } => {
            value_depends_on_secondary_result(function, expr_analysis, *src, visited)
        }
        _ => false,
    }
}

/// Compose a condition prefix and its nested value select when both feed the
/// same PHI. The proof deliberately lives in analysis so region assembly and
/// expression lowering consume one typed owner instead of rediscovering CFG
/// relationships independently.
fn compose_control_components(
    function: &SsaFunction,
    expr_analysis: &DecompileAnalysis,
    mut components: Vec<ControlComponent>,
) -> Vec<ControlComponent> {
    let mut removed = vec![false; components.len()];

    for value_index in 0..components.len() {
        let ControlComponent::Value(value) = &components[value_index] else {
            continue;
        };
        let candidate = components
            .iter()
            .enumerate()
            .filter_map(|(index, component)| {
                let chain = component.condition()?;
                let fallback = guarded_value_fallback(function, chain, value)?;
                Some((index, chain, fallback))
            })
            .min_by_key(|(_, chain, _)| chain.start);
        let Some((condition_index, chain, fallback)) = candidate else {
            continue;
        };

        // `guard and value` is expression-equivalent only when the guard's
        // other edge contributes false. For any other fallback, retain the
        // condition region and its PHI assignments; flattening the nested
        // value select would move its temporaries out of their control scope.
        if !is_boolean_false(function, expr_analysis, fallback) {
            removed[value_index] = true;
            continue;
        }

        let mut composed = value.clone();
        composed.start = chain.start;
        composed.kind = ValuePlanKind::Guarded {
            guards: chain.segments.clone(),
            value: Box::new(composed.kind),
        };
        components[value_index] = ControlComponent::Value(composed);
        removed[condition_index] = true;
    }

    // A value component owns every block in its select range. Discard suffix
    // condition facts that were independently recognized inside that range.
    let value_ranges = components
        .iter()
        .enumerate()
        .filter(|(index, _)| !removed[*index])
        .filter_map(|(index, component)| {
            component
                .value()
                .map(|plan| (index, plan.start, plan.merge))
        })
        .collect::<Vec<_>>();
    for (index, component) in components.iter().enumerate() {
        if removed[index] {
            continue;
        }
        let start = component.start();
        let merge = component.merge();
        if value_ranges
            .iter()
            .any(|(owner, owner_start, owner_merge)| {
                *owner != index && *owner_start <= start && merge <= *owner_merge
            })
        {
            removed[index] = true;
        }
    }

    components
        .into_iter()
        .enumerate()
        .filter_map(|(index, component)| (!removed[index]).then_some(component))
        .collect()
}

fn guarded_value_fallback(
    function: &SsaFunction,
    chain: &ConditionChain,
    value: &ValuePlan,
) -> Option<SsaRef> {
    // The asymmetry is deliberate: the TRUE-branch body of the chain must be
    // where the value plan starts, and their merges must coincide. Clippy
    // suggests `value.body`, which is not a field of `ValuePlan`.
    #[allow(clippy::suspicious_operation_groupings)]
    if chain.body != value.start
        || chain.merge != value.merge
        || !value.consumed_blocks().contains(&chain.false_target)
    {
        return None;
    }

    phi_sources(function, value.merge)
        .find(|phi| phi.dest == value.dest)
        .and_then(|phi| phi.operand_from(chain.false_target))
}

fn is_boolean_false(
    function: &SsaFunction,
    expr_analysis: &DecompileAnalysis,
    mut reference: SsaRef,
) -> bool {
    for _ in 0..8 {
        let Some(node) = expr_analysis
            .def_site(reference)
            .and_then(|id| expr_analysis.node(function, id))
        else {
            return false;
        };
        match &node.op {
            SsaOp::LoadBool { value: false, .. }
            | SsaOp::LoadLiteral {
                value: SsaLiteral::Boolean(false),
            } => return true,
            SsaOp::Move { src } => reference = *src,
            _ => return false,
        }
    }
    false
}

pub(crate) fn branch_at(function: &SsaFunction, id: NodeId) -> Option<&crate::ir::SsaNode> {
    function
        .blocks
        .get(id.block)
        .and_then(|block| block.nodes.get(id.node))
}

fn branch_info(
    function: &SsaFunction,
    block: usize,
    pc_map: &[Option<usize>],
) -> Option<BranchInfo> {
    conditionals::branch_info(function, block, pc_map)
}

fn has_prefix_def_used_after_branch(
    function: &SsaFunction,
    expr_analysis: &DecompileAnalysis,
    block: usize,
) -> bool {
    let Some(block_ref) = function.blocks.get(block) else {
        return false;
    };
    let Some(branch_index) = block_ref
        .nodes
        .iter()
        .position(|node| matches!(node.op, SsaOp::Branch { .. }))
    else {
        return false;
    };

    for node_index in 0..branch_index {
        let id = NodeId {
            block,
            node: node_index,
        };
        for reference in expr_analysis.defs_at(id) {
            let facts = expr_analysis.facts(*reference);
            if facts.phi_uses > 0 || facts.upvalue_captures > 0 {
                return true;
            }
            if expr_analysis
                .real_uses(*reference)
                .iter()
                .any(|use_site| use_site.block != block || use_site.node > branch_index)
            {
                return true;
            }
        }
    }

    false
}

fn has_unrelated_side_effect_before_branch(function: &SsaFunction, block: usize) -> bool {
    let Some(block) = function.blocks.get(block) else {
        return false;
    };
    let Some((branch_index, branch)) = block
        .nodes
        .iter()
        .enumerate()
        .find(|(_, node)| matches!(node.op, SsaOp::Branch { .. }))
    else {
        return false;
    };

    let mut needed = branch_operands(branch);
    for node in block.nodes.iter().take(branch_index).rev() {
        let feeds_condition = node.dest != SsaRef::None && needed.contains(&node.dest);
        if node.op.effects().is_observable() && !feeds_condition {
            return true;
        }
        if feeds_condition {
            add_used_refs(&node.op, &mut needed);
        }
    }

    false
}

fn branch_operands(node: &SsaNode) -> Vec<SsaRef> {
    let SsaOp::Branch { a, b, .. } = node.op else {
        return Vec::new();
    };
    [a, b]
        .into_iter()
        .filter(|reference| *reference != SsaRef::None)
        .collect()
}

fn add_used_refs(op: &SsaOp, out: &mut Vec<SsaRef>) {
    op.visit_uses(|reference, _| {
        if reference != SsaRef::None && !out.contains(&reference) {
            out.push(reference);
        }
    });
}

const fn is_pure_value_node(node: &SsaNode) -> bool {
    matches!(
        node.op,
        SsaOp::Phi { .. }
            | SsaOp::Nop
            | SsaOp::Jump { .. }
            | SsaOp::Branch { .. }
            | SsaOp::Move { .. }
            | SsaOp::LoadK { .. }
            | SsaOp::LoadLiteral { .. }
            | SsaOp::LoadBool { .. }
            | SsaOp::LoadNil { .. }
            | SsaOp::GetUpval { .. }
            | SsaOp::GetGlobal { .. }
            | SsaOp::GetTable { .. }
            | SsaOp::NewTable { .. }
            | SsaOp::SelfOp { .. }
            | SsaOp::BinOp { .. }
            | SsaOp::UnOp { .. }
            | SsaOp::Concat { .. }
            | SsaOp::Call { .. }
            | SsaOp::Closure { .. }
    )
}

fn phi_sources(function: &SsaFunction, block: usize) -> impl Iterator<Item = PhiData<'_>> {
    function
        .blocks
        .get(block)
        .into_iter()
        .flat_map(|block| block.nodes.iter())
        .filter_map(|node| {
            let SsaOp::Phi { operands, blocks } = &node.op else {
                return None;
            };
            Some(PhiData {
                dest: node.dest,
                pc: node.pc,
                operands,
                blocks,
            })
        })
}

#[derive(Debug, Clone, Copy)]
struct PhiData<'a> {
    dest: SsaRef,
    pc: i32,
    operands: &'a [SsaRef],
    blocks: &'a [usize],
}

impl PhiData<'_> {
    fn operand_from(self, block: usize) -> Option<SsaRef> {
        self.blocks
            .iter()
            .copied()
            .zip(self.operands.iter().copied())
            .find_map(|(source, operand)| (source == block).then_some(operand))
    }
}
