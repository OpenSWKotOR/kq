//! NCS → NSS decompiler (DeNCS algorithm port).

mod actions;
mod actions_gen;
mod ast;
mod build;
mod cfg;
mod fallback;
mod globals;
mod names;
mod protos;
mod split;
mod stack;
mod ty;

pub use ast::{BinOp, Block, ElseArm, Expr, Stmt, SwitchCase, UnaryOp};
pub use build::{build_sub, BuildError};
pub use cfg::{analyze, BlockEnd, Cfg};
pub use globals::{build_globals, GlobalTable, GlobalVar, GlobalsError};
pub use kq_index::Game;
pub use names::{name_from_action, NameGen};
pub use protos::{infer_prototypes, SubInfo};
pub use split::{split, DeferredRegion, SplitError, SplitProgram, SubKind, SubRange};
pub use stack::{
    stack_offset_to_pos, stack_size_to_pos, Const, CpDownTarget, Entry, LocalStack, StackError,
    Var, VarId, VarKind,
};
pub use ty::{StructDef, StructId, StructTable, Ty};

#[derive(Clone, Debug)]
pub struct Decompiled {
    pub source: String,
    pub complete: bool,
    pub warnings: Vec<Warning>,
    pub subs: Vec<SubReport>,
}

#[derive(Clone, Debug)]
pub struct Warning {
    pub sub: Option<u16>,
    pub pos: Option<u32>,
    pub severity: Severity,
    pub msg: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Info,
    Warn,
    Error,
}

#[derive(Clone, Debug)]
pub struct SubReport {
    pub id: SubId,
    pub start: u32,
    pub end: u32,
    pub status: SubStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SubId {
    Header,
    Globals,
    Main,
    User(u16),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubStatus {
    Ok,
    Fallback(String),
}

/// Never panics. Never returns Err.
pub fn decompile(ncs: &kq_format::ncs::Ncs, game: Game) -> Decompiled {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| decompile_inner(ncs, game)))
        .unwrap_or_else(|_| fallback_decompile(ncs, "decompiler panic"))
}

fn decompile_inner(ncs: &kq_format::ncs::Ncs, game: Game) -> Decompiled {
    use std::collections::HashMap;

    let program = match split(&ncs.instructions) {
        Ok(program) => program,
        Err(err) => return fallback_decompile(ncs, &format!("split failed: {err:?}")),
    };

    let mut cfgs = HashMap::new();
    if let Some(globals) = &program.globals {
        cfgs.insert(
            SubId::Globals,
            analyze(&ncs.instructions, globals, &program.deferred),
        );
    }
    cfgs.insert(
        SubId::Main,
        analyze(&ncs.instructions, &program.main, &program.deferred),
    );
    for user in &program.users {
        if let SubKind::User(id) = user.kind {
            cfgs.insert(
                SubId::User(id),
                analyze(&ncs.instructions, user, &program.deferred),
            );
        }
    }

    let globals = match &program.globals {
        Some(range) => match build_globals(&ncs.instructions, range, game) {
            Ok(table) => table,
            Err(err) => return fallback_decompile(ncs, &format!("globals failed: {err:?}")),
        },
        None => GlobalTable { vars: Vec::new() },
    };
    let (protos, mut warnings) = infer_prototypes(&ncs.instructions, &program, &cfgs, game);

    let mut built = Vec::new();
    let mut reports = Vec::new();
    let mut ranges = vec![(SubId::Main, &program.main)];
    ranges.extend(program.users.iter().filter_map(|range| match range.kind {
        SubKind::User(id) => Some((SubId::User(id), range)),
        _ => None,
    }));
    for (id, range) in ranges {
        let Some(info) = protos.get(&id) else {
            return fallback_decompile(ncs, "missing inferred prototype");
        };
        let Some(cfg) = cfgs.get(&id) else {
            return fallback_decompile(ncs, "missing CFG");
        };
        match build_sub(&ncs.instructions, info, cfg, &globals, &protos, game) {
            Ok(block) => {
                reports.push(SubReport {
                    id,
                    start: range.start_pos,
                    end: range.end_pos,
                    status: SubStatus::Ok,
                });
                built.push((id, info, block));
            }
            Err(err) => {
                return fallback_decompile(ncs, &format!("build failed at {id:?}: {err:?}"));
            }
        }
    }

    let mut source = String::new();
    for (index, (id, info, block)) in built.iter().enumerate() {
        if index != 0 {
            source.push('\n');
        }
        emit_function(
            &mut source,
            *id,
            info,
            block,
            &globals,
            program.conditional_header,
        );
    }
    if source.is_empty() {
        warnings.push(Warning {
            sub: None,
            pos: None,
            severity: Severity::Warn,
            msg: "no functions emitted".into(),
        });
        return fallback_decompile(ncs, "no functions emitted");
    }
    Decompiled {
        source,
        complete: true,
        warnings,
        subs: reports,
    }
}

