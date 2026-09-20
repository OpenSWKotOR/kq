//! Stack-to-AST build pass for expressions, `if`/`else`, loops, switch, and deferred calls.

use std::collections::HashMap;

use kq_format::ncs::{Arg, Instruction};

use crate::actions::action;
use crate::ast::{BinOp, Block, ElseArm, Expr, Stmt, SwitchCase, UnaryOp};
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
    /// After prefix `++`/`--`, the following `CPTOPSP`/`CPTOPBP` is the new value and is dropped (§6.5).
    suppress_next_copy: bool,
    /// `STORE_STATE` results consumed as `Ty::Action` ACTION args (size 0, not stacked).
    pending_deferred: Vec<Expr>,
    /// Forward `JMP` to this index inside a switch case is `break`.
    break_target: Option<usize>,
}

enum LoopKind {
    While { jz: usize, back: usize },
    DoWhile { jz: usize, back: usize },
}

struct SwitchPlan {
    /// (case value or `None` = default, body start, body end)
    arms: Vec<(Option<Const>, usize, usize)>,
    end: usize,
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
        suppress_next_copy: false,
        pending_deferred: Vec::new(),
        break_target: None,
    };
    builder.build_range(sub.range.start, sub.range.end)
}

impl Builder<'_> {
    fn build_range(&mut self, start: usize, end: usize) -> Result<Block, BuildError> {
        let mut block = Block::default();
        let mut i = start;
        while i < end {
            if self.cfg.dead.get(i).copied().unwrap_or(false) {
                i += 1;
                continue;
            }
            let op = self.ins[i].op;
            if self.suppress_next_copy && !matches!(op, "CPTOPSP" | "CPTOPBP") {
                self.suppress_next_copy = false;
            }
            if let Some(kind) = self.loop_at(i, end) {
                let (stmt, next) = self.build_loop(i, kind)?;
                block.stmts.push(stmt);
                i = next;
                continue;
            }
            let inst = &self.ins[i];
            match inst.op {
                op if op.starts_with("RSADD") => {
                    let ty = ty_from_rsadd(op).ok_or_else(|| self.unsupported(inst))?;
                    let id = VarId(self.next_var);
                    self.next_var += 1;
                    self.stack.push(Value::Local(id));
                    block.stmts.push(Stmt::VarDecl {
                        var: id,
                        ty,
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
                "MOVSP" => self.movsp(inst, &mut block)?,
                "INCxSP" | "DECxSP" => self.inc_sp(inst)?,
                "INCxBP" | "DECxBP" => self.inc_bp(inst)?,
                "STORE_STATE" => {
                    self.build_store_state(i)?;
                    i = jump_index(self.cfg, &self.ins[i + 1])?;
                    continue;
                }
                "ACTION" => self.call_action(inst, &mut block)?,
                "JSR" => self.call_sub(inst, &mut block)?,
                op if binary_op(op).is_some() => self.binary(inst, binary_op(op).unwrap())?,
                op if unary_op(op).is_some() => self.unary(inst, unary_op(op).unwrap())?,
                "JZ" | "JNZ" if self.is_short_circuit_jz(i) => {
                    self.pop(inst)?;
                }
                "JZ" | "JNZ" => {
                    if inst.op == "JNZ" {
                        if let Some((stmt, next)) = self.try_switch(i, end)? {
                            block.stmts.push(stmt);
                            i = next;
                            continue;
                        }
                    }
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
                "JMP" => {
                    if let Some(target) = self.break_target {
                        if jump_index(self.cfg, inst)? == target {
                            block.stmts.push(Stmt::Break);
                        }
                    }
                }
                "RETN" | "SAVEBP" | "RESTOREBP" => {}
                op if op.starts_with("NOP") => {}
                _ => return Err(self.unsupported(inst)),
            }
            i += 1;
        }
        Ok(block)
    }

    fn loop_at(&self, i: usize, end: usize) -> Option<LoopKind> {
        let origins = self.cfg.origins_backward.get(&i)?;
        let back = origins
            .iter()
            .copied()
            .filter(|&o| o >= i && o < end)
            .max()?;
        if self.is_do_while(back) {
            Some(LoopKind::DoWhile { jz: back - 1, back })
        } else {
            Some(LoopKind::While {
                jz: self.find_while_jz(i, back)?,
                back,
            })
        }
    }

    fn is_do_while(&self, back: usize) -> bool {
        if back == 0 {
            return false;
        }
        let jz = &self.ins[back - 1];
        if jz.op != "JZ" {
            return false;
        }
        matches!(jz.args.first(), Some(Arg::Jump(t)) if *t == jz.offset + 12)
    }

    fn find_while_jz(&self, head: usize, back: usize) -> Option<usize> {
        let after = back + 1;
        for j in head..back {
            if self.ins[j].op != "JZ" {
                continue;
            }
            if self.cfg.and_guards.contains(&j)
                || self.cfg.log_or_extra_jz.contains(&j)
                || self.is_or_guard_head(j)
            {
                continue;
            }
            let Ok(target) = jump_index(self.cfg, &self.ins[j]) else {
                continue;
            };
            if target == after {
                return Some(j);
            }
        }
        None
    }

    fn build_loop(&mut self, head: usize, kind: LoopKind) -> Result<(Stmt, usize), BuildError> {
        match kind {
            LoopKind::While { jz, back } => {
                let _cond_block = self.build_range(head, jz)?;
                let cond = self.pop_at(jz)?;
                let stack = self.stack.clone();
                let body = self.build_range(jz + 1, back)?;
                self.stack = stack;
                Ok((Stmt::While { cond, body }, back + 1))
            }
            LoopKind::DoWhile { jz, back } => {
                let stack = self.stack.clone();
                let body = self.build_range(head, jz)?;
                let cond = self.pop_at(jz)?;
                self.stack = stack;
                Ok((Stmt::DoWhile { body, cond }, back + 1))
            }
        }
    }

    fn pop_at(&mut self, i: usize) -> Result<Expr, BuildError> {
        let pos = self.ins[i].offset;
        self.stack
            .pop()
            .map(|v| v.expr())
            .ok_or(BuildError::StackUnderflow { pos })
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
        if self.suppress_next_copy {
            self.suppress_next_copy = false;
            return Ok(());
        }
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
        if self.suppress_next_copy {
            self.suppress_next_copy = false;
            return Ok(());
        }
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
            if let Some(Stmt::VarDecl { var, init, .. }) =
                block.stmts.iter_mut().rev().find(
                    |stmt| matches!(stmt, Stmt::VarDecl { var, init: None, .. } if *var == id),
                )
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

    fn movsp(&mut self, inst: &Instruction, block: &mut Block) -> Result<(), BuildError> {
        let amount = arg_i32(inst, 0)?;
        if amount >= 0 {
            return Err(self.unsupported(inst));
        }
        let count = stack_offset_to_pos(amount);
        if count > self.stack.len() {
            return Err(self.underflow(inst));
        }
        let start = self.stack.len() - count;
        let popped: Vec<Value> = self.stack.drain(start..).collect();
        for value in popped {
            if is_inc_expr(&value) {
                block.stmts.push(Stmt::Expr(value.expr()));
            }
        }
        Ok(())
    }

    fn inc_sp(&mut self, inst: &Instruction) -> Result<(), BuildError> {
        let loc = stack_offset_to_pos(arg_i32(inst, 0)?);
        if loc == 0 || loc > self.stack.len() {
            return Err(self.bad_operand(inst));
        }
        let dest = self.stack.len() - loc;
        self.apply_inc(inst, self.stack[dest].clone(), loc > 1)
    }

    fn inc_bp(&mut self, inst: &Instruction) -> Result<(), BuildError> {
        let pos = stack_offset_to_pos(arg_i32(inst, 0)?);
        if pos == 0 || pos > self.globals.vars.len() {
            return Err(self.bad_operand(inst));
        }
        let var = Value::Global(self.globals.vars.len() - pos);
        self.apply_inc(inst, var, true)
    }

    fn apply_inc(
        &mut self,
        inst: &Instruction,
        var: Value,
        copy_may_precede: bool,
    ) -> Result<(), BuildError> {
        match &var {
            Value::Local(_) | Value::Global(_) => {}
            _ => return Err(self.unsupported(inst)),
        }
        let postfix = copy_may_precede
            && self
                .stack
                .last()
                .is_some_and(|top| values_same_var(top, &var));
        let is_inc = inst.op.starts_with("INC");
        let op = match (is_inc, postfix) {
            (true, true) => UnaryOp::PostInc,
            (false, true) => UnaryOp::PostDec,
            (true, false) => UnaryOp::PreInc,
            (false, false) => UnaryOp::PreDec,
        };
        let expr = Expr::Unary {
            op,
            expr: Box::new(var.expr()),
        };
        if postfix {
            self.stack.pop();
            self.stack.push(Value::Expr(expr));
        } else {
            self.suppress_next_copy = true;
            self.stack.push(Value::Expr(expr));
        }
        Ok(())
    }

    fn call_action(&mut self, inst: &Instruction, block: &mut Block) -> Result<(), BuildError> {
        let id = inst.routine.unwrap_or(arg_i32(inst, 0)? as u16);
        let argc = inst.argc.unwrap_or(arg_i32(inst, 1)? as u8) as usize;
        let sig = action(self.game, id).ok_or_else(|| self.bad_operand(inst))?;
        let mut args = Vec::with_capacity(argc);
        for idx in 0..argc {
            let is_action = sig.params.get(idx).is_some_and(|p| p.ty == Ty::Action);
            if is_action {
                args.push(
                    self.pending_deferred
                        .pop()
                        .ok_or_else(|| self.bad_operand(inst))?,
                );
            } else {
                args.push(self.pop(inst)?.expr());
            }
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

    fn is_short_circuit_jz(&self, i: usize) -> bool {
        self.cfg.and_guards.contains(&i)
            || self.cfg.log_or_extra_jz.contains(&i)
            || self.is_or_guard_head(i)
    }

    /// First `JZ` of `CPTOPSP -4 4; JZ; CPTOPSP -4 4; JZ` (§5.7). Extra JZ is `log_or_extra_jz`.
    fn is_or_guard_head(&self, i: usize) -> bool {
        if self.ins.get(i).is_none_or(|inst| inst.op != "JZ") {
            return false;
        }
        matches!(
            (self.ins.get(i + 1), self.ins.get(i + 2)),
            (Some(copy), Some(jz2))
                if is_cptopsp_dup(copy)
                    && jz2.op == "JZ"
                    && self.cfg.log_or_extra_jz.contains(&(i + 2))
        )
    }

    fn build_store_state(&mut self, i: usize) -> Result<(), BuildError> {
        let inst = &self.ins[i];
        let region = self
            .cfg
            .deferred
            .iter()
            .find(|d| d.store_idx == i)
            .cloned()
            .ok_or_else(|| self.unsupported(inst))?;
        if self.ins.get(i + 1).is_none_or(|j| j.op != "JMP") {
            return Err(self.unsupported(inst));
        }
        let slots = stack_size_to_pos(arg_i32(inst, 1)?);
        if slots > self.stack.len() {
            return Err(self.underflow(inst));
        }
        let seed = self.stack[self.stack.len() - slots..].to_vec();
        let saved_stack = std::mem::replace(&mut self.stack, seed);
        let saved_pending = std::mem::take(&mut self.pending_deferred);
        let saved_suppress = self.suppress_next_copy;
        let saved_break = self.break_target;
        self.suppress_next_copy = false;
        self.break_target = None;
        let body = self.build_range(region.body.start, region.body.end);
        self.stack = saved_stack;
        self.pending_deferred = saved_pending;
        self.suppress_next_copy = saved_suppress;
        self.break_target = saved_break;
        let mut body = body?;
        if body.stmts.len() != 1 {
            return Err(self.unsupported(&self.ins[i]));
        }
        let stmt = body.stmts.pop().expect("len == 1");
        self.pending_deferred.push(Expr::Deferred(Box::new(stmt)));
        Ok(())
    }

    /// Selector-dup + `EQUAL`; `JNZ` chain + trailing `MOVSP -4` (§5.5). Else `Ok(None)` → if/else.
    fn try_switch(&mut self, i: usize, end: usize) -> Result<Option<(Stmt, usize)>, BuildError> {
        let Some(plan) = self.parse_switch(i, end) else {
            return Ok(None);
        };
        let eq = self.pop_at(i)?;
        let sel = match eq {
            Expr::Binary {
                op: BinOp::Eq, lhs, ..
            } => *lhs,
            other => other,
        };
        let stack = self.stack.clone();
        let saved_break = self.break_target;
        self.break_target = Some(plan.end);
        let mut cases = Vec::new();
        for (value, start, body_end) in plan.arms {
            self.stack = stack.clone();
            let body = self.build_range(start, body_end)?;
            cases.push(SwitchCase { value, body });
        }
        self.break_target = saved_break;
        self.stack = stack;
        self.stack.pop().ok_or(BuildError::StackUnderflow {
            pos: self.ins[i].offset,
        })?;
        Ok(Some((Stmt::Switch { sel, cases }, plan.end + 1)))
    }

    fn parse_switch(&self, first_jnz: usize, end: usize) -> Option<SwitchPlan> {
        let mut j = first_jnz;
        let mut cases: Vec<(Const, usize)> = Vec::new();
        loop {
            let value = self.dup_eq_jnz_const(j)?;
            let target = jump_index(self.cfg, &self.ins[j]).ok()?;
            if target <= j || target > end {
                return None;
            }
            cases.push((value, target));
            let next = j + 1;
            if next < end && is_cptopsp_dup(&self.ins[next]) {
                j = next + 3;
                continue;
            }
            if next < end && self.ins[next].op == "JMP" {
                let d = jump_index(self.cfg, &self.ins[next]).ok()?;
                let body_lo = cases.iter().map(|(_, t)| *t).min()?;
                let switch_end = self.find_switch_end(body_lo, end, d)?;
                if !self.is_movsp_m4(switch_end) {
                    return None;
                }
                let mut arms = Vec::new();
                let mut starts: Vec<usize> = cases.iter().map(|(_, t)| *t).collect();
                let has_default = !self.is_movsp_m4(d);
                if has_default {
                    starts.push(d);
                }
                starts.push(switch_end);
                starts.sort_unstable();
                starts.dedup();
                for (value, start) in &cases {
                    let body_end = starts.iter().copied().find(|s| *s > *start)?;
                    arms.push((Some(value.clone()), *start, body_end));
                }
                if has_default {
                    let body_end = starts.iter().copied().find(|s| s > &d)?;
                    arms.push((None, d, body_end));
                }
                return Some(SwitchPlan {
                    arms,
                    end: switch_end,
                });
            }
            return None;
        }
    }

    fn dup_eq_jnz_const(&self, jnz: usize) -> Option<Const> {
        if jnz < 3 || self.ins[jnz].op != "JNZ" {
            return None;
        }
        if !is_cptopsp_dup(&self.ins[jnz - 3]) {
            return None;
        }
        if self.ins[jnz - 2].op != "CONSTI" || !self.ins[jnz - 1].op.starts_with("EQUAL") {
            return None;
        }
        match self.ins[jnz - 2].args.first() {
            Some(Arg::Int(v)) => Some(Const::Int(*v as i32)),
            _ => None,
        }
    }

    fn find_switch_end(&self, body_lo: usize, range_end: usize, d: usize) -> Option<usize> {
        if self.is_movsp_m4(d) {
            return Some(d);
        }
        let mut found = None;
        for k in body_lo..range_end {
            if self.ins[k].op != "JMP" {
                continue;
            }
            let Ok(t) = jump_index(self.cfg, &self.ins[k]) else {
                continue;
            };
            if t <= k || t >= range_end {
                continue;
            }
            if self.is_movsp_m4(t) {
                found = Some(t);
            }
        }
        found.or_else(|| (d..range_end).find(|&k| self.is_movsp_m4(k)))
    }

    fn is_movsp_m4(&self, i: usize) -> bool {
        self.ins.get(i).is_some_and(|inst| {
            inst.op == "MOVSP" && matches!(inst.args.first(), Some(Arg::Int(-4)))
        })
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

fn is_cptopsp_dup(inst: &Instruction) -> bool {
    inst.op == "CPTOPSP"
        && matches!(inst.args.first(), Some(Arg::Int(-4)))
        && matches!(inst.args.get(1), Some(Arg::Int(4)))
}

fn values_same_var(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Local(x), Value::Local(y)) => x == y,
        (Value::Global(x), Value::Global(y)) => x == y,
        _ => false,
    }
}

fn is_inc_expr(value: &Value) -> bool {
    matches!(
        value,
        Value::Expr(Expr::Unary {
            op: UnaryOp::PreInc | UnaryOp::PreDec | UnaryOp::PostInc | UnaryOp::PostDec,
            ..
        })
    )
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
