use bstr::BString;

use super::{BinOp, FuncBody, Name, TableField, UnOp};

/// Expressions in the compact decompiler IR.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Nil,
    True,
    False,
    VarArg,
    Number(f64),
    Integer(i64),
    Str(BString),
    Name(Name),
    Global(BString),
    Index {
        obj: Box<Self>,
        key: Box<Self>,
    },
    Field {
        obj: Box<Self>,
        name: Name,
    },
    Call {
        func: Box<Self>,
        args: Vec<Self>,
        method: Option<Name>,
    },
    Function(FuncBody),
    Table(Vec<TableField>),
    Binary {
        op: BinOp,
        lhs: Box<Self>,
        rhs: Box<Self>,
    },
    Unary {
        op: UnOp,
        operand: Box<Self>,
    },
    Paren(Box<Self>),
}