fn fallback_decompile(ncs: &kq_format::ncs::Ncs, reason: &str) -> Decompiled {
    let disasm = fallback::disasm_lines(ncs);
    let mut indented = String::new();
    for line in disasm.lines() {
        indented.push_str("     ");
        indented.push_str(line);
        indented.push('\n');
    }
    let source =
        format!("void main() {{\n\t/* kq: {reason}.\n\t   Disassembly:\n{indented}\t*/\n}}\n");
    let (start, end) = ncs
        .instructions
        .first()
        .map(|first| {
            let end = ncs
                .instructions
                .last()
                .map(|last| last.offset)
                .unwrap_or(first.offset);
            (first.offset, end)
        })
        .unwrap_or((0, 0));
    Decompiled {
        source,
        complete: false,
        warnings: Vec::new(),
        subs: vec![SubReport {
            id: SubId::Main,
            start,
            end,
            status: SubStatus::Fallback(reason.into()),
        }],
    }
}

fn emit_function(
    out: &mut String,
    id: SubId,
    info: &SubInfo,
    block: &Block,
    globals: &GlobalTable,
    conditional_main: bool,
) {
    let (ret, name) = match id {
        SubId::Main if conditional_main => ("int", "StartingConditional".to_string()),
        SubId::Main => ("void", "main".to_string()),
        SubId::User(n) => (ty_name(&info.ret), format!("sub{n}")),
        _ => return,
    };
    out.push_str(ret);
    out.push(' ');
    out.push_str(&name);
    out.push('(');
    for (i, ty) in info.params.iter().enumerate() {
        if i != 0 {
            out.push_str(", ");
        }
        out.push_str(ty_name(ty));
        out.push(' ');
        out.push_str(&format!("{}Param{}", ty_name(ty), i + 1));
    }
    out.push_str(") {\n");
    emit_block(out, block, globals, 1);
    out.push_str("}\n");
}

fn emit_block(out: &mut String, block: &Block, globals: &GlobalTable, indent: usize) {
    for stmt in &block.stmts {
        emit_stmt(out, stmt, globals, indent);
    }
}

