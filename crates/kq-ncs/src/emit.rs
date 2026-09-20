//! NSS emission (research-dencs §9).
//!
//! Float constants: DeNCS widens each CONSTF f32 to f64 and formats with
//! `DecimalFormat("0.0##############")`, which prints extra digits
//! (`1.100000023841858`). We emit the shortest f32 Display that always
//! contains `.` and never uses exponent (`1.1`, `1.0`, `0.00001`).

use std::collections::HashMap;

use crate::ast::{BinOp, Block, ElseArm, Expr, Stmt, UnaryOp};
use crate::build::GLOBAL_BASE;
use crate::globals::GlobalTable;
use crate::names::NameGen;
use crate::protos::SubInfo;
use crate::stack::{Const, Var, VarId, VarKind};
use crate::ty::{StructTable, Ty};
use crate::SubId;

pub fn format_float(v: f32) -> String {
    if v.is_nan() {
        return "0.0".into();
    }
    if !v.is_finite() {
        return if v.is_sign_positive() {
            "999999999999999.0".into()
        } else {
            "-999999999999999.0".into()
        };
    }
    if v == 0.0 {
        return "0.0".into();
    }
    let raw = v.to_string();
    ensure_dot(&expand_no_exponent(&raw))
}

fn expand_no_exponent(s: &str) -> String {
    let (neg, body) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s),
    };
    let lower = body.to_ascii_lowercase();
    let expanded = if let Some((mant, exp)) = lower.split_once('e') {
        let exp: i32 = exp.parse().unwrap_or(0);
        expand_exp(mant, exp)
    } else {
        body.to_string()
    };
    if neg {
        format!("-{expanded}")
    } else {
        expanded
    }
}

fn expand_exp(mant: &str, exp: i32) -> String {
    let (digits, int_digits) = if let Some((a, b)) = mant.split_once('.') {
        (format!("{a}{b}"), a.len() as i32)
    } else {
        (mant.to_string(), mant.len() as i32)
    };
    let point = int_digits + exp;
    if point <= 0 {
        let mut s = String::from("0.");
        s.push_str(&"0".repeat((-point) as usize));
        let frac = digits.trim_end_matches('0');
        s.push_str(if frac.is_empty() { "0" } else { frac });
        s
    } else if (point as usize) >= digits.len() {
        let mut s = digits;
        s.push_str(&"0".repeat(point as usize - s.len()));
        s.push_str(".0");
        s
    } else {
        let (whole, frac) = digits.split_at(point as usize);
        let frac = frac.trim_end_matches('0');
        format!("{whole}.{}", if frac.is_empty() { "0" } else { frac })
    }
}

fn ensure_dot(s: &str) -> String {
    if s.contains('.') {
        s.to_string()
    } else {
        format!("{s}.0")
    }
}

pub fn emit_program(
    structs: &StructTable,
    globals: Option<&GlobalTable>,
    protos: &[(SubId, SubInfo, Block)],
) -> String {
    let mut out = String::new();

    let struct_decls: Vec<_> = structs.decls().collect();
    if !struct_decls.is_empty() {
        for def in &struct_decls {
            out.push_str("struct ");
            out.push_str(&def.name);
            out.push_str(" {\n");
            for (ty, field) in def.elements.iter().zip(&def.field_names) {
                out.push('\t');
                out.push_str(&ty_name(ty, structs));
                out.push(' ');
                out.push_str(field);
                out.push_str(";\n");
            }
            out.push_str("};\n");
        }
        out.push('\n');
    }

    if let Some(globals) = globals {
        out.push_str("// Globals\n");
        let locals = HashMap::new();
        for g in &globals.vars {
            out.push('\t');
            out.push_str(&ty_name(&g.ty, structs));
            out.push(' ');
            out.push_str(&g.name);
            if let Some(init) = &g.init {
                out.push_str(" = ");
                emit_expr(&mut out, init, structs, Some(globals), &locals, 0);
            }
            out.push_str(";\n");
        }
        out.push('\n');
    }

    let users: Vec<_> = protos
        .iter()
        .filter(|(id, _, _)| matches!(id, SubId::User(_)))
        .collect();
    if !users.is_empty() {
        out.push_str("// Prototypes\n");
        for (id, info, _) in &users {
            emit_signature(&mut out, *id, info, structs);
            out.push_str(";\n");
        }
        out.push('\n');
    }

    let bodies: Vec<_> = protos
        .iter()
        .filter(|(id, _, _)| matches!(id, SubId::User(_) | SubId::Main))
        .collect();
    for (i, (id, info, block)) in bodies.iter().enumerate() {
        if i != 0 {
            out.push('\n');
        }
        emit_function(&mut out, *id, info, block, structs, globals);
    }
    out
}

