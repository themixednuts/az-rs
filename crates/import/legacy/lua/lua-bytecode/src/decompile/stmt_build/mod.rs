//! Linear SSA node stream to statement reconstruction for Phase 4.

use std::collections::HashSet;

use bstr::BString;

use crate::{
    LuaError,
    bytecode::OpcodeTable,
    chunk::{Constant, Proto},
    decompile::ast::{self, Expr, FuncBody, Name, Stmt, TableField},
    ir::{SsaFunction, SsaNode, SsaOp, SsaRef},
};

use super::{
    analysis::{DecompileAnalysis, NodeId},
    boolean::{BooleanAnalysis, ConditionChain, ValuePlan, normalize},
    closure,
    expr_build::{ExprBuilder, global_expr_from_name, index_expr, is_inlineable_def},
    multi::{self, plan::NodeEmission},
    naming::LocalBinding,
    naming::NameResolver,
    reconstruction::{BindingId, ReconstructionPlan},
    region::LinearRegion,
};

const LFIELDS_PER_FLUSH: i64 = 50;

/// Build a compact AST block from a linear SSA region.
///
/// # Errors
///
/// Returns [`LuaError::Unsupported`] for an SSA op with no statement form in
/// this phase, and any error the expression builder returns for an operand
/// that cannot be resolved at its use site.
pub fn build_block(
    proto: &Proto,
    function: &SsaFunction,
    table: &OpcodeTable,
    region: &LinearRegion,
    analysis: &DecompileAnalysis,
    names: &NameResolver<'_>,
) -> Result<ast::Block, LuaError> {
    let booleans = BooleanAnalysis::empty();
    let plan = ReconstructionPlan::build(proto, function, table, analysis, names, &booleans, None);
    StatementBuilder::new(proto, function, table, analysis, names, &booleans, &plan).build(region)
}

mod conditions;
mod emit;
mod helpers;
mod materialize;

use helpers::is_stable_assignment_target;
pub(crate) use helpers::{assign_one, local_one};

pub(crate) struct StatementBuilder<'a> {
    proto: &'a Proto,
    function: &'a SsaFunction,
    table: &'a OpcodeTable,
    analysis: &'a DecompileAnalysis,
    names: &'a NameResolver<'a>,
    booleans: &'a BooleanAnalysis,
    plan: &'a ReconstructionPlan,
    exprs: ExprBuilder<'a>,
    pending_declarations: HashSet<BindingId>,
    claimed_control_phis: HashSet<SsaRef>,
}

impl<'a> StatementBuilder<'a> {
    pub(crate) fn new(
        proto: &'a Proto,
        function: &'a SsaFunction,
        table: &'a OpcodeTable,
        analysis: &'a DecompileAnalysis,
        names: &'a NameResolver<'a>,
        booleans: &'a BooleanAnalysis,
        plan: &'a ReconstructionPlan,
    ) -> Self {
        let pending_declarations = plan.declaration_bindings();
        Self {
            proto,
            function,
            table,
            analysis,
            names,
            booleans,
            plan,
            exprs: ExprBuilder::new(proto, function, table, analysis, names, booleans, plan),
            pending_declarations,
            claimed_control_phis: HashSet::new(),
        }
    }

    fn build(mut self, region: &LinearRegion) -> Result<ast::Block, LuaError> {
        Ok(ast::Block::new(self.emit_linear_region(region)?))
    }

    pub(crate) fn emit_linear_region(
        &mut self,
        region: &LinearRegion,
    ) -> Result<Vec<Stmt>, LuaError> {
        self.emit_node_ids(region.nodes.iter().copied())
    }

    pub(crate) fn emit_entry_declarations(&self) -> Option<Stmt> {
        let names = self.plan.entry_declarations().to_vec();
        let values = self
            .names
            .legacy_vararg_table_name()
            .filter(|legacy_arg| {
                names
                    .iter()
                    .any(|name| name.binding() == legacy_arg.binding())
            })
            .map(|_| {
                vec![Expr::Table(vec![
                    TableField::Named {
                        name: Name::from("n"),
                        value: Expr::Call {
                            func: Box::new(Expr::Global(BString::from("select"))),
                            args: vec![Expr::Str(BString::from("#")), Expr::VarArg],
                            method: None,
                        },
                    },
                    TableField::List(Expr::VarArg),
                ])]
            })
            .unwrap_or_default();
        (!names.is_empty()).then_some(Stmt::Local {
            names,
            attribs: Vec::new(),
            values,
        })
    }

