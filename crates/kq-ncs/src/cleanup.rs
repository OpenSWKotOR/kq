//! CleanupPass (research-dencs §8): decl+assign merge, struct-field collapse,
//! unwrap a lone code block. Dangling expressions are a BuildError, not wrapped.

use std::collections::HashMap;

use crate::ast::{Block, ElseArm, Expr, Stmt};
use crate::stack::{Var, VarId};

pub type VarTable = HashMap<VarId, Var>;

pub fn cleanup(block: &mut Block, vars: &mut VarTable) {
    unwrap_lone_code_block(block);
    walk(block, vars);
    if matches!(block.stmts.last(), Some(Stmt::Return(None))) {
        block.stmts.pop();
    }
}

fn unwrap_lone_code_block(block: &mut Block) {
    if matches!(block.stmts.as_slice(), [Stmt::Block(_)]) {
        let Stmt::Block(inner) = block.stmts.remove(0) else {
            return;
        };
        *block = inner;
    }
}

fn walk(block: &mut Block, vars: &mut VarTable) {
    merge_decl_assign(&mut block.stmts);
    collapse_struct_fields(&mut block.stmts, vars);
    for stmt in &mut block.stmts {
        match stmt {
            Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                walk(then_body, vars);
                if let Some(arm) = else_body {
                    walk_else(arm, vars);
                }
            }
            Stmt::While { body, .. } | Stmt::DoWhile { body, .. } | Stmt::Block(body) => {
                walk(body, vars);
            }
            Stmt::Switch { cases, .. } => {
                for case in cases {
                    walk(&mut case.body, vars);
                }
            }
            _ => {}
        }
    }
}

fn walk_else(arm: &mut ElseArm, vars: &mut VarTable) {
    match arm {
        ElseArm::Else(body) => walk(body, vars),
        ElseArm::ElseIf {
            then_body,
            else_body,
            ..
        } => {
            walk(then_body, vars);
            if let Some(next) = else_body {
                walk_else(next, vars);
            }
        }
    }
}

fn merge_decl_assign(stmts: &mut Vec<Stmt>) {
    let mut i = 0;
    while i + 1 < stmts.len() {
        let merged = match (&stmts[i], &stmts[i + 1]) {
            (
                Stmt::VarDecl {
                    var,
                    ty,
                    init: None,
                },
                Stmt::Expr(Expr::Assign { lhs, rhs }),
            ) if matches!(lhs.as_ref(), Expr::Var(id) if id == var) => Some(Stmt::VarDecl {
                var: *var,
                ty: *ty,
                init: Some(rhs.as_ref().clone()),
            }),
            _ => None,
        };
        if let Some(decl) = merged {
            stmts[i] = decl;
            stmts.remove(i + 1);
        }
        i += 1;
    }
}

fn collapse_struct_fields(stmts: &mut Vec<Stmt>, vars: &VarTable) {
    let mut i = 0;
    while i < stmts.len() {
        let Some(field) = decl_var(&stmts[i]) else {
            i += 1;
            continue;
        };
        let Some(parent) = vars.get(&field).and_then(|v| v.parent_struct) else {
            i += 1;
            continue;
        };
        let mut j = i + 1;
        while j < stmts.len() {
            let Some(next) = decl_var(&stmts[j]) else {
                break;
            };
            if vars.get(&next).and_then(|v| v.parent_struct) != Some(parent) {
                break;
            }
            j += 1;
        }
        let ty = vars
            .get(&parent)
            .map(|v| v.ty)
            .unwrap_or(crate::ty::Ty::Unknown);
        stmts.splice(
            i..j,
            [Stmt::VarDecl {
                var: parent,
                ty,
                init: None,
            }],
        );
        i += 1;
    }
}

fn decl_var(stmt: &Stmt) -> Option<VarId> {
    match stmt {
        Stmt::VarDecl { var, .. } => Some(*var),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Expr, Stmt};
    use crate::stack::{Const, VarKind};
    use crate::ty::{StructId, Ty};

    fn local(ty: Ty, parent: Option<VarId>) -> Var {
        Var {
            ty,
            name: None,
            kind: VarKind::Local,
            assigned: false,
            on_stack: 0,
            parent_struct: parent,
        }
    }

    #[test]
    fn cleanup_merges_decl_assign() {
        let mut block = Block {
            stmts: vec![
                Stmt::VarDecl {
                    var: VarId(0),
                    ty: Ty::Int,
                    init: None,
                },
                Stmt::Expr(Expr::Assign {
                    lhs: Box::new(Expr::Var(VarId(0))),
                    rhs: Box::new(Expr::Const(Const::Int(1))),
                }),
            ],
        };
        let mut vars = VarTable::new();
        cleanup(&mut block, &mut vars);
        assert_eq!(
            block.stmts,
            vec![Stmt::VarDecl {
                var: VarId(0),
                ty: Ty::Int,
                init: Some(Expr::Const(Const::Int(1))),
            }]
        );
    }

    #[test]
    fn cleanup_collapses_struct_fields_only_with_vartable() {
        let field_a = VarId(0);
        let field_b = VarId(1);
        let parent = VarId(10);
        let field_decls = vec![
            Stmt::VarDecl {
                var: field_a,
                ty: Ty::Int,
                init: None,
            },
            Stmt::VarDecl {
                var: field_b,
                ty: Ty::Int,
                init: None,
            },
        ];
        let mut empty_block = Block {
            stmts: field_decls.clone(),
        };
        let mut empty_vars = VarTable::new();
        cleanup(&mut empty_block, &mut empty_vars);
        assert_eq!(
            empty_block.stmts, field_decls,
            "empty VarTable cannot collapse field decls"
        );

        let mut vars = VarTable::new();
        vars.insert(field_a, local(Ty::Int, Some(parent)));
        vars.insert(field_b, local(Ty::Int, Some(parent)));
        vars.insert(parent, local(Ty::Struct(StructId(0)), None));
        let mut filled_block = Block { stmts: field_decls };
        cleanup(&mut filled_block, &mut vars);
        assert_eq!(
            filled_block.stmts,
            vec![Stmt::VarDecl {
                var: parent,
                ty: Ty::Struct(StructId(0)),
                init: None,
            }]
        );
    }
}
