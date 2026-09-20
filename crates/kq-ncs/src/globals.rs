//! Globals sub: `RSADDx` before SAVEBP/JSR → named `*GLOB_N` (DoGlobalVars).

use kq_format::ncs::{Arg, Instruction};

use crate::ast::Expr;
use crate::names::NameGen;
use crate::split::SubRange;
use crate::stack::Const;
use crate::ty::Ty;
use crate::Game;

#[derive(Clone, Debug, PartialEq)]
pub struct GlobalTable {
    /// Declaration order: `vars[0]` is first declared (`intGLOB_1`).
    /// DeNCS stack pos 1 is the last declared (top after SAVEBP).
    pub vars: Vec<GlobalVar>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GlobalVar {
    pub ty: Ty,
    pub name: String,
    pub init: Option<Expr>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GlobalsError {
    /// Range has no `RSADDx` before SAVEBP/JSR/RESTOREBP.
    NoRecoverableGlobals,
}

/// Recover typed global declarations from the SAVEBP/RESTOREBP globals sub.
///
/// Simple `RSADDx; CONSTx; CPDOWNSP; MOVSP` inits become `Some(Expr::Const)`.
/// Failed init recovery still yields typed names with `init: None`.
/// `Err` only when no `RSADDx` is recoverable at all.
pub fn build_globals(
    ins: &[Instruction],
    globals: &SubRange,
    game: Game,
) -> Result<GlobalTable, GlobalsError> {
    let _ = game;
    let slice = match ins.get(globals.range.clone()) {
        Some(s) => s,
        None => return Err(GlobalsError::NoRecoverableGlobals),
    };

    let mut names = NameGen::default();
    let mut vars = Vec::new();

    for (i, inst) in slice.iter().enumerate() {
        if freeze(inst.op) {
            break;
        }
        let Some(ty) = ty_from_rsadd(inst.op) else {
            continue;
        };
        let name = names.global(&ty);
        let init = simple_const_init(&slice[i..], ty);
        vars.push(GlobalVar { ty, name, init });
    }

    if vars.is_empty() {
        return Err(GlobalsError::NoRecoverableGlobals);
    }
    Ok(GlobalTable { vars })
}

fn freeze(op: &str) -> bool {
    matches!(op, "SAVEBP" | "RESTOREBP" | "JSR")
}

fn ty_from_rsadd(op: &str) -> Option<Ty> {
    Some(match op {
        "RSADDI" => Ty::Int,
        "RSADDF" => Ty::Float,
        "RSADDS" => Ty::Str,
        "RSADDO" => Ty::Object,
        "RSADDEFF" => Ty::Effect,
        "RSADDEVT" => Ty::Event,
        "RSADDLOC" => Ty::Location,
        "RSADDTAL" => Ty::Talent,
        _ => return None,
    })
}

/// `RSADDx; CONSTx; CPDOWNSP -8 4; MOVSP -4` → `Some(Expr::Const)`.
fn simple_const_init(at_rsadd: &[Instruction], ty: Ty) -> Option<Expr> {
    let const_ins = at_rsadd.get(1)?;
    let cpdown = at_rsadd.get(2)?;
    let movsp = at_rsadd.get(3)?;

    let value = const_from_ins(const_ins)?;
    if const_ty(&value) != ty {
        return None;
    }
    if cpdown.op != "CPDOWNSP" || !args_i64(cpdown, &[-8, 4]) {
        return None;
    }
    if movsp.op != "MOVSP" || !args_i64(movsp, &[-4]) {
        return None;
    }
    Some(Expr::Const(value))
}

fn const_from_ins(ins: &Instruction) -> Option<Const> {
    match ins.op {
        "CONSTI" => match ins.args.first() {
            Some(Arg::Int(v)) => Some(Const::Int(*v as i32)),
            _ => None,
        },
        "CONSTF" => match ins.args.first() {
            Some(Arg::Float(v)) => Some(Const::Float(*v as f32)),
            _ => None,
        },
        "CONSTS" => match ins.args.first() {
            Some(Arg::Str(s)) => Some(Const::Str(s.clone())),
            _ => None,
        },
        "CONSTO" => match ins.args.first() {
            Some(Arg::Int(v)) => Some(Const::Object(*v as i32)),
            _ => None,
        },
        _ => None,
    }
}

fn const_ty(c: &Const) -> Ty {
    match c {
        Const::Int(_) => Ty::Int,
        Const::Float(_) => Ty::Float,
        Const::Str(_) => Ty::Str,
        Const::Object(_) => Ty::Object,
    }
}

fn args_i64(ins: &Instruction, expected: &[i64]) -> bool {
    if ins.args.len() != expected.len() {
        return false;
    }
    ins.args.iter().zip(expected).all(|(arg, want)| match arg {
        Arg::Int(v) => *v == *want,
        _ => false,
    })
}