    pub(crate) fn emit_node_ids(
        &mut self,
        nodes: impl IntoIterator<Item = NodeId>,
    ) -> Result<Vec<Stmt>, LuaError> {
        let mut stmts = Vec::new();

        for node_id in nodes {
            let Some(node) = self.analysis.node(self.function, node_id) else {
                continue;
            };
            if let NodeEmission::Owner(plan) = self.plan.node_emission(node_id).clone() {
                let emitted = multi::emit(self, &plan)?;
                let is_return = matches!(emitted.last(), Some(Stmt::Return(_)));
                stmts.extend(emitted);
                if is_return {
                    break;
                }
                continue;
            }
            if matches!(
                self.plan.node_emission(node_id),
                NodeEmission::Member { .. }
            ) {
                continue;
            }
            if matches!(self.plan.node_emission(node_id), NodeEmission::Omitted) {
                continue;
            }
            if let Some(stmt) = self.emit_node(node_id, node)? {
                let is_return = matches!(stmt, Stmt::Return(_));
                stmts.push(stmt);
                if is_return {
                    break;
                }
            }
        }

        Ok(stmts)
    }

    pub(crate) fn expr_for_node(&mut self, node: &SsaNode) -> Result<Expr, LuaError> {
        self.exprs.node_expr(node)
    }

    pub(crate) fn expr_for_ref(&mut self, reference: SsaRef, pc: i32) -> Result<Expr, LuaError> {
        self.exprs.expr_for_ref(reference, pc)
    }

    pub(crate) fn expr_for_fixed_last_ref(
        &mut self,
        reference: SsaRef,
        pc: i32,
    ) -> Result<Expr, LuaError> {
        self.exprs.expr_for_fixed_last_ref(reference, pc)
    }

    pub(crate) fn call_expr(
        &mut self,
        func: SsaRef,
        args: &[SsaRef],
        arg_count: i32,
        pc: i32,
    ) -> Result<Expr, LuaError> {
        self.exprs
            .call_expr_with_arg_count(func, args, arg_count, pc)
    }

    pub(crate) fn node(&self, id: NodeId) -> Option<&SsaNode> {
        self.analysis.node(self.function, id)
    }

    pub(crate) const fn proto(&self) -> &'a Proto {
        self.proto
    }

    pub(crate) const fn function(&self) -> &'a SsaFunction {
        self.function
    }

    pub(crate) fn will_declare(&self, reference: SsaRef) -> bool {
        self.plan
            .binding(reference)
            .is_some_and(|binding| self.pending_declarations.contains(&binding))
    }

    pub(crate) fn claim_declaration(&mut self, reference: SsaRef) -> bool {
        self.plan
            .binding(reference)
            .is_some_and(|binding| self.pending_declarations.remove(&binding))
    }

    /// Claim the outermost structured region responsible for initializing a
    /// PHI destination. Nested regions that share the same merge still emit
    /// edge-specific assignments, but must not reset the value on entry.
    pub(crate) fn claim_control_phi(&mut self, reference: SsaRef) -> bool {
        self.claimed_control_phis.insert(reference)
    }

    pub(crate) const fn table(&self) -> &'a OpcodeTable {
        self.table
    }

    pub(crate) const fn names(&self) -> &'a NameResolver<'a> {
        self.names
    }

    pub(crate) fn def_at_reg(&self, id: NodeId, reg: u16) -> Option<SsaRef> {
        self.analysis.def_at_reg(id, reg)
    }

    pub(crate) fn binding_for_def(&self, reg: u16, pc: i32) -> Option<LocalBinding> {
        self.names.binding_for_def(reg, pc)
    }

    pub(crate) fn materialization_pc(&self, reference: SsaRef) -> Option<i32> {
        self.plan.materialization_pc(reference)
    }

    pub(crate) fn name_for_ref(&self, reference: SsaRef, pc: i32) -> Name {
        self.names.name_for_ref(reference, pc)
    }

    pub(crate) fn name_for_binding_def(&self, binding: &LocalBinding, reference: SsaRef) -> Name {
        self.names.name_for_binding_def(binding, reference)
    }

    pub(crate) fn activate(&mut self, reference: SsaRef) {
        self.exprs.activate(reference);
    }
}
