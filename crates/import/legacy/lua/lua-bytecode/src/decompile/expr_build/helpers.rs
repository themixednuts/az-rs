use super::{BString, Expr, Name, SsaOp, SsaRef, ast, ir, is_valid_identifier};

pub const fn is_inlineable_def(op: &SsaOp) -> bool {
    matches!(
        op,
        SsaOp::Move { .. }
            | SsaOp::LoadK { .. }
            | SsaOp::LoadLiteral { .. }
            | SsaOp::LoadBool { .. }
            | SsaOp::LoadNil { .. }
            | SsaOp::GetUpval { .. }
            | SsaOp::GetGlobal { .. }
            | SsaOp::GetTable { .. }
            | SsaOp::SelfOp { .. }
            | SsaOp::BinOp { .. }
            | SsaOp::UnOp { .. }
            | SsaOp::Concat { .. }
            | SsaOp::Call { .. }
            | SsaOp::Closure { .. }
            | SsaOp::VarArg { .. }
    )
}

pub(super) const fn is_pure_def(op: &SsaOp) -> bool {
    matches!(
        op,
        SsaOp::Move { .. }
            | SsaOp::LoadK { .. }
            | SsaOp::LoadLiteral { .. }
            | SsaOp::LoadBool { .. }
            | SsaOp::LoadNil { .. }
            | SsaOp::Closure { .. }
            | SsaOp::VarArg { .. }
    )
}

pub(super) const fn constructor_mutation_table(op: &SsaOp) -> Option<SsaRef> {
    match op {
        SsaOp::SetTable { table, .. } | SsaOp::SetList { table, .. } => Some(*table),
        _ => None,
    }
}

pub(super) fn direct_eval_order_refs(op: &SsaOp) -> Vec<SsaRef> {
    match op {
        SsaOp::Move { src }
        | SsaOp::UnOp { value: src, .. }
        | SsaOp::SetGlobal { src, .. }
        | SsaOp::SetUpval { src, .. } => vec![*src],
        SsaOp::GetTable { table, key } | SsaOp::SelfOp { table, key, .. } => vec![*table, *key],
        SsaOp::SetTable { table, key, value } => vec![*table, *key, *value],
        SsaOp::BinOp { left, right, .. } => vec![*left, *right],
        SsaOp::Branch { a, b, .. } => vec![*a, *b],
        SsaOp::Call { func, args, .. } | SsaOp::TailCall { func, args, .. } => {
            let mut refs = Vec::with_capacity(args.len() + 1);
            refs.push(*func);
            refs.extend(args.iter().copied());
            refs
        }
        SsaOp::Return { values, .. } | SsaOp::SetList { values, .. } => values.clone(),
        SsaOp::Concat { operands } | SsaOp::Phi { operands, .. } => operands.clone(),
        SsaOp::Nop
        | SsaOp::LoadK { .. }
        | SsaOp::LoadLiteral { .. }
        | SsaOp::LoadBool { .. }
        | SsaOp::LoadNil { .. }
        | SsaOp::GetUpval { .. }
        | SsaOp::GetGlobal { .. }
        | SsaOp::NewTable { .. }
        | SsaOp::Jump { .. }
        | SsaOp::ForPrep { .. }
        | SsaOp::ForLoop { .. }
        | SsaOp::TForLoop { .. }
        | SsaOp::Close { .. }
        | SsaOp::Closure { .. }
        | SsaOp::VarArg { .. } => Vec::new(),
    }
}

pub(super) const fn map_bin_op(op: ir::BinOp) -> ast::BinOp {
    match op {
        ir::BinOp::Add => ast::BinOp::Add,
        ir::BinOp::Sub => ast::BinOp::Sub,
        ir::BinOp::Mul => ast::BinOp::Mul,
        ir::BinOp::Div => ast::BinOp::Div,
        ir::BinOp::Mod => ast::BinOp::Mod,
        ir::BinOp::Pow => ast::BinOp::Pow,
        ir::BinOp::IDiv => ast::BinOp::IDiv,
        ir::BinOp::BAnd => ast::BinOp::BAnd,
        ir::BinOp::BOr => ast::BinOp::BOr,
        ir::BinOp::BXor => ast::BinOp::BXor,
        ir::BinOp::Shl => ast::BinOp::Shl,
        ir::BinOp::Shr => ast::BinOp::Shr,
    }
}

pub(super) const fn map_un_op(op: ir::UnOp) -> ast::UnOp {
    match op {
        ir::UnOp::Neg => ast::UnOp::Neg,
        ir::UnOp::Not => ast::UnOp::Not,
        ir::UnOp::Len => ast::UnOp::Len,
        ir::UnOp::BNot => ast::UnOp::BNot,
    }
}

pub fn index_expr(obj: Expr, key: Expr) -> Expr {
    if let Some(name) = ident_from_string_expr(&key) {
        Expr::Field {
            obj: Box::new(obj),
            name,
        }
    } else {
        Expr::Index {
            obj: Box::new(obj),
            key: Box::new(key),
        }
    }
}

pub fn global_expr_from_name(name: BString) -> Expr {
    if is_valid_identifier(&name) {
        Expr::Global(name)
    } else {
        Expr::Index {
            obj: Box::new(Expr::Global(BString::from("_G"))),
            key: Box::new(Expr::Str(name)),
        }
    }
}

pub fn ident_from_string_expr(expr: &Expr) -> Option<Name> {
    let Expr::Str(bytes) = expr else {
        return None;
    };
    is_valid_identifier(bytes).then(|| Name::new(bytes.clone()))
}
