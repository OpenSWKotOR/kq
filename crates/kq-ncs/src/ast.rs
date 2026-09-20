//! Statement and expression AST (research-dencs §5–9).

use crate::stack::{Const, VarId};
use crate::ty::Ty;

#[derive(Clone, Debug, PartialEq)]
pub enum Stmt {
    VarDecl {
        var: VarId,
        ty: Ty,
        init: Option<Expr>,
    },
    Expr(Expr),
    Return(Option<Expr>),
    Break,
    Continue,
    If {
        cond: Expr,
        then_body: Block,
        else_body: Option<ElseArm>,
    },
    While {
        cond: Expr,
        body: Block,
    },
    DoWhile {
        body: Block,
        cond: Expr,
    },
    Switch {
        sel: Expr,
        cases: Vec<SwitchCase>,
    },
    Block(Block),
    Comment(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum ElseArm {
    Else(Block),
    ElseIf {
        cond: Expr,
        then_body: Block,
        else_body: Option<Box<ElseArm>>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Const(Const),
    Var(VarId),
    Field {
        base: VarId,
        field: String,
    },
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
    },
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Assign {
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    CallAction {
        name: String,
        args: Vec<Expr>,
    },
    CallSub {
        id: u16,
        args: Vec<Expr>,
    },
    /// Action-typed argument (`STORE_STATE` body).
    Deferred(Box<Stmt>),
    /// Boxed because `[Expr; 3]` is recursive (Expr contains VectorLit).
    VectorLit(Box<[Expr; 3]>),
    Grouped(Box<Expr>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    BitNot,
    Not,
    PreInc,
    PreDec,
    PostInc,
    PostDec,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Shl,
    Shr,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    LogAnd,
    LogOr,
    BitOr,
    BitXor,
    BitAnd,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SwitchCase {
    /// `None` is `default`.
    pub value: Option<Const>,
    pub body: Block,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Block {
    pub stmts: Vec<Stmt>,
}

impl Expr {
    pub fn as_str_const(&self) -> Option<&str> {
        match self {
            Expr::Const(Const::Str(s)) => Some(s),
            _ => None,
        }
    }

    pub fn as_int_const(&self) -> Option<i32> {
        match self {
            Expr::Const(Const::Int(n)) => Some(*n),
            _ => None,
        }
    }
}
