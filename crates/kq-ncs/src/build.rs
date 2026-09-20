//! Stack-to-AST build pass for expressions and straight-line `if`/`else`.

use std::collections::HashMap;

use kq_format::ncs::{Arg, Instruction};

use crate::actions::action;
use crate::ast::{BinOp, Block, ElseArm, Expr, Stmt, UnaryOp};
use crate::cfg::Cfg;
use crate::globals::GlobalTable;
use crate::stack::{stack_offset_to_pos, stack_size_to_pos, Const, VarId};
use crate::ty::Ty;
use crate::{Game, SubId, SubInfo};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildError {
    StackUnderflow { pos: u32 },
    BadOperand { pos: u32 },
    BadJump { pos: u32 },
    Unsupported { pos: u32, op: String },
}

#[derive(Clone)]
enum Value {
    Expr(Expr),
    Local(VarId),
    Global(usize),
}

impl Value {
    fn expr(&self) -> Expr {
        match self {
            Self::Expr(e) => e.clone(),
            Self::Local(id) => Expr::Var(*id),
            Self::Global(index) => Expr::Var(VarId(GLOBAL_BASE + *index as u32)),
        }
    }
}

pub(crate) const GLOBAL_BASE: u32 = 1_000_000;

struct Builder<'a> {
    ins: &'a [Instruction],
    cfg: &'a Cfg,
    globals: &'a GlobalTable,
    protos: &'a HashMap<SubId, SubInfo>,
    game: Game,
    stack: Vec<Value>,
    next_var: u32,
}

pub fn build_sub(
    ins: &[Instruction],
    sub: &SubInfo,
    cfg: &Cfg,
    globals: &GlobalTable,
    protos: &HashMap<SubId, SubInfo>,
    game: Game,
) -> Result<Block, BuildError> {
    let mut builder = Builder {
        ins,
        cfg,
        globals,
        protos,
        game,
        stack: Vec::new(),
        next_var: 0,
    };
    builder.build_range(sub.range.start, sub.range.end)
}