fn emit_stmt(out: &mut String, stmt: &Stmt, globals: &GlobalTable, indent: usize) {
    let tabs = "\t".repeat(indent);
    match stmt {
        Stmt::VarDecl { var, init } => {
            out.push_str(&tabs);
            out.push_str("int ");
            out.push_str(&var_name(*var, globals));
            if let Some(expr) = init {
                out.push_str(" = ");
                emit_expr(out, expr, globals, 0);
            }
            out.push_str(";\n");
        }
        Stmt::Expr(expr) => {
            out.push_str(&tabs);
            emit_expr(out, expr, globals, 0);
            out.push_str(";\n");
        }
        Stmt::Return(expr) => {
            out.push_str(&tabs);
            out.push_str("return");
            if let Some(expr) = expr {
                out.push(' ');
                emit_expr(out, expr, globals, 0);
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
            emit_expr(out, cond, globals, 0);
            out.push_str(") {\n");
            emit_block(out, then_body, globals, indent + 1);
            out.push_str(&tabs);
            out.push('}');
            if let Some(arm) = else_body {
                emit_else(out, arm, globals, indent);
            } else {
                out.push('\n');
            }
        }
        Stmt::Block(body) => emit_block(out, body, globals, indent),
        Stmt::Break => out.push_str(&format!("{tabs}break;\n")),
        Stmt::Continue => out.push_str(&format!("{tabs}continue;\n")),
        Stmt::Comment(text) => out.push_str(&format!("{tabs}/* {text} */\n")),
        Stmt::While { cond, body } => {
            out.push_str(&tabs);
            out.push_str("while (");
            emit_expr(out, cond, globals, 0);
            out.push_str(") {\n");
            emit_block(out, body, globals, indent + 1);
            out.push_str(&tabs);
            out.push_str("}\n");
        }
        Stmt::DoWhile { body, cond } => {
            out.push_str(&tabs);
            out.push_str("do {\n");
            emit_block(out, body, globals, indent + 1);
            out.push_str(&tabs);
            out.push_str("} while (");
            emit_expr(out, cond, globals, 0);
            out.push_str(");\n");
        }
        _ => out.push_str(&format!("{tabs}/* unsupported statement */\n")),
    }
}

fn emit_else(out: &mut String, arm: &ElseArm, globals: &GlobalTable, indent: usize) {
    match arm {
        ElseArm::Else(body) => {
            out.push_str(" else {\n");
            emit_block(out, body, globals, indent + 1);
            out.push_str(&"\t".repeat(indent));
            out.push_str("}\n");
        }
        ElseArm::ElseIf {
            cond,
            then_body,
            else_body,
        } => {
            out.push_str(" else if (");
            emit_expr(out, cond, globals, 0);
            out.push_str(") {\n");
            emit_block(out, then_body, globals, indent + 1);
            out.push_str(&"\t".repeat(indent));
            out.push('}');
            if let Some(next) = else_body {
                emit_else(out, next, globals, indent);
            } else {
                out.push('\n');
            }
        }
    }
}

fn emit_expr(out: &mut String, expr: &Expr, globals: &GlobalTable, parent_prec: u8) {
    match expr {
        Expr::Const(stack::Const::Int(v)) | Expr::Const(stack::Const::Object(v)) => {
            out.push_str(&v.to_string())
        }
        Expr::Const(stack::Const::Float(v)) => out.push_str(&format!("{v:?}")),
        Expr::Const(stack::Const::Str(v)) => {
            out.push('"');
            for c in v.chars() {
                match c {
                    '\\' => out.push_str("\\\\"),
                    '"' => out.push_str("\\\""),
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    _ => out.push(c),
                }
            }
            out.push('"');
        }
        Expr::Var(id) => out.push_str(&var_name(*id, globals)),
        Expr::Unary { op, expr } => {
            out.push_str(match op {
                UnaryOp::Neg => "-",
                UnaryOp::BitNot => "~",
                UnaryOp::Not => "!",
                UnaryOp::PreInc => "++",
                UnaryOp::PreDec => "--",
                UnaryOp::PostInc | UnaryOp::PostDec => "",
            });
            emit_expr(out, expr, globals, 13);
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
            emit_expr(out, lhs, globals, prec);
            out.push(' ');
            out.push_str(bin_text(*op));
            out.push(' ');
            emit_expr(out, rhs, globals, prec + 1);
            if parens {
                out.push(')');
            }
        }
        Expr::Assign { lhs, rhs } => {
            emit_expr(out, lhs, globals, 1);
            out.push_str(" = ");
            emit_expr(out, rhs, globals, 1);
        }
        Expr::CallAction { name, args } => emit_call(out, name, args, globals),
        Expr::CallSub { id, args } => emit_call(out, &format!("sub{id}"), args, globals),
        Expr::Grouped(inner) => {
            out.push('(');
            emit_expr(out, inner, globals, 0);
            out.push(')');
        }
        _ => out.push_str("/* unsupported expression */"),
    }
}

fn emit_call(out: &mut String, name: &str, args: &[Expr], globals: &GlobalTable) {
    out.push_str(name);
    out.push('(');
    for (i, arg) in args.iter().enumerate() {
        if i != 0 {
            out.push_str(", ");
        }
        emit_expr(out, arg, globals, 0);
    }
    out.push(')');
}

fn var_name(id: stack::VarId, globals: &GlobalTable) -> String {
    if id.0 >= build::GLOBAL_BASE {
        globals
            .vars
            .get((id.0 - build::GLOBAL_BASE) as usize)
            .map(|v| v.name.clone())
            .unwrap_or_else(|| format!("global{}", id.0 - build::GLOBAL_BASE + 1))
    } else {
        format!("int{}", id.0 + 1)
    }
}

fn ty_name(ty: &Ty) -> &'static str {
    match ty {
        Ty::Void => "void",
        Ty::Float => "float",
        Ty::Str => "string",
        Ty::Object => "object",
        Ty::Effect => "effect",
        Ty::Event => "event",
        Ty::Location => "location",
        Ty::Talent => "talent",
        Ty::Vector => "vector",
        Ty::Action => "action",
        Ty::Struct(_) => "struct",
        Ty::Unknown | Ty::Int => "int",
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

pub fn disasm_comment(ncs: &kq_format::ncs::Ncs) -> String {
    fallback::disasm_lines(ncs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kq_format::ncs::{self, Ncs};
    use std::path::Path;

    fn minimal_main_ncs() -> Ncs {
        // NCS V1.0 + magic + size, then: JSR ->21, RETN, RETN (empty main)
        let mut data = b"NCS V1.0".to_vec();
        data.push(0x42);
        let body = [
            0x1E, 0x00, 0x00, 0x00, 0x00, 0x08, // JSR +8 → offset 21
            0x20, 0x00, // RETN header
            0x20, 0x00, // RETN main
        ];
        let size = (13 + body.len()) as u32;
        data.extend_from_slice(&size.to_be_bytes());
        data.extend_from_slice(&body);
        ncs::read(&data, Path::new("t.ncs")).unwrap()
    }

    #[test]
    fn decompile_never_empty_and_never_panics() {
        let d = decompile(&minimal_main_ncs(), Game::K1);
        assert!(!d.source.is_empty());
        assert!(d.source.contains("/*") || d.source.contains("void") || d.source.contains("RETN"));
    }
}