fn emit_signature(out: &mut String, id: SubId, info: &SubInfo, structs: &StructTable) {
    let (ret, name) = match id {
        SubId::Main if info.ret != Ty::Void => {
            ("int".to_string(), "StartingConditional".to_string())
        }
        SubId::Main => ("void".to_string(), "main".to_string()),
        SubId::User(n) => (ty_name(&info.ret, structs), format!("sub{n}")),
        _ => return,
    };
    out.push_str(&ret);
    out.push(' ');
    out.push_str(&name);
    out.push('(');
    let mut ng = NameGen::default();
    for (i, ty) in info.params.iter().enumerate() {
        if i != 0 {
            out.push_str(", ");
        }
        out.push_str(&ty_name(ty, structs));
        out.push(' ');
        out.push_str(&ng.param(ty, i + 1));
    }
    out.push(')');
}

fn emit_function(
    out: &mut String,
    id: SubId,
    info: &SubInfo,
    block: &Block,
    structs: &StructTable,
    globals: Option<&GlobalTable>,
) {
    emit_signature(out, id, info, structs);
    out.push_str(" {\n");
    let locals = name_locals(block);
    emit_block(out, block, structs, globals, &locals, 1);
    out.push_str("}\n");
}

fn name_locals(block: &Block) -> HashMap<VarId, String> {
    let mut ng = NameGen::default();
    let mut names = HashMap::new();
    name_block(block, &mut ng, &mut names);
    names
}

fn name_block(block: &Block, ng: &mut NameGen, names: &mut HashMap<VarId, String>) {
    for stmt in &block.stmts {
        name_stmt(stmt, ng, names);
    }
}

fn name_stmt(stmt: &Stmt, ng: &mut NameGen, names: &mut HashMap<VarId, String>) {
    match stmt {
        Stmt::VarDecl { var, ty, init } => {
            let mut var_rec = Var {
                ty: *ty,
                name: Some(ng.generic(ty)),
                kind: VarKind::Local,
                assigned: false,
                on_stack: 0,
                parent_struct: None,
            };
            if let Some(Expr::CallAction { name, args }) = init {
                ng.apply_action_hint(&mut var_rec, name, args);
            }
            names.insert(*var, var_rec.name.unwrap());
        }
        Stmt::If {
            then_body,
            else_body,
            ..
        } => {
            name_block(then_body, ng, names);
            if let Some(arm) = else_body {
                name_else(arm, ng, names);
            }
        }
        Stmt::While { body, .. } | Stmt::DoWhile { body, .. } | Stmt::Block(body) => {
            name_block(body, ng, names);
        }
        Stmt::Switch { cases, .. } => {
            for case in cases {
                name_block(&case.body, ng, names);
            }
        }
        _ => {}
    }
}

fn name_else(arm: &ElseArm, ng: &mut NameGen, names: &mut HashMap<VarId, String>) {
    match arm {
        ElseArm::Else(body) => name_block(body, ng, names),
        ElseArm::ElseIf {
            then_body,
            else_body,
            ..
        } => {
            name_block(then_body, ng, names);
            if let Some(next) = else_body {
                name_else(next, ng, names);
            }
        }
    }
}