impl Builder<'_> {
    fn build_range(&mut self, start: usize, end: usize) -> Result<Block, BuildError> {
        let mut block = Block::default();
        let mut i = start;
        while i < end {
            let inst = &self.ins[i];
            if self.cfg.dead.get(i).copied().unwrap_or(false) {
                i += 1;
                continue;
            }
            match inst.op {
                op if op.starts_with("RSADD") => {
                    let _ty = ty_from_rsadd(op).ok_or_else(|| self.unsupported(inst))?;
                    let id = VarId(self.next_var);
                    self.next_var += 1;
                    self.stack.push(Value::Local(id));
                    block.stmts.push(Stmt::VarDecl {
                        var: id,
                        init: None,
                    });
                }
                "CONSTI" => self.push_const(inst, |v| Const::Int(v as i32))?,
                "CONSTF" => match inst.args.first() {
                    Some(Arg::Float(v)) => self
                        .stack
                        .push(Value::Expr(Expr::Const(Const::Float(*v as f32)))),
                    _ => return Err(self.bad_operand(inst)),
                },
                "CONSTS" => match inst.args.first() {
                    Some(Arg::Str(v)) => self
                        .stack
                        .push(Value::Expr(Expr::Const(Const::Str(v.clone())))),
                    _ => return Err(self.bad_operand(inst)),
                },
                "CONSTO" => self.push_const(inst, |v| Const::Object(v as i32))?,
                "CPTOPSP" => self.copy_sp(inst)?,
                "CPTOPBP" => self.copy_bp(inst)?,
                "CPDOWNSP" => self.assign_sp(inst, &mut block)?,
                "CPDOWNBP" => self.assign_bp(inst, &mut block)?,
                "MOVSP" => self.movsp(inst)?,
                "ACTION" => self.call_action(inst, &mut block)?,
                "JSR" => self.call_sub(inst, &mut block)?,
                op if binary_op(op).is_some() => self.binary(inst, binary_op(op).unwrap())?,
                op if unary_op(op).is_some() => self.unary(inst, unary_op(op).unwrap())?,
                "JZ" | "JNZ"
                    if self.cfg.and_guards.contains(&i)
                        || self.cfg.log_or_extra_jz.contains(&i) =>
                {
                    self.pop(inst)?;
                }
                "JZ" | "JNZ" => {
                    let mut cond = self.pop(inst)?.expr();
                    if inst.op == "JNZ" {
                        cond = Expr::Unary {
                            op: UnaryOp::Not,
                            expr: Box::new(cond),
                        };
                    }
                    let target = jump_index(self.cfg, inst)?;
                    if target <= i || target > end {
                        return Err(self.unsupported(inst));
                    }
                    let last = self
                        .cfg
                        .block_ends
                        .get(&i)
                        .map(|e| e.last)
                        .ok_or_else(|| self.bad_jump(inst))?;
                    let closing_jump =
                        (last < target && self.ins[last].op == "JMP").then_some(last);
                    let then_end = closing_jump.unwrap_or(target);
                    let branch_stack = self.stack.clone();
                    let then_body = self.build_range(i + 1, then_end)?;
                    self.stack = branch_stack.clone();
                    let (else_body, next) = if let Some(jmp) = closing_jump {
                        let join = jump_index(self.cfg, &self.ins[jmp])?;
                        if join > target {
                            let body = self.build_range(target, join.min(end))?;
                            self.stack = branch_stack.clone();
                            (Some(to_else_arm(body)), join)
                        } else {
                            (None, target)
                        }
                    } else {
                        (None, target)
                    };
                    self.stack = branch_stack;
                    block.stmts.push(Stmt::If {
                        cond,
                        then_body,
                        else_body,
                    });
                    i = next;
                    continue;
                }
                "JMP" | "RETN" | "SAVEBP" | "RESTOREBP" => {}
                op if op.starts_with("NOP") => {}
                _ => return Err(self.unsupported(inst)),
            }
            i += 1;
        }
        Ok(block)
    }

    fn push_const(
        &mut self,
        inst: &Instruction,
        make: impl FnOnce(i64) -> Const,
    ) -> Result<(), BuildError> {
        match inst.args.first() {
            Some(Arg::Int(v)) => {
                self.stack.push(Value::Expr(Expr::Const(make(*v))));
                Ok(())
            }
            _ => Err(self.bad_operand(inst)),
        }
    }

    fn copy_sp(&mut self, inst: &Instruction) -> Result<(), BuildError> {
        let loc = stack_offset_to_pos(arg_i32(inst, 0)?);
        let count = stack_size_to_pos(arg_i32(inst, 1)?);
        if loc == 0 || count == 0 || count > loc || loc > self.stack.len() {
            return Err(self.bad_operand(inst));
        }
        let first = self.stack.len() - loc;
        let values = self.stack[first..first + count].to_vec();
        self.stack.extend(values);
        Ok(())
    }

    fn copy_bp(&mut self, inst: &Instruction) -> Result<(), BuildError> {
        let pos = stack_offset_to_pos(arg_i32(inst, 0)?);
        let count = stack_size_to_pos(arg_i32(inst, 1)?);
        if pos == 0 || count != 1 || pos > self.globals.vars.len() {
            return Err(self.bad_operand(inst));
        }
        self.stack
            .push(Value::Global(self.globals.vars.len() - pos));
        Ok(())
    }

    fn assign_sp(&mut self, inst: &Instruction, block: &mut Block) -> Result<(), BuildError> {
        let loc = stack_offset_to_pos(arg_i32(inst, 0)?);
        let count = stack_size_to_pos(arg_i32(inst, 1)?);
        if count != 1 {
            return Err(self.unsupported(inst));
        }
        let rhs = self
            .stack
            .last()
            .cloned()
            .ok_or_else(|| self.underflow(inst))?
            .expr();
        if loc > self.stack.len()
            || (loc == self.stack.len()
                && !matches!(self.stack.first(), Some(Value::Local(_) | Value::Global(_))))
        {
            block.stmts.push(Stmt::Return(Some(rhs)));
            return Ok(());
        }
        let dest = self.stack.len() - loc;
        let lhs = self.stack[dest].clone();
        if let Value::Local(id) = lhs {
            if let Some(Stmt::VarDecl { var, init }) = block
                .stmts
                .iter_mut()
                .rev()
                .find(|stmt| matches!(stmt, Stmt::VarDecl { var, init: None } if *var == id))
            {
                if *var == id {
                    *init = Some(rhs);
                    return Ok(());
                }
            }
        }
        block.stmts.push(Stmt::Expr(Expr::Assign {
            lhs: Box::new(lhs.expr()),
            rhs: Box::new(rhs),
        }));
        Ok(())
    }

    fn assign_bp(&mut self, inst: &Instruction, block: &mut Block) -> Result<(), BuildError> {
        let pos = stack_offset_to_pos(arg_i32(inst, 0)?);
        if stack_size_to_pos(arg_i32(inst, 1)?) != 1 || pos == 0 || pos > self.globals.vars.len() {
            return Err(self.bad_operand(inst));
        }
        let rhs = self.pop(inst)?.expr();
        let lhs = Value::Global(self.globals.vars.len() - pos).expr();
        block.stmts.push(Stmt::Expr(Expr::Assign {
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
        }));
        Ok(())
    }

    fn movsp(&mut self, inst: &Instruction) -> Result<(), BuildError> {
        let amount = arg_i32(inst, 0)?;
        if amount >= 0 {
            return Err(self.unsupported(inst));
        }
        let count = stack_offset_to_pos(amount);
        if count > self.stack.len() {
            return Err(self.underflow(inst));
        }
        self.stack.truncate(self.stack.len() - count);
        Ok(())
    }

    fn call_action(&mut self, inst: &Instruction, block: &mut Block) -> Result<(), BuildError> {
        let id = inst.routine.unwrap_or(arg_i32(inst, 0)? as u16);
        let argc = inst.argc.unwrap_or(arg_i32(inst, 1)? as u8) as usize;
        let sig = action(self.game, id).ok_or_else(|| self.bad_operand(inst))?;
        let mut args = Vec::with_capacity(argc);
        for _ in 0..argc {
            args.push(self.pop(inst)?.expr());
        }
        let call = Expr::CallAction {
            name: sig.name.to_string(),
            args,
        };
        if sig.ret == Ty::Void {
            block.stmts.push(Stmt::Expr(call));
        } else {
            self.stack.push(Value::Expr(call));
        }
        Ok(())
    }

    fn call_sub(&mut self, inst: &Instruction, block: &mut Block) -> Result<(), BuildError> {
        let target = match inst.args.first() {
            Some(Arg::Jump(v)) => *v,
            _ => return Err(self.bad_operand(inst)),
        };
        let (&id, info) = self
            .protos
            .iter()
            .find(|(_, info)| info.start_pos == target)
            .ok_or_else(|| self.bad_jump(inst))?;
        let mut args = Vec::with_capacity(info.param_count);
        for _ in 0..info.param_count {
            args.push(self.pop(inst)?.expr());
        }
        let user = match id {
            SubId::User(n) => n,
            _ => return Err(self.unsupported(inst)),
        };
        let call = Expr::CallSub { id: user, args };
        if info.ret == Ty::Void {
            block.stmts.push(Stmt::Expr(call));
        } else {
            self.stack.push(Value::Expr(call));
        }
        Ok(())
    }

    fn binary(&mut self, inst: &Instruction, op: BinOp) -> Result<(), BuildError> {
        let rhs = self.pop(inst)?.expr();
        let lhs = self.pop(inst)?.expr();
        self.stack.push(Value::Expr(Expr::Binary {
            op,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
        }));
        Ok(())
    }

    fn unary(&mut self, inst: &Instruction, op: UnaryOp) -> Result<(), BuildError> {
        let expr = self.pop(inst)?.expr();
        self.stack.push(Value::Expr(Expr::Unary {
            op,
            expr: Box::new(expr),
        }));
        Ok(())
    }

    fn pop(&mut self, inst: &Instruction) -> Result<Value, BuildError> {
        self.stack.pop().ok_or_else(|| self.underflow(inst))
    }

    fn underflow(&self, inst: &Instruction) -> BuildError {
        BuildError::StackUnderflow { pos: inst.offset }
    }

    fn bad_operand(&self, inst: &Instruction) -> BuildError {
        BuildError::BadOperand { pos: inst.offset }
    }

    fn bad_jump(&self, inst: &Instruction) -> BuildError {
        BuildError::BadJump { pos: inst.offset }
    }

    fn unsupported(&self, inst: &Instruction) -> BuildError {
        BuildError::Unsupported {
            pos: inst.offset,
            op: inst.op.to_string(),
        }
    }
}

