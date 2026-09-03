//! Lexical-scope normalization for reconstructed local bindings.
//!
//! SSA and the region tree describe value flow and control flow, respectively,
//! but neither one alone owns the lexical scope of a source local. A value can
//! first materialize inside a reconstructed loop or branch and still feed code
//! after that construct. In that case, claiming the first materialization as
//! the `local` declaration produces an AST whose binding does not dominate all
//! of its uses.
//!
//! This pass widens only synthesized declarations whose binding identity proves
//! that they escape their reconstructed block. The initializer remains at its
//! original control-flow location as an assignment, so evaluation frequency and
//! branch behavior do not change. Declarations that remain within their block,
//! including per-iteration locals captured by closures, are left untouched.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use crate::decompile::ast::{BindingId, Block, Expr, FuncBody, Name, Stmt, TableField};

/// Make synthesized local declarations dominate every reference to their
/// binding, recursively normalizing nested functions as separate lexical units.
pub fn normalize(mut block: Block) -> Block {
    normalize_nested_functions(&mut block);
    normalize_function_block(&mut block);
    block
}

fn normalize_nested_functions(block: &mut Block) {
    for stmt in &mut block.0 {
        normalize_nested_functions_in_stmt(stmt);
    }
}

fn normalize_nested_functions_in_stmt(stmt: &mut Stmt) {
    match stmt {
        Stmt::Local { values, .. } | Stmt::Assign { values, .. } | Stmt::Return(values) => {
            for value in values {
                normalize_nested_functions_in_expr(value);
            }
        }
        Stmt::Call(expr) => normalize_nested_functions_in_expr(expr),
        Stmt::Do(body) => normalize_nested_functions(body),
        Stmt::While { cond, body } => {
            normalize_nested_functions_in_expr(cond);
            normalize_nested_functions(body);
        }
        Stmt::Repeat { body, cond } => {
            normalize_nested_functions(body);
            normalize_nested_functions_in_expr(cond);
        }
        Stmt::If { arms, else_ } => {
            for (cond, body) in arms {
                normalize_nested_functions_in_expr(cond);
                normalize_nested_functions(body);
            }
            if let Some(body) = else_ {
                normalize_nested_functions(body);
            }
        }
        Stmt::NumericFor {
            start,
            stop,
            step,
            body,
            ..
        } => {
            normalize_nested_functions_in_expr(start);
            normalize_nested_functions_in_expr(stop);
            if let Some(step) = step {
                normalize_nested_functions_in_expr(step);
            }
            normalize_nested_functions(body);
        }
        Stmt::GenericFor { exprs, body, .. } => {
            for expr in exprs {
                normalize_nested_functions_in_expr(expr);
            }
            normalize_nested_functions(body);
        }
        Stmt::Function { body, .. } | Stmt::FunctionDecl { body, .. } => {
            body.body = normalize(std::mem::replace(&mut body.body, Block::empty()));
        }
        Stmt::Break | Stmt::Goto(_) | Stmt::Label(_) => {}
    }

    if let Stmt::Assign { targets, .. } = stmt {
        for target in targets {
            normalize_nested_functions_in_expr(target);
        }
    }
}

fn normalize_nested_functions_in_expr(expr: &mut Expr) {
    match expr {
        Expr::Index { obj, key } => {
            normalize_nested_functions_in_expr(obj);
            normalize_nested_functions_in_expr(key);
        }
        Expr::Field { obj, .. } => normalize_nested_functions_in_expr(obj),
        Expr::Call { func, args, .. } => {
            normalize_nested_functions_in_expr(func);
            for arg in args {
                normalize_nested_functions_in_expr(arg);
            }
        }
        Expr::Function(body) => {
            body.body = normalize(std::mem::replace(&mut body.body, Block::empty()));
        }
        Expr::Table(fields) => {
            for field in fields {
                match field {
                    TableField::List(value) | TableField::Named { value, .. } => {
                        normalize_nested_functions_in_expr(value);
                    }
                    TableField::ExprKey { key, value } => {
                        normalize_nested_functions_in_expr(key);
                        normalize_nested_functions_in_expr(value);
                    }
                }
            }
        }
        Expr::Binary { lhs, rhs, .. } => {
            normalize_nested_functions_in_expr(lhs);
            normalize_nested_functions_in_expr(rhs);
        }
        Expr::Unary { operand, .. } | Expr::Paren(operand) => {
            normalize_nested_functions_in_expr(operand);
        }
        Expr::Nil
        | Expr::True
        | Expr::False
        | Expr::VarArg
        | Expr::Number(_)
        | Expr::Integer(_)
        | Expr::Str(_)
        | Expr::Name(_)
        | Expr::Global(_) => {}
    }
}