fn emit_block(
    out: &mut String,
    block: &Block,
    structs: &StructTable,
    globals: Option<&GlobalTable>,
    locals: &HashMap<VarId, String>,
    indent: usize,
) {
    for stmt in &block.stmts {
        emit_stmt(out, stmt, structs, globals, locals, indent);
    }
}

fn emit_stmt(
    out: &mut String,
    stmt: &Stmt,
    structs: &StructTable,
    globals: Option<&GlobalTable>,
    locals: &HashMap<VarId, String>,
    indent: usize,
) {
    let tabs = "\t".repeat(indent);
    match stmt {
        Stmt::VarDecl { var, ty, init } => {
            out.push_str(&tabs);
            out.push_str(&ty_name(ty, structs));
            out.push(' ');
            out.push_str(&var_name(*var, globals, locals));
            if let Some(expr) = init {
                out.push_str(" = ");
                emit_expr(out, expr, structs, globals, locals, 0);
            }
            out.push_str(";\n");
        }
        Stmt::Expr(expr) => {
            out.push_str(&tabs);
            emit_expr(out, expr, structs, globals, locals, 0);
            out.push_str(";\n");
        }
        Stmt::Return(expr) => {
            out.push_str(&tabs);
            out.push_str("return");
            if let Some(expr) = expr {
                out.push(' ');
                emit_expr(out, expr, structs, globals, locals, 0);
            }
            out.push_str(";\n");
        }
        Stmt::If {
            cond,
            then_body,
            else_body,
        } => {
            out.push_str(&tabs);
            out.push_str("if (");
            emit_expr(out, cond, structs, globals, locals, 0);
            out.push_str(") {\n");
            emit_block(out, then_body, structs, globals, locals, indent + 1);
            out.push_str(&tabs);
            out.push('}');
            if let Some(arm) = else_body {
                emit_else(out, arm, structs, globals, locals, indent);
            } else {
                out.push('\n');
            }
        }
        Stmt::Block(body) => {
            out.push_str(&tabs);
            out.push_str("{\n");
            emit_block(out, body, structs, globals, locals, indent + 1);
            out.push_str(&tabs);
            out.push_str("}\n");
        }
        Stmt::Break => out.push_str(&format!("{tabs}break;\n")),
        Stmt::Continue => out.push_str(&format!("{tabs}continue;\n")),
        Stmt::Comment(text) => out.push_str(&format!("{tabs}/* {text} */\n")),
        Stmt::While { cond, body } => {
            out.push_str(&tabs);
            out.push_str("while (");
            emit_expr(out, cond, structs, globals, locals, 0);
            out.push_str(") {\n");
            emit_block(out, body, structs, globals, locals, indent + 1);
            out.push_str(&tabs);
            out.push_str("}\n");
        }
        Stmt::DoWhile { body, cond } => {
            out.push_str(&tabs);
            out.push_str("do {\n");
            emit_block(out, body, structs, globals, locals, indent + 1);
            out.push_str(&tabs);
            out.push_str("} while (");
            emit_expr(out, cond, structs, globals, locals, 0);
            out.push_str(");\n");
        }
        Stmt::Switch { sel, cases } => {
            out.push_str(&tabs);
            out.push_str("switch(");
            emit_expr(out, sel, structs, globals, locals, 0);
            out.push_str(") {\n");
            for case in cases {
                out.push_str(&"\t".repeat(indent + 1));
                match &case.value {
                    Some(c) => {
                        out.push_str("case ");
                        emit_const(out, c);
                        out.push_str(":\n");
                    }
                    None => out.push_str("default:\n"),
                }
                emit_block(out, &case.body, structs, globals, locals, indent + 2);
            }
            out.push_str(&tabs);
            out.push_str("}\n");
        }
    }
}

