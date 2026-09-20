//! CFG destinations, origins, dead code, and short-circuit guards (DeNCS §5.7 / §5.9).

use std::collections::{HashMap, HashSet, VecDeque};

use kq_format::ncs::{Arg, Instruction};

use crate::split::{DeferredRegion, SubRange};

#[derive(Clone, Debug)]
pub struct Cfg {
    /// File offset → instruction index in `ins`.
    pub index_of: HashMap<u32, usize>,
    pub succ: Vec<Vec<usize>>,
    pub pred: Vec<Vec<usize>>,
    /// dest → JMP origins with `target < pos` (§5.3).
    pub origins_backward: HashMap<usize, Vec<usize>>,
    pub dead: Vec<bool>,
    pub log_or_extra_jz: HashSet<usize>,
    /// `CPTOPSP -4 4; JZ` short-circuit for `&&` (§5.7).
    pub and_guards: HashSet<usize>,
    pub deferred: Vec<DeferredRegion>,
    /// Jump index → last instruction of the block (CFG predecessor of the target, not `target-6`).
    pub block_ends: HashMap<usize, BlockEnd>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockEnd {
    pub last: usize,
}

/// Build the intra-subroutine CFG for `sub`.
pub fn analyze(ins: &[Instruction], sub: &SubRange, deferred: &[DeferredRegion]) -> Cfg {
    let n = ins.len();
    let mut index_of = HashMap::with_capacity(n);
    for (i, inst) in ins.iter().enumerate() {
        index_of.insert(inst.offset, i);
    }

    let range = sub.range.clone();
    let log_or_extra_jz = detect_log_or_extra_jz(ins, &range);

    let mut succ = vec![Vec::new(); n];
    for i in range.clone() {
        succ[i] = successors(ins, i, &range, &index_of, &log_or_extra_jz);
    }

    let mut pred = vec![Vec::new(); n];
    for (src, dests) in succ.iter().enumerate() {
        for &d in dests {
            pred[d].push(src);
        }
    }
    for p in &mut pred {
        p.sort_unstable();
        p.dedup();
    }

    let mut origins_backward: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in range.clone() {
        if ins[i].op != "JMP" {
            continue;
        }
        let Some(target) = jump_target(&ins[i]) else {
            continue;
        };
        if target >= ins[i].offset {
            continue;
        }
        if let Some(&dest) = index_of.get(&target) {
            origins_backward.entry(dest).or_default().push(i);
        }
    }
    for v in origins_backward.values_mut() {
        v.sort_unstable();
        v.dedup();
    }

    let dead = mark_dead(n, range.start, &succ);

    let and_guards = detect_and_guards(ins, &range, &index_of, &log_or_extra_jz);
    let block_ends = detect_block_ends(ins, &range, &index_of);

    let deferred = deferred
        .iter()
        .filter(|d| range.contains(&d.store_idx))
        .cloned()
        .collect();

    Cfg {
        index_of,
        succ,
        pred,
        origins_backward,
        dead,
        log_or_extra_jz,
        and_guards,
        deferred,
        block_ends,
    }
}

fn successors(
    ins: &[Instruction],
    i: usize,
    range: &std::ops::Range<usize>,
    index_of: &HashMap<u32, usize>,
    log_or_extra_jz: &HashSet<usize>,
) -> Vec<usize> {
    let mut out = Vec::new();
    let fallthrough = i + 1;
    let in_sub = |j: usize| range.contains(&j);

    match ins[i].op {
        "RETN" => {}
        "JMP" => push_target(&mut out, ins, i, index_of),
        "JZ" | "JNZ" => {
            if in_sub(fallthrough) {
                out.push(fallthrough);
            }
            // Extra || JZ is not a typing/CFG decision (§5.7); fall through to `b`.
            if !log_or_extra_jz.contains(&i) {
                push_target(&mut out, ins, i, index_of);
            }
        }
        "STORE_STATE" => {
            if in_sub(fallthrough) {
                out.push(fallthrough);
            }
            // Body after STORE_STATE; JMP is reachable as fallthrough; engine runs the skipped range later (§5.9).
            if ins.get(fallthrough).is_some_and(|j| j.op == "JMP") && in_sub(i + 2) {
                out.push(i + 2);
            }
        }
        _ => {
            if in_sub(fallthrough) {
                out.push(fallthrough);
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

fn push_target(
    out: &mut Vec<usize>,
    ins: &[Instruction],
    i: usize,
    index_of: &HashMap<u32, usize>,
) {
    if let Some(target) = jump_target(&ins[i]) {
        if let Some(&j) = index_of.get(&target) {
            out.push(j);
        }
    }
}

fn jump_target(ins: &Instruction) -> Option<u32> {
    match ins.args.first() {
        Some(Arg::Jump(t)) => Some(*t),
        _ => None,
    }
}

fn mark_dead(n: usize, entry: usize, succ: &[Vec<usize>]) -> Vec<bool> {
    let mut dead = vec![true; n];
    if entry >= n {
        return dead;
    }
    let mut q = VecDeque::new();
    dead[entry] = false;
    q.push_back(entry);
    while let Some(i) = q.pop_front() {
        for &s in &succ[i] {
            if dead[s] {
                dead[s] = false;
                q.push_back(s);
            }
        }
    }
    dead
}

fn is_cptopsp_guard(ins: &Instruction) -> bool {
    ins.op == "CPTOPSP"
        && matches!(ins.args.first(), Some(Arg::Int(-4)))
        && matches!(ins.args.get(1), Some(Arg::Int(4)))
}

/// `CPTOPSP -4 4; JZ; CPTOPSP -4 4; JZ` → mark the second JZ (§5.7).
fn detect_log_or_extra_jz(ins: &[Instruction], range: &std::ops::Range<usize>) -> HashSet<usize> {
    let mut extra = HashSet::new();
    let mut state = 0u8;
    for i in range.clone() {
        loop {
            match (state, ins[i].op) {
                (0, _) if is_cptopsp_guard(&ins[i]) => {
                    state = 1;
                    break;
                }
                (1, "JZ") => {
                    state = 2;
                    break;
                }
                (2, _) if is_cptopsp_guard(&ins[i]) => {
                    state = 3;
                    break;
                }
                (3, "JZ") => {
                    extra.insert(i);
                    state = 0;
                    break;
                }
                (_, _) if state != 0 => {
                    state = 0;
                    continue;
                }
                _ => break,
            }
        }
    }
    extra
}

/// `CPTOPSP -4 4; JZ ->X` where `X` is the instruction after `LOGANDII`/`LOGORII`.
fn detect_and_guards(
    ins: &[Instruction],
    range: &std::ops::Range<usize>,
    index_of: &HashMap<u32, usize>,
    log_or_extra_jz: &HashSet<usize>,
) -> HashSet<usize> {
    let mut guards = HashSet::new();
    let start = range.start;
    let end = range.end;
    if end - start < 2 {
        return guards;
    }
    for i in start..end - 1 {
        if !is_cptopsp_guard(&ins[i]) || ins[i + 1].op != "JZ" {
            continue;
        }
        let jz = i + 1;
        if log_or_extra_jz.contains(&jz) {
            continue;
        }
        let Some(target) = jump_target(&ins[jz]) else {
            continue;
        };
        let Some(&tidx) = index_of.get(&target) else {
            continue;
        };
        if tidx == 0 {
            continue;
        }
        let prev = &ins[tidx - 1];
        if prev.op == "LOGANDII" || prev.op == "LOGORII" {
            guards.insert(jz);
        }
    }
    guards
}

fn detect_block_ends(
    ins: &[Instruction],
    range: &std::ops::Range<usize>,
    index_of: &HashMap<u32, usize>,
) -> HashMap<usize, BlockEnd> {
    let mut ends = HashMap::new();
    for i in range.clone() {
        if !matches!(ins[i].op, "JMP" | "JZ" | "JNZ") {
            continue;
        }
        let Some(target) = jump_target(&ins[i]) else {
            continue;
        };
        if target <= ins[i].offset {
            continue;
        }
        let Some(&tidx) = index_of.get(&target) else {
            continue;
        };
        if tidx == 0 {
            continue;
        }
        ends.insert(i, BlockEnd { last: tidx - 1 });
    }
    ends
}

#[cfg(test)]
mod tests {
    use super::analyze;
    use crate::split;
    use kq_format::ncs::{Arg, Instruction};

    #[derive(Clone, Debug)]
    enum AsmArg {
        JumpAbs(u32),
        Int(i64),
    }

    fn asm(lines: &[(&str, Vec<AsmArg>)]) -> Vec<Instruction> {
        let mut offset = 13u32;
        let mut out = Vec::with_capacity(lines.len());
        for (op, args) in lines {
            let spec = kotor_ncs_isa::lookup_mnemonic(op)
                .unwrap_or_else(|| panic!("unknown mnemonic {op}"));
            let size = 2 + spec.operands.byte_len().expect("fixed-size op") as u32;
            let mut ins = Instruction {
                offset,
                op: spec.mnemonic,
                args: Vec::new(),
                routine: None,
                routine_name: None,
                argc: None,
            };
            for arg in args {
                match arg {
                    AsmArg::JumpAbs(t) => ins.args.push(Arg::Jump(*t)),
                    AsmArg::Int(v) => ins.args.push(Arg::Int(*v)),
                }
            }
            if *op == "ACTION" {
                if let Some(Arg::Int(id)) = ins.args.first() {
                    ins.routine = Some(*id as u16);
                }
                if let Some(Arg::Int(argc)) = ins.args.get(1) {
                    ins.argc = Some(*argc as u8);
                }
            }
            out.push(ins);
            offset += size;
        }
        out
    }

    fn idx_at(ins: &[Instruction], pos: u32) -> usize {
        ins.iter()
            .position(|i| i.offset == pos)
            .unwrap_or_else(|| panic!("no instruction at offset {pos}"))
    }

    #[test]
    fn dead_epilogue_after_return_is_dead() {
        // StartingConditional shape from §2.1(c): MOVSP before final RETN after JMP is dead
        let ins = asm(&[
            ("RSADDI", vec![]),
            ("JSR", vec![AsmArg::JumpAbs(23)]),
            ("RETN", vec![]),
            ("CONSTI", vec![AsmArg::Int(0)]),
            ("CPDOWNSP", vec![AsmArg::Int(-8), AsmArg::Int(4)]),
            ("MOVSP", vec![AsmArg::Int(-4)]),
            ("JMP", vec![AsmArg::JumpAbs(55)]),
            ("MOVSP", vec![AsmArg::Int(-4)]),
            ("RETN", vec![]),
        ]);
        let p = split(&ins).unwrap();
        let cfg = analyze(&ins, &p.main, &[]);
        let dead_movsp = idx_at(&ins, 49);
        assert!(cfg.dead[dead_movsp]);
        assert!(!cfg.dead[idx_at(&ins, 55)]);
    }

    #[test]
    fn log_or_extra_jz_detected() {
        // Shape from §9.4 example (5) k_pdan_mand04:
        // CPTOPSP -4 4; JZ ->R; CPTOPSP -4 4; JZ ->J; R: <b>; J: LOGORII
        // Offsets: a@21, first JZ@35→55, extra JZ@49→61, b@55, LOGORII@61.
        let ins = asm(&[
            ("JSR", vec![AsmArg::JumpAbs(21)]),
            ("RETN", vec![]),
            ("CONSTI", vec![AsmArg::Int(0)]),
            ("CPTOPSP", vec![AsmArg::Int(-4), AsmArg::Int(4)]),
            ("JZ", vec![AsmArg::JumpAbs(55)]),
            ("CPTOPSP", vec![AsmArg::Int(-4), AsmArg::Int(4)]),
            ("JZ", vec![AsmArg::JumpAbs(61)]),
            ("CONSTI", vec![AsmArg::Int(1)]),
            ("LOGORII", vec![]),
            ("RETN", vec![]),
        ]);
        let p = split(&ins).unwrap();
        let cfg = analyze(&ins, &p.main, &p.deferred);
        let extra_jz_idx = idx_at(&ins, 49);
        assert!(cfg.log_or_extra_jz.contains(&extra_jz_idx));
        assert!(!cfg.log_or_extra_jz.contains(&idx_at(&ins, 35)));
        assert!(!cfg.and_guards.contains(&extra_jz_idx));
    }

    #[test]
    fn and_guard_jz_detected() {
        // §5.7 / §9.4 (3): CPTOPSP -4 4; JZ ->X where X is the instruction after LOGANDII.
        // Offsets: CONSTI@21, CPTOPSP@27, JZ@35→49, CONSTI@41, LOGANDII@47, RETN@49.
        let ins = asm(&[
            ("JSR", vec![AsmArg::JumpAbs(21)]),
            ("RETN", vec![]),
            ("CONSTI", vec![AsmArg::Int(1)]),
            ("CPTOPSP", vec![AsmArg::Int(-4), AsmArg::Int(4)]),
            ("JZ", vec![AsmArg::JumpAbs(49)]),
            ("CONSTI", vec![AsmArg::Int(1)]),
            ("LOGANDII", vec![]),
            ("RETN", vec![]),
        ]);
        let p = split(&ins).unwrap();
        let cfg = analyze(&ins, &p.main, &[]);
        let jz = idx_at(&ins, 35);
        assert!(cfg.and_guards.contains(&jz));
        assert!(cfg.log_or_extra_jz.is_empty());
    }

    #[test]
    fn origins_backward_from_while_back_edge() {
        // §5.3: backward JMP to loop head L.
        // L CPTOPSP@21, JZ@29→41, JMP@35→21, RETN@41.
        let ins = asm(&[
            ("JSR", vec![AsmArg::JumpAbs(21)]),
            ("RETN", vec![]),
            ("CPTOPSP", vec![AsmArg::Int(-4), AsmArg::Int(4)]),
            ("JZ", vec![AsmArg::JumpAbs(41)]),
            ("JMP", vec![AsmArg::JumpAbs(21)]),
            ("RETN", vec![]),
        ]);
        let p = split(&ins).unwrap();
        let cfg = analyze(&ins, &p.main, &[]);
        let head = idx_at(&ins, 21);
        let back = idx_at(&ins, 35);
        assert_eq!(
            cfg.origins_backward.get(&head).map(Vec::as_slice),
            Some(&[back][..])
        );
    }

    #[test]
    fn store_state_body_is_reachable() {
        // JMP after STORE_STATE skips the body; the body must still be live (§5.9 / §5.10).
        let ins = asm(&[
            ("JSR", vec![AsmArg::JumpAbs(21)]),
            ("RETN", vec![]),
            ("STORE_STATE", vec![AsmArg::Int(0), AsmArg::Int(8)]),
            ("JMP", vec![AsmArg::JumpAbs(44)]),
            ("ACTION", vec![AsmArg::Int(6), AsmArg::Int(2)]),
            ("RETN", vec![]),
            ("RETN", vec![]),
        ]);
        let p = split(&ins).unwrap();
        let cfg = analyze(&ins, &p.main, &p.deferred);
        let body = idx_at(&ins, 37);
        assert!(!cfg.dead[body]);
        assert_eq!(cfg.deferred.len(), 1);
    }

    #[test]
    fn block_end_is_predecessor_not_target_minus_six() {
        // Then-block ends on ACTION (5 bytes). target-6 is not an instruction start (R11).
        // CONSTI@21, JZ@27→38, ACTION@33, RETN@38.
        let ins = asm(&[
            ("JSR", vec![AsmArg::JumpAbs(21)]),
            ("RETN", vec![]),
            ("CONSTI", vec![AsmArg::Int(0)]),
            ("JZ", vec![AsmArg::JumpAbs(38)]),
            ("ACTION", vec![AsmArg::Int(6), AsmArg::Int(0)]),
            ("RETN", vec![]),
        ]);
        let p = split(&ins).unwrap();
        let cfg = analyze(&ins, &p.main, &[]);
        let jz = idx_at(&ins, 27);
        let action = idx_at(&ins, 33);
        let end = cfg.block_ends.get(&jz).expect("block end for JZ");
        assert_eq!(end.last, action);
        assert_eq!(ins[end.last].offset, 33);
        assert_ne!(ins[end.last].offset, 38 - 6);
    }
}