fn normalize_function_block(block: &mut Block) {
    let mut facts = BTreeMap::<BindingId, BindingFacts>::new();
    collect_block_facts(block, &BlockPath::default(), &mut facts);
    let plan = ScopePlan::build(facts);
    if plan.hoisted.is_empty() {
        return;
    }

    let mut spellings = HashSet::new();
    collect_spellings_in_block(block, &mut spellings);
    let mut temps = TempNames {
        used: spellings,
        next: 0,
    };
    rewrite_block(block, &BlockPath::default(), &plan, &mut temps);
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
struct BlockPath(Vec<BlockEdge>);

impl BlockPath {
    fn child(&self, edge: BlockEdge) -> Self {
        let mut path = self.0.clone();
        path.push(edge);
        Self(path)
    }

    fn is_ancestor_of(&self, other: &Self) -> bool {
        other.0.starts_with(&self.0)
    }

    fn common_ancestor(&self, other: &Self) -> Self {
        let length = self
            .0
            .iter()
            .zip(&other.0)
            .take_while(|(left, right)| left == right)
            .count();
        Self(self.0[..length].to_vec())
    }

    fn containing_statement(&self, site: &Site) -> usize {
        if self == &site.path {
            site.stmt
        } else {
            site.path.0[self.0.len()].statement()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum BlockEdge {
    Do(usize),
    While(usize),
    Repeat(usize),
    IfArm { stmt: usize, arm: usize },
    IfElse(usize),
    NumericFor(usize),
    GenericFor(usize),
}

impl BlockEdge {
    const fn statement(self) -> usize {
        match self {
            Self::Do(stmt)
            | Self::While(stmt)
            | Self::Repeat(stmt)
            | Self::IfArm { stmt, .. }
            | Self::IfElse(stmt)
            | Self::NumericFor(stmt)
            | Self::GenericFor(stmt) => stmt,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Site {
    path: BlockPath,
    stmt: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeclarationKind {
    Local,
    LocalFunction,
}

#[derive(Debug, Clone)]
struct Declaration {
    site: Site,
    name: Name,
    kind: DeclarationKind,
    attributed: bool,
}

#[derive(Debug, Default)]
struct BindingFacts {
    declarations: Vec<Declaration>,
    uses: Vec<Site>,
}

fn collect_block_facts(
    block: &Block,
    path: &BlockPath,
    facts: &mut BTreeMap<BindingId, BindingFacts>,
) {
    for (stmt_index, stmt) in block.0.iter().enumerate() {
        let site = Site {
            path: path.clone(),
            stmt: stmt_index,
        };
        collect_stmt_facts(stmt, &site, facts);

        match stmt {
            Stmt::Do(body) => {
                collect_block_facts(body, &path.child(BlockEdge::Do(stmt_index)), facts);
            }
            Stmt::While { body, .. } => {
                collect_block_facts(body, &path.child(BlockEdge::While(stmt_index)), facts);
            }
            Stmt::Repeat { body, cond } => {
                let body_path = path.child(BlockEdge::Repeat(stmt_index));
                collect_block_facts(body, &body_path, facts);
                collect_expr_uses(
                    cond,
                    &Site {
                        path: body_path,
                        stmt: body.0.len(),
                    },
                    facts,
                );
            }
            Stmt::If { arms, else_ } => {
                for (arm_index, (_, body)) in arms.iter().enumerate() {
                    collect_block_facts(
                        body,
                        &path.child(BlockEdge::IfArm {
                            stmt: stmt_index,
                            arm: arm_index,
                        }),
                        facts,
                    );
                }
                if let Some(body) = else_ {
                    collect_block_facts(body, &path.child(BlockEdge::IfElse(stmt_index)), facts);
                }
            }
            Stmt::NumericFor { body, .. } => {
                collect_block_facts(body, &path.child(BlockEdge::NumericFor(stmt_index)), facts);
            }
            Stmt::GenericFor { body, .. } => {
                collect_block_facts(body, &path.child(BlockEdge::GenericFor(stmt_index)), facts);
            }
            Stmt::Local { .. }
            | Stmt::Assign { .. }
            | Stmt::Call(_)
            | Stmt::Function { .. }
            | Stmt::FunctionDecl { .. }
            | Stmt::Return(_)
            | Stmt::Break
            | Stmt::Goto(_)
            | Stmt::Label(_) => {}
        }
    }
}

fn collect_stmt_facts(stmt: &Stmt, site: &Site, facts: &mut BTreeMap<BindingId, BindingFacts>) {
    match stmt {
        Stmt::Local {
            names,
            attribs,
            values,
        } => {
            for (index, name) in names.iter().enumerate() {
                record_declaration(
                    name,
                    site,
                    DeclarationKind::Local,
                    attribs.get(index).is_some_and(Option::is_some),
                    facts,
                );
            }
            collect_exprs_uses(values, site, facts);
        }
        Stmt::Assign { targets, values } => {
            collect_exprs_uses(targets, site, facts);
            collect_exprs_uses(values, site, facts);
        }
        Stmt::Call(expr) => collect_expr_uses(expr, site, facts),
        // These introduce no uses of their own; their bodies are visited
        // through the block walk.
        Stmt::Do(_) | Stmt::Repeat { .. } | Stmt::Break | Stmt::Goto(_) | Stmt::Label(_) => {}
        Stmt::While { cond, .. } => collect_expr_uses(cond, site, facts),
        Stmt::If { arms, .. } => {
            for (cond, _) in arms {
                collect_expr_uses(cond, site, facts);
            }
        }
        Stmt::NumericFor {
            start, stop, step, ..
        } => {
            collect_expr_uses(start, site, facts);
            collect_expr_uses(stop, site, facts);
            if let Some(step) = step {
                collect_expr_uses(step, site, facts);
            }
        }
        Stmt::GenericFor { exprs, .. } => collect_exprs_uses(exprs, site, facts),
        Stmt::Function { name, body, local } => {
            if *local {
                record_declaration(name, site, DeclarationKind::LocalFunction, false, facts);
            } else {
                record_use(name, site, facts);
            }
            collect_nested_function_uses(body, site, facts);
        }
        Stmt::FunctionDecl { name, body } => {
            for part in &name.path {
                record_use(part, site, facts);
            }
            collect_nested_function_uses(body, site, facts);
        }
        Stmt::Return(values) => collect_exprs_uses(values, site, facts),
    }
}

fn record_declaration(
    name: &Name,
    site: &Site,
    kind: DeclarationKind,
    attributed: bool,
    facts: &mut BTreeMap<BindingId, BindingFacts>,
) {
    let Some(binding) = name.binding().cloned() else {
        return;
    };
    facts
        .entry(binding)
        .or_default()
        .declarations
        .push(Declaration {
            site: site.clone(),
            name: name.clone(),
            kind,
            attributed,
        });
}

fn record_use(name: &Name, site: &Site, facts: &mut BTreeMap<BindingId, BindingFacts>) {
    let Some(binding) = name.binding().cloned() else {
        return;
    };
    facts.entry(binding).or_default().uses.push(site.clone());
}

fn collect_exprs_uses(exprs: &[Expr], site: &Site, facts: &mut BTreeMap<BindingId, BindingFacts>) {
    for expr in exprs {
        collect_expr_uses(expr, site, facts);
    }
}

fn collect_expr_uses(expr: &Expr, site: &Site, facts: &mut BTreeMap<BindingId, BindingFacts>) {
    match expr {
        Expr::Name(name) => record_use(name, site, facts),
        Expr::Index { obj, key } => {
            collect_expr_uses(obj, site, facts);
            collect_expr_uses(key, site, facts);
        }
        Expr::Field { obj, .. } => collect_expr_uses(obj, site, facts),
        Expr::Call { func, args, .. } => {
            collect_expr_uses(func, site, facts);
            collect_exprs_uses(args, site, facts);
        }
        Expr::Function(body) => collect_nested_function_uses(body, site, facts),
        Expr::Table(fields) => {
            for field in fields {
                match field {
                    TableField::List(value) | TableField::Named { value, .. } => {
                        collect_expr_uses(value, site, facts);
                    }
                    TableField::ExprKey { key, value } => {
                        collect_expr_uses(key, site, facts);
                        collect_expr_uses(value, site, facts);
                    }
                }
            }
        }
        Expr::Binary { lhs, rhs, .. } => {
            collect_expr_uses(lhs, site, facts);
            collect_expr_uses(rhs, site, facts);
        }
        Expr::Unary { operand, .. } | Expr::Paren(operand) => {
            collect_expr_uses(operand, site, facts);
        }
        Expr::Nil
        | Expr::True
        | Expr::False
        | Expr::VarArg
        | Expr::Number(_)
        | Expr::Integer(_)
        | Expr::Str(_)
        | Expr::Global(_) => {}
    }
}

fn collect_nested_function_uses(
    body: &FuncBody,
    site: &Site,
    facts: &mut BTreeMap<BindingId, BindingFacts>,
) {
    for param in &body.params {
        record_use(param, site, facts);
    }
    if let Some(receiver) = &body.implicit_receiver {
        record_use(receiver, site, facts);
    }
    collect_all_bound_names_in_block(&body.body, site, facts);
}

fn collect_all_bound_names_in_block(
    block: &Block,
    site: &Site,
    facts: &mut BTreeMap<BindingId, BindingFacts>,
) {
    for stmt in &block.0 {
        match stmt {
            Stmt::Local { names, values, .. } => {
                for name in names {
                    record_use(name, site, facts);
                }
                collect_exprs_uses(values, site, facts);
            }
            Stmt::Assign { targets, values } => {
                collect_exprs_uses(targets, site, facts);
                collect_exprs_uses(values, site, facts);
            }
            Stmt::Call(expr) => collect_expr_uses(expr, site, facts),
            Stmt::Do(body) => collect_all_bound_names_in_block(body, site, facts),
            Stmt::While { cond, body } => {
                collect_expr_uses(cond, site, facts);
                collect_all_bound_names_in_block(body, site, facts);
            }
            Stmt::Repeat { body, cond } => {
                collect_all_bound_names_in_block(body, site, facts);
                collect_expr_uses(cond, site, facts);
            }
            Stmt::If { arms, else_ } => {
                for (cond, body) in arms {
                    collect_expr_uses(cond, site, facts);
                    collect_all_bound_names_in_block(body, site, facts);
                }
                if let Some(body) = else_ {
                    collect_all_bound_names_in_block(body, site, facts);
                }
            }
            Stmt::NumericFor {
                var,
                start,
                stop,
                step,
                body,
            } => {
                record_use(var, site, facts);
                collect_expr_uses(start, site, facts);
                collect_expr_uses(stop, site, facts);
                if let Some(step) = step {
                    collect_expr_uses(step, site, facts);
                }
                collect_all_bound_names_in_block(body, site, facts);
            }
            Stmt::GenericFor { names, exprs, body } => {
                for name in names {
                    record_use(name, site, facts);
                }
                collect_exprs_uses(exprs, site, facts);
                collect_all_bound_names_in_block(body, site, facts);
            }
            Stmt::Function { name, body, .. } => {
                record_use(name, site, facts);
                collect_nested_function_uses(body, site, facts);
            }
            Stmt::FunctionDecl { name, body } => {
                for part in &name.path {
                    record_use(part, site, facts);
                }
                collect_nested_function_uses(body, site, facts);
            }
            Stmt::Return(values) => collect_exprs_uses(values, site, facts),
            Stmt::Break | Stmt::Goto(_) | Stmt::Label(_) => {}
        }
    }
}

#[derive(Debug, Default)]
struct ScopePlan {
    hoisted: BTreeSet<BindingId>,
    insertions: BTreeMap<BlockPath, BTreeMap<usize, Vec<Name>>>,
}

impl ScopePlan {
    fn build(facts: BTreeMap<BindingId, BindingFacts>) -> Self {
        let mut planned = Vec::new();
        for (binding, facts) in facts {
            let [declaration] = facts.declarations.as_slice() else {
                continue;
            };
            if declaration.attributed
                || !declaration.name.is_synthetic()
                || facts.uses.is_empty()
                || facts
                    .uses
                    .iter()
                    .all(|site| declaration_visible_at(declaration, site))
            {
                continue;
            }

            let target = facts
                .uses
                .iter()
                .fold(declaration.site.path.clone(), |scope, use_site| {
                    scope.common_ancestor(&use_site.path)
                });
            let insert_before = std::iter::once(&declaration.site)
                .chain(&facts.uses)
                .map(|site| target.containing_statement(site))
                .min()
                .unwrap_or(0);
            planned.push((
                target,
                insert_before,
                declaration.site.clone(),
                binding,
                declaration.name.clone(),
            ));
        }

        planned.sort_by(|left, right| {
            (
                &left.0,
                left.1,
                &left.2.path,
                left.2.stmt,
                left.4.as_bytes(),
            )
                .cmp(&(
                    &right.0,
                    right.1,
                    &right.2.path,
                    right.2.stmt,
                    right.4.as_bytes(),
                ))
        });

        let mut plan = Self::default();
        for (target, insert_before, _, binding, name) in planned {
            plan.hoisted.insert(binding);
            plan.insertions
                .entry(target)
                .or_default()
                .entry(insert_before)
                .or_default()
                .push(name);
        }
        plan
    }
}

fn declaration_visible_at(declaration: &Declaration, use_site: &Site) -> bool {
    if declaration.site.path == use_site.path {
        return use_site.stmt > declaration.site.stmt
            || (use_site.stmt == declaration.site.stmt
                && declaration.kind == DeclarationKind::LocalFunction);
    }
    if !declaration.site.path.is_ancestor_of(&use_site.path) {
        return false;
    }
    let containing_statement = use_site.path.0[declaration.site.path.0.len()].statement();
    containing_statement > declaration.site.stmt
        || (containing_statement == declaration.site.stmt
            && declaration.kind == DeclarationKind::LocalFunction)
}

struct TempNames {
    used: HashSet<Vec<u8>>,
    next: usize,
}

impl TempNames {
    fn fresh(&mut self) -> Name {
        loop {
            let candidate = format!("__az_scope_{}", self.next);
            self.next += 1;
            if self.used.insert(candidate.as_bytes().to_vec()) {
                return Name::synthetic(candidate);
            }
        }
    }
}

fn rewrite_block(block: &mut Block, path: &BlockPath, plan: &ScopePlan, temps: &mut TempNames) {
    let original = std::mem::take(&mut block.0);
    let mut rewritten = Vec::with_capacity(original.len());
    let insertions = plan.insertions.get(path);

    for (stmt_index, mut stmt) in original.into_iter().enumerate() {
        if let Some(names) = insertions.and_then(|by_index| by_index.get(&stmt_index)) {
            rewritten.push(Stmt::Local {
                names: names.clone(),
                attribs: Vec::new(),
                values: Vec::new(),
            });
        }

        rewrite_child_blocks(&mut stmt, path, stmt_index, plan, temps);
        rewrite_declaration(stmt, plan, temps, &mut rewritten);
    }
    block.0 = rewritten;
}

fn rewrite_child_blocks(
    stmt: &mut Stmt,
    path: &BlockPath,
    stmt_index: usize,
    plan: &ScopePlan,
    temps: &mut TempNames,
) {
    match stmt {
        Stmt::Do(body) => rewrite_block(body, &path.child(BlockEdge::Do(stmt_index)), plan, temps),
        Stmt::While { body, .. } => {
            rewrite_block(body, &path.child(BlockEdge::While(stmt_index)), plan, temps);
        }
        Stmt::Repeat { body, .. } => rewrite_block(
            body,
            &path.child(BlockEdge::Repeat(stmt_index)),
            plan,
            temps,
        ),
        Stmt::If { arms, else_ } => {
            for (arm_index, (_, body)) in arms.iter_mut().enumerate() {
                rewrite_block(
                    body,
                    &path.child(BlockEdge::IfArm {
                        stmt: stmt_index,
                        arm: arm_index,
                    }),
                    plan,
                    temps,
                );
            }
            if let Some(body) = else_ {
                rewrite_block(
                    body,
                    &path.child(BlockEdge::IfElse(stmt_index)),
                    plan,
                    temps,
                );
            }
        }
        Stmt::NumericFor { body, .. } => rewrite_block(
            body,
            &path.child(BlockEdge::NumericFor(stmt_index)),
            plan,
            temps,
        ),
        Stmt::GenericFor { body, .. } => rewrite_block(
            body,
            &path.child(BlockEdge::GenericFor(stmt_index)),
            plan,
            temps,
        ),
        Stmt::Local { .. }
        | Stmt::Assign { .. }
        | Stmt::Call(_)
        | Stmt::Function { .. }
        | Stmt::FunctionDecl { .. }
        | Stmt::Return(_)
        | Stmt::Break
        | Stmt::Goto(_)
        | Stmt::Label(_) => {}
    }
}

fn rewrite_declaration(
    stmt: Stmt,
    plan: &ScopePlan,
    temps: &mut TempNames,
    output: &mut Vec<Stmt>,
) {
    match stmt {
        Stmt::Local {
            names,
            attribs,
            values,
        } => rewrite_local(names, attribs, values, plan, temps, output),
        Stmt::Function {
            name,
            body,
            local: true,
        } if name
            .binding()
            .is_some_and(|binding| plan.hoisted.contains(binding)) =>
        {
            output.push(Stmt::Function {
                name,
                body,
                local: false,
            });
        }
        other => output.push(other),
    }
}

fn rewrite_local(
    names: Vec<Name>,
    attribs: Vec<Option<crate::decompile::ast::Attrib>>,
    values: Vec<Expr>,
    plan: &ScopePlan,
    temps: &mut TempNames,
    output: &mut Vec<Stmt>,
) {
    let hoisted = names
        .iter()
        .map(|name| {
            name.binding()
                .is_some_and(|binding| plan.hoisted.contains(binding))
        })
        .collect::<Vec<_>>();
    if !hoisted.iter().any(|is_hoisted| *is_hoisted) {
        output.push(Stmt::Local {
            names,
            attribs,
            values,
        });
        return;
    }

    let all_hoisted = hoisted.iter().all(|is_hoisted| *is_hoisted);
    if all_hoisted {
        if !values.is_empty() {
            output.push(Stmt::Assign {
                targets: names.into_iter().map(Expr::Name).collect(),
                values,
            });
        }
        return;
    }

    let mut kept_names = Vec::new();
    let mut kept_attribs = Vec::new();
    for (index, name) in names.iter().enumerate() {
        if !hoisted[index] {
            kept_names.push(name.clone());
            kept_attribs.push(attribs.get(index).copied().unwrap_or(None));
        }
    }
    if values.is_empty() {
        output.push(Stmt::Local {
            names: kept_names,
            attribs: kept_attribs,
            values,
        });
        return;
    }

    let temporary_names = (0..names.len()).map(|_| temps.fresh()).collect::<Vec<_>>();
    output.push(Stmt::Local {
        names: temporary_names.clone(),
        attribs: Vec::new(),
        values,
    });

    let mut hoisted_targets = Vec::new();
    let mut hoisted_values = Vec::new();
    let mut kept_values = Vec::new();
    for (index, name) in names.into_iter().enumerate() {
        if hoisted[index] {
            hoisted_targets.push(Expr::Name(name));
            hoisted_values.push(Expr::Name(temporary_names[index].clone()));
        } else {
            kept_values.push(Expr::Name(temporary_names[index].clone()));
        }
    }
    output.push(Stmt::Assign {
        targets: hoisted_targets,
        values: hoisted_values,
    });
    output.push(Stmt::Local {
        names: kept_names,
        attribs: kept_attribs,
        values: kept_values,
    });
}

fn collect_spellings_in_block(block: &Block, spellings: &mut HashSet<Vec<u8>>) {
    for stmt in &block.0 {
        collect_spellings_in_stmt(stmt, spellings);
    }
}

fn collect_spellings_in_stmt(stmt: &Stmt, spellings: &mut HashSet<Vec<u8>>) {
    match stmt {
        Stmt::Local { names, values, .. } => {
            collect_name_spellings(names, spellings);
            collect_expr_spellings(values, spellings);
        }
        Stmt::Assign { targets, values } => {
            collect_expr_spellings(targets, spellings);
            collect_expr_spellings(values, spellings);
        }
        Stmt::Call(expr) => collect_expr_spelling(expr, spellings),
        Stmt::Do(body) => collect_spellings_in_block(body, spellings),
        Stmt::While { cond, body } | Stmt::Repeat { body, cond } => {
            collect_expr_spelling(cond, spellings);
            collect_spellings_in_block(body, spellings);
        }
        Stmt::If { arms, else_ } => {
            for (cond, body) in arms {
                collect_expr_spelling(cond, spellings);
                collect_spellings_in_block(body, spellings);
            }
            if let Some(body) = else_ {
                collect_spellings_in_block(body, spellings);
            }
        }
        Stmt::NumericFor {
            var,
            start,
            stop,
            step,
            body,
        } => {
            collect_name_spelling(var, spellings);
            collect_expr_spelling(start, spellings);
            collect_expr_spelling(stop, spellings);
            if let Some(step) = step {
                collect_expr_spelling(step, spellings);
            }
            collect_spellings_in_block(body, spellings);
        }
        Stmt::GenericFor { names, exprs, body } => {
            collect_name_spellings(names, spellings);
            collect_expr_spellings(exprs, spellings);
            collect_spellings_in_block(body, spellings);
        }
        Stmt::Function { name, body, .. } => {
            collect_name_spelling(name, spellings);
            collect_spellings_in_func_body(body, spellings);
        }
        Stmt::FunctionDecl { name, body } => {
            collect_name_spellings(&name.path, spellings);
            if let Some(method) = &name.method {
                collect_name_spelling(method, spellings);
            }
            collect_spellings_in_func_body(body, spellings);
        }
        Stmt::Return(values) => collect_expr_spellings(values, spellings),
        Stmt::Break | Stmt::Goto(_) | Stmt::Label(_) => {}
    }
}

fn collect_spellings_in_func_body(body: &FuncBody, spellings: &mut HashSet<Vec<u8>>) {
    collect_name_spellings(&body.params, spellings);
    if let Some(receiver) = &body.implicit_receiver {
        collect_name_spelling(receiver, spellings);
    }
    collect_spellings_in_block(&body.body, spellings);
}

fn collect_name_spellings(names: &[Name], spellings: &mut HashSet<Vec<u8>>) {
    for name in names {
        collect_name_spelling(name, spellings);
    }
}

fn collect_name_spelling(name: &Name, spellings: &mut HashSet<Vec<u8>>) {
    spellings.insert(name.as_bytes().to_vec());
}

fn collect_expr_spellings(exprs: &[Expr], spellings: &mut HashSet<Vec<u8>>) {
    for expr in exprs {
        collect_expr_spelling(expr, spellings);
    }
}

fn collect_expr_spelling(expr: &Expr, spellings: &mut HashSet<Vec<u8>>) {
    match expr {
        Expr::Name(name) => collect_name_spelling(name, spellings),
        Expr::Global(name) => {
            spellings.insert(name.to_vec());
        }
        Expr::Index { obj, key } => {
            collect_expr_spelling(obj, spellings);
            collect_expr_spelling(key, spellings);
        }
        Expr::Field { obj, .. } => collect_expr_spelling(obj, spellings),
        Expr::Call { func, args, .. } => {
            collect_expr_spelling(func, spellings);
            collect_expr_spellings(args, spellings);
        }
        Expr::Function(body) => collect_spellings_in_func_body(body, spellings),
        Expr::Table(fields) => {
            for field in fields {
                match field {
                    TableField::List(value) | TableField::Named { value, .. } => {
                        collect_expr_spelling(value, spellings);
                    }
                    TableField::ExprKey { key, value } => {
                        collect_expr_spelling(key, spellings);
                        collect_expr_spelling(value, spellings);
                    }
                }
            }
        }
        Expr::Binary { lhs, rhs, .. } => {
            collect_expr_spelling(lhs, spellings);
            collect_expr_spelling(rhs, spellings);
        }
        Expr::Unary { operand, .. } | Expr::Paren(operand) => {
            collect_expr_spelling(operand, spellings);
        }
        Expr::Nil
        | Expr::True
        | Expr::False
        | Expr::VarArg
        | Expr::Number(_)
        | Expr::Integer(_)
        | Expr::Str(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decompile::ast::{FunctionId, Name};

    fn bound(name: &str, slot: usize) -> Name {
        Name::synthetic(name).with_binding(BindingId::synthetic(&FunctionId::root(), slot))
    }

    #[test]
    fn escaping_loop_local_moves_to_the_parent_without_moving_its_initializer() {
        let value = bound("value", 1);
        let mut block = Block(vec![
            Stmt::While {
                cond: Expr::True,
                body: Block(vec![Stmt::Local {
                    names: vec![value.clone()],
                    attribs: Vec::new(),
                    values: vec![Expr::Number(7.0)],
                }]),
            },
            Stmt::Return(vec![Expr::Name(value.clone())]),
        ]);

        normalize_function_block(&mut block);

        assert!(matches!(
            &block.0[0],
            Stmt::Local { names, values, .. }
                if names == &vec![value.clone()] && values.is_empty()
        ));
        let Stmt::While { body, .. } = &block.0[1] else {
            panic!("expected loop after widened declaration");
        };
        assert!(matches!(
            &body.0[0],
            Stmt::Assign { targets, values }
                if targets == &vec![Expr::Name(value)] && values == &vec![Expr::Number(7.0)]
        ));
    }

    #[test]
    fn per_iteration_captured_local_stays_inside_the_loop() {
        let value = bound("value", 1);
        let closure = FuncBody::new(
            Vec::new(),
            false,
            Block(vec![Stmt::Return(vec![Expr::Name(value.clone())])]),
        );
        let mut block = Block(vec![Stmt::While {
            cond: Expr::True,
            body: Block(vec![
                Stmt::Local {
                    names: vec![value.clone()],
                    attribs: Vec::new(),
                    values: vec![Expr::Number(7.0)],
                },
                Stmt::Call(Expr::Call {
                    func: Box::new(Expr::Global("consume".into())),
                    args: vec![Expr::Function(closure)],
                    method: None,
                }),
            ]),
        }]);

        normalize_function_block(&mut block);

        let Stmt::While { body, .. } = &block.0[0] else {
            panic!("expected loop");
        };
        assert!(matches!(
            &body.0[0],
            Stmt::Local { names, .. } if names == &vec![value]
        ));
    }
}