fn to_else_arm(mut body: Block) -> ElseArm {
    if body.stmts.len() == 1 && matches!(body.stmts.first(), Some(Stmt::If { .. })) {
        if let Stmt::If {
            cond,
            then_body,
            else_body,
        } = body.stmts.remove(0)
        {
            return ElseArm::ElseIf {
                cond,
                then_body,
                else_body: else_body.map(Box::new),
            };
        }
    }
    ElseArm::Else(body)
}

fn jump_index(cfg: &Cfg, inst: &Instruction) -> Result<usize, BuildError> {
    let target = match inst.args.first() {
        Some(Arg::Jump(v)) => *v,
        _ => return Err(BuildError::BadJump { pos: inst.offset }),
    };
    cfg.index_of
        .get(&target)
        .copied()
        .ok_or(BuildError::BadJump { pos: inst.offset })
}

fn arg_i32(inst: &Instruction, index: usize) -> Result<i32, BuildError> {
    match inst.args.get(index) {
        Some(Arg::Int(v)) => Ok(*v as i32),
        _ => Err(BuildError::BadOperand { pos: inst.offset }),
    }
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

fn unary_op(op: &str) -> Option<UnaryOp> {
    match op {
        "NEGI" | "NEGF" => Some(UnaryOp::Neg),
        "NOTI" => Some(UnaryOp::Not),
        "COMPI" => Some(UnaryOp::BitNot),
        _ => None,
    }
}

fn binary_op(op: &str) -> Option<BinOp> {
    if op.starts_with("NEQUAL") {
        Some(BinOp::Ne)
    } else if op.starts_with("EQUAL") {
        Some(BinOp::Eq)
    } else if op.starts_with("LOGAND") {
        Some(BinOp::LogAnd)
    } else if op.starts_with("LOGOR") {
        Some(BinOp::LogOr)
    } else if op.starts_with("BOOLAND") {
        Some(BinOp::BitAnd)
    } else if op.starts_with("INCOR") {
        Some(BinOp::BitOr)
    } else if op.starts_with("EXCOR") {
        Some(BinOp::BitXor)
    } else if op.starts_with("SHLEFT") {
        Some(BinOp::Shl)
    } else if op.starts_with("SHRIGHT") || op.starts_with("USHRIGHT") {
        Some(BinOp::Shr)
    } else if op.starts_with("LEQ") {
        Some(BinOp::Le)
    } else if op.starts_with("GEQ") {
        Some(BinOp::Ge)
    } else if op.starts_with("LT") {
        Some(BinOp::Lt)
    } else if op.starts_with("GT") {
        Some(BinOp::Gt)
    } else if op.starts_with("ADD") {
        Some(BinOp::Add)
    } else if op.starts_with("SUB") {
        Some(BinOp::Sub)
    } else if op.starts_with("MUL") {
        Some(BinOp::Mul)
    } else if op.starts_with("DIV") {
        Some(BinOp::Div)
    } else if op.starts_with("MOD") {
        Some(BinOp::Mod)
    } else {
        None
    }
}