fn emit_else(
    out: &mut String,
    arm: &ElseArm,
    structs: &StructTable,
    globals: Option<&GlobalTable>,
    locals: &HashMap<VarId, String>,
    indent: usize,
) {
    match arm {
        ElseArm::Else(body) => {
            out.push_str(" else {\n");
            emit_block(out, body, structs, globals, locals, indent + 1);
            out.push_str(&"\t".repeat(indent));
            out.push_str("}\n");
        }
        ElseArm::ElseIf {
            cond,
            then_body,
            else_body,
        } => {
            out.push_str(" else if (");
            emit_expr(out, cond, structs, globals, locals, 0);
            out.push_str(") {\n");
            emit_block(out, then_body, structs, globals, locals, indent + 1);
            out.push_str(&"\t".repeat(indent));
            out.push('}');
            if let Some(next) = else_body {
                emit_else(out, next, structs, globals, locals, indent);
            } else {
                out.push('\n');
            }
        }
    }
}

fn emit_const(out: &mut String, c: &Const) {
    match c {
        Const::Int(v) => out.push_str(&v.to_string()),
        Const::Object(0) => out.push_str("OBJECT_SELF"),
        Const::Object(1) => out.push_str("OBJECT_INVALID"),
        Const::Object(v) => out.push_str(&v.to_string()),
        Const::Float(v) => out.push_str(&format_float(*v)),
        Const::Str(v) => {
            out.push('"');
            for ch in v.chars() {
                match ch {
                    '\\' => out.push_str("\\\\"),
                    '"' => out.push_str("\\\""),
                    _ => out.push(ch),
                }
            }
            out.push('"');
        }
    }
}

fn emit_expr(
    out: &mut String,
    expr: &Expr,
    structs: &StructTable,
    globals: Option<&GlobalTable>,
    locals: &HashMap<VarId, String>,
    parent_prec: u8,
) {
    match expr {
        Expr::Const(c) => emit_const(out, c),
        Expr::Var(id) => out.push_str(&var_name(*id, globals, locals)),
        Expr::Field { base, field } => {
            out.push_str(&var_name(*base, globals, locals));
            out.push('.');
            out.push_str(field);
        }
        Expr::Unary { op, expr } => {
            out.push_str(match op {
                UnaryOp::Neg => "-",
                UnaryOp::BitNot => "~",
                UnaryOp::Not => "!",
                UnaryOp::PreInc => "++",
                UnaryOp::PreDec => "--",
                UnaryOp::PostInc | UnaryOp::PostDec => "",
            });
            emit_expr(out, expr, structs, globals, locals, 13);
            if matches!(op, UnaryOp::PostInc | UnaryOp::PostDec) {
                out.push_str(if matches!(op, UnaryOp::PostInc) {
                    "++"
                } else {
                    "--"
                });
            }
        }
        Expr::Binary { op, lhs, rhs } => {
            let prec = bin_prec(*op);
            let parens = prec < parent_prec;
            if parens {
                out.push('(');
            }
            emit_expr(out, lhs, structs, globals, locals, prec);
            out.push(' ');
            out.push_str(bin_text(*op));
            out.push(' ');
            emit_expr(out, rhs, structs, globals, locals, prec + 1);
            if parens {
                out.push(')');
            }
        }
        Expr::Assign { lhs, rhs } => {
            emit_expr(out, lhs, structs, globals, locals, 1);
            out.push_str(" = ");
            emit_expr(out, rhs, structs, globals, locals, 1);
        }
        Expr::CallAction { name, args } => emit_call(out, name, args, structs, globals, locals),
        Expr::CallSub { id, args } => {
            emit_call(out, &format!("sub{id}"), args, structs, globals, locals)
        }
        Expr::Grouped(inner) => {
            out.push('(');
            emit_expr(out, inner, structs, globals, locals, 0);
            out.push(')');
        }
        Expr::VectorLit(components) => {
            out.push('[');
            for (i, component) in components.iter().enumerate() {
                if i != 0 {
                    out.push_str(", ");
                }
                emit_expr(out, component, structs, globals, locals, 0);
            }
            out.push(']');
        }
        Expr::Deferred(inner) => match inner.as_ref() {
            Stmt::Expr(e) => emit_expr(out, e, structs, globals, locals, 0),
            other => emit_stmt(out, other, structs, globals, locals, 0),
        },
    }
}

fn emit_call(
    out: &mut String,
    name: &str,
    args: &[Expr],
    structs: &StructTable,
    globals: Option<&GlobalTable>,
    locals: &HashMap<VarId, String>,
) {
    out.push_str(name);
    out.push('(');
    for (i, arg) in args.iter().enumerate() {
        if i != 0 {
            out.push_str(", ");
        }
        emit_expr(out, arg, structs, globals, locals, 0);
    }
    out.push(')');
}

fn var_name(id: VarId, globals: Option<&GlobalTable>, locals: &HashMap<VarId, String>) -> String {
    if id.0 >= GLOBAL_BASE {
        globals
            .and_then(|g| g.vars.get((id.0 - GLOBAL_BASE) as usize))
            .map(|v| v.name.clone())
            .unwrap_or_else(|| format!("global{}", id.0 - GLOBAL_BASE + 1))
    } else {
        locals
            .get(&id)
            .cloned()
            .unwrap_or_else(|| format!("int{}", id.0 + 1))
    }
}

fn ty_name(ty: &Ty, structs: &StructTable) -> String {
    match ty {
        Ty::Unknown => "int".into(),
        other => other.decl_name(structs),
    }
}

fn bin_prec(op: BinOp) -> u8 {
    match op {
        BinOp::LogOr => 2,
        BinOp::LogAnd => 3,
        BinOp::BitOr => 4,
        BinOp::BitXor => 5,
        BinOp::BitAnd => 6,
        BinOp::Eq | BinOp::Ne => 7,
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => 8,
        BinOp::Shl | BinOp::Shr => 9,
        BinOp::Add | BinOp::Sub => 10,
        BinOp::Mul | BinOp::Div | BinOp::Mod => 11,
    }
}

fn bin_text(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Mod => "%",
        BinOp::Shl => "<<",
        BinOp::Shr => ">>",
        BinOp::Eq => "==",
        BinOp::Ne => "!=",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
        BinOp::LogAnd => "&&",
        BinOp::LogOr => "||",
        BinOp::BitOr => "|",
        BinOp::BitXor => "^",
        BinOp::BitAnd => "&",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::globals::GlobalVar;
    use crate::split::SubKind;
    use std::ops::Range;

    fn dummy_info(kind: SubKind, ret: Ty, params: Vec<Ty>) -> SubInfo {
        SubInfo {
            range: Range { start: 0, end: 0 },
            start_pos: 0,
            kind,
            ret,
            ret_slots: if ret == Ty::Void { 0 } else { 1 },
            ret_depth: 0,
            param_count: params.len(),
            params,
            params_typed: true,
        }
    }

    #[test]
    fn emit_float_shortest() {
        assert_eq!(format_float(1.1f32), "1.1");
        assert_eq!(format_float(1.0f32), "1.0");
        assert_eq!(format_float(1e-5f32), "0.00001");
    }

    #[test]
    fn emit_globals_and_prototypes_layout() {
        let structs = StructTable::new();
        let globals = GlobalTable {
            vars: vec![GlobalVar {
                ty: Ty::Int,
                name: "intGLOB_1".into(),
                init: None,
            }],
        };
        let protos = [
            (
                SubId::User(1),
                dummy_info(SubKind::User(1), Ty::Void, vec![]),
                Block::default(),
            ),
            (
                SubId::Main,
                dummy_info(SubKind::Main, Ty::Void, vec![]),
                Block::default(),
            ),
        ];
        let src = emit_program(&structs, Some(&globals), &protos);
        assert!(src.starts_with("// Globals\n\tint intGLOB_1"));
        assert!(src.contains("// Prototypes\nvoid sub1("));
        assert!(src.ends_with("void main() {\n}\n") || src.contains("void main()"));
    }
}
