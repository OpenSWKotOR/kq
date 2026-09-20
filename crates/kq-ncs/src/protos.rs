//! Prototype inference: callee scan, call graph, Tarjan SCC (DeNCS §4.3 / §4.5).

use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::Range;

use kq_format::ncs::{Arg, Instruction};

use crate::actions::action;
use crate::cfg::Cfg;
use crate::split::{SplitProgram, SubKind, SubRange};
use crate::stack::{stack_offset_to_pos, stack_size_to_pos};
use crate::ty::Ty;
use crate::{Game, Severity, SubId, Warning};

const MAX_PASSES: usize = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubInfo {
    pub range: Range<usize>,
    pub start_pos: u32,
    pub kind: SubKind,
    pub ret: Ty,
    pub ret_slots: usize,
    pub ret_depth: i32,
    pub param_count: usize, // slots
    pub params: Vec<Ty>,    // bottom-first; default Int placeholders
    pub params_typed: bool,
}

pub fn infer_prototypes(
    ins: &[Instruction],
    split: &SplitProgram,
    cfgs: &HashMap<SubId, Cfg>,
    game: Game,
) -> (HashMap<SubId, SubInfo>, Vec<Warning>) {
    let subs = collect_subs(split);
    let by_pos = start_pos_map(&subs);
    let nodes: Vec<SubId> = subs.iter().map(|(id, _)| *id).collect();
    let edges = call_graph(ins, &subs, cfgs, &by_pos);
    let sccs = tarjan_sccs(&nodes, &edges);
    let site_est = call_site_estimates(ins, &subs, cfgs, &by_pos);

    let mut infos: HashMap<SubId, SubInfo> = HashMap::new();
    let mut warnings = Vec::new();

    for scc in &sccs {
        for _round in 0..MAX_PASSES {
            let mut changed = false;
            for &id in scc {
                let sub = match subs.iter().find(|(sid, _)| *sid == id) {
                    Some((_, s)) => *s,
                    None => continue,
                };
                let next = infer_one(
                    ins,
                    id,
                    sub,
                    cfgs.get(&id),
                    &by_pos,
                    &infos,
                    game,
                    &mut warnings,
                );
                changed |= merge_info(id, next, &mut infos, &mut warnings);
            }
            if !changed {
                break;
            }
        }
        for &id in scc {
            finalize_return(id, &site_est, &mut infos, &mut warnings);
        }
    }

    cross_check(&infos, &site_est, &mut warnings);

    (infos, warnings)
}

fn collect_subs(split: &SplitProgram) -> Vec<(SubId, &SubRange)> {
    let mut out = Vec::new();
    if let Some(g) = &split.globals {
        out.push((SubId::Globals, g));
    }
    out.push((SubId::Main, &split.main));
    for u in &split.users {
        out.push((sub_id(u.kind), u));
    }
    out
}

fn sub_id(kind: SubKind) -> SubId {
    match kind {
        SubKind::Globals => SubId::Globals,
        SubKind::Main => SubId::Main,
        SubKind::User(n) => SubId::User(n),
    }
}

fn start_pos_map(subs: &[(SubId, &SubRange)]) -> HashMap<u32, SubId> {
    subs.iter().map(|(id, s)| (s.start_pos, *id)).collect()
}

fn call_graph(
    ins: &[Instruction],
    subs: &[(SubId, &SubRange)],
    cfgs: &HashMap<SubId, Cfg>,
    by_pos: &HashMap<u32, SubId>,
) -> HashMap<SubId, Vec<SubId>> {
    let mut edges: HashMap<SubId, Vec<SubId>> = HashMap::new();
    for (id, sub) in subs {
        let mut dests = Vec::new();
        let mut seen = HashSet::new();
        let dead = cfgs.get(id).map(|c| c.dead.as_slice());
        for i in sub.range.clone() {
            if dead
                .map(|d| d.get(i).copied().unwrap_or(true))
                .unwrap_or(false)
            {
                continue;
            }
            if ins[i].op != "JSR" {
                continue;
            }
            if let Some(t) = arg_jump(&ins[i]) {
                if let Some(&callee) = by_pos.get(&t) {
                    if seen.insert(callee) {
                        dests.push(callee);
                    }
                }
            }
        }
        edges.insert(*id, dests);
    }
    edges
}

fn tarjan_sccs(nodes: &[SubId], edges: &HashMap<SubId, Vec<SubId>>) -> Vec<Vec<SubId>> {
    let mut index: HashMap<SubId, i32> = HashMap::new();
    let mut lowlink: HashMap<SubId, i32> = HashMap::new();
    let mut on_stack: HashSet<SubId> = HashSet::new();
    let mut stack: Vec<SubId> = Vec::new();
    let mut next_index = 0i32;
    let mut sccs = Vec::new();

    fn strongconnect(
        v: SubId,
        edges: &HashMap<SubId, Vec<SubId>>,
        index: &mut HashMap<SubId, i32>,
        lowlink: &mut HashMap<SubId, i32>,
        on_stack: &mut HashSet<SubId>,
        stack: &mut Vec<SubId>,
        next_index: &mut i32,
        sccs: &mut Vec<Vec<SubId>>,
    ) {
        index.insert(v, *next_index);
        lowlink.insert(v, *next_index);
        *next_index += 1;
        stack.push(v);
        on_stack.insert(v);

        for &w in edges.get(&v).map(|v| v.as_slice()).unwrap_or(&[]) {
            if !index.contains_key(&w) {
                strongconnect(w, edges, index, lowlink, on_stack, stack, next_index, sccs);
                let lw = lowlink[&w];
                let lv = lowlink[&v];
                lowlink.insert(v, lv.min(lw));
            } else if on_stack.contains(&w) {
                let iw = index[&w];
                let lv = lowlink[&v];
                lowlink.insert(v, lv.min(iw));
            }
        }

        if lowlink[&v] == index[&v] {
            let mut scc = Vec::new();
            loop {
                let w = stack.pop().expect("tarjan stack");
                on_stack.remove(&w);
                scc.push(w);
                if w == v {
                    break;
                }
            }
            sccs.push(scc);
        }
    }

    for &v in nodes {
        if !index.contains_key(&v) {
            strongconnect(
                v,
                edges,
                &mut index,
                &mut lowlink,
                &mut on_stack,
                &mut stack,
                &mut next_index,
                &mut sccs,
            );
        }
    }
    sccs
}

fn merge_info(
    id: SubId,
    next: SubInfo,
    infos: &mut HashMap<SubId, SubInfo>,
    warnings: &mut Vec<Warning>,
) -> bool {
    let Some(cur) = infos.get_mut(&id) else {
        infos.insert(id, next);
        return true;
    };
    let mut changed = false;
    if cur.param_count == 0 && next.param_count > 0 {
        cur.param_count = next.param_count;
        cur.params = next.params.clone();
        cur.params_typed = next.params_typed;
        changed = true;
    } else if next.param_count > 0 && next.param_count != cur.param_count {
        warnings.push(warn(
            id,
            Some(cur.start_pos),
            format!(
                "param_count changed {} -> {}; keeping first non-zero",
                cur.param_count, next.param_count
            ),
        ));
    } else if next.params_typed && !cur.params_typed {
        cur.params = next.params.clone();
        cur.params_typed = true;
        changed = true;
    } else if next.params != cur.params && next.params.iter().any(|t| *t != Ty::Int) {
        for (dst, src) in cur.params.iter_mut().zip(next.params.iter()) {
            if *dst == Ty::Int && *src != Ty::Int && *src != Ty::Unknown {
                *dst = *src;
                changed = true;
            }
        }
        if cur.params.iter().all(|t| *t != Ty::Unknown) && !cur.params.is_empty() {
            cur.params_typed = next.params_typed;
        }
    }
    if cur.ret == Ty::Unknown && next.ret != Ty::Unknown {
        cur.ret = next.ret;
        cur.ret_slots = next.ret_slots;
        cur.ret_depth = next.ret_depth;
        changed = true;
    } else if cur.ret == Ty::Void && next.ret != Ty::Void && next.ret != Ty::Unknown {
        cur.ret = next.ret;
        cur.ret_slots = next.ret_slots;
        cur.ret_depth = next.ret_depth;
        changed = true;
    }
    changed
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Slot {
    ty: Ty,
    /// 1-based depth below the frame when this slot is a param copy.
    param_depth: Option<usize>,
    /// A type conflict is distinct from an unresolved type and must stay unknown.
    type_conflict: bool,
}

impl Slot {
    fn new(ty: Ty) -> Self {
        Self {
            ty,
            param_depth: None,
            type_conflict: false,
        }
    }

    fn param(ty: Ty, depth: usize) -> Self {
        Self {
            ty,
            param_depth: Some(depth),
            type_conflict: false,
        }
    }
}

struct Walk {
    param_height0: Option<usize>,
    min_below: Option<usize>,
    last_movsp: Option<usize>,
    max_param_depth: usize,
    below_cpdown: Vec<BelowCp>,
    param_ty: HashMap<usize, Ty>,
    param_seen: HashSet<usize>,
}

struct BelowCp {
    depth: usize,
    ty: Ty,
    slots: usize,
}

fn infer_one(
    ins: &[Instruction],
    id: SubId,
    sub: &SubRange,
    cfg: Option<&Cfg>,
    by_pos: &HashMap<u32, SubId>,
    known: &HashMap<SubId, SubInfo>,
    game: Game,
    warnings: &mut Vec<Warning>,
) -> SubInfo {
    let mut walk = Walk {
        param_height0: None,
        min_below: None,
        last_movsp: None,
        max_param_depth: 0,
        below_cpdown: Vec::new(),
        param_ty: HashMap::new(),
        param_seen: HashSet::new(),
    };

    let range = sub.range.clone();
    if range.start >= ins.len() || range.start >= range.end {
        return finish_info(sub, &walk);
    }

    let mut stack_in: HashMap<usize, Vec<Slot>> = HashMap::new();
    let mut q = VecDeque::new();
    let entry = range.start;
    if cfg
        .map(|c| c.dead.get(entry).copied().unwrap_or(true))
        .unwrap_or(false)
    {
        return finish_info(sub, &walk);
    }
    stack_in.insert(entry, Vec::new());
    q.push_back(entry);

    while let Some(i) = q.pop_front() {
        let mut stack = stack_in.get(&i).cloned().unwrap_or_default();
        apply(ins, i, &mut stack, &mut walk, by_pos, known, game);

        let succs: Vec<usize> = if let Some(cfg) = cfg {
            cfg.succ.get(i).cloned().unwrap_or_default()
        } else if ins[i].op == "RETN" || i + 1 >= range.end {
            Vec::new()
        } else {
            vec![i + 1]
        };
        for s in succs {
            if !range.contains(&s) {
                continue;
            }
            if cfg
                .map(|c| c.dead.get(s).copied().unwrap_or(false))
                .unwrap_or(false)
            {
                continue;
            }
            match stack_in.get(&s) {
                None => {
                    stack_in.insert(s, stack.clone());
                    q.push_back(s);
                }
                Some(old) => {
                    let mut merged = old.clone();
                    match merge_stack_state(&mut merged, &stack) {
                        Ok(true) => {
                            stack_in.insert(s, merged);
                            q.push_back(s);
                        }
                        Ok(false) => {}
                        Err((old_height, incoming_height)) => {
                            push_warning_unique(
                                warnings,
                                warn(
                                    id,
                                    Some(ins[s].offset),
                                    format!(
                                        "irreconcilable stack heights at join: \
                                         {old_height} and {incoming_height}"
                                    ),
                                ),
                            );
                        }
                    }
                }
            }
        }
    }

    finish_info(sub, &walk)
}

fn apply(
    ins: &[Instruction],
    i: usize,
    stack: &mut Vec<Slot>,
    walk: &mut Walk,
    by_pos: &HashMap<u32, SubId>,
    known: &HashMap<SubId, SubInfo>,
    game: Game,
) {
    let inst = &ins[i];
    match inst.op {
        "MOVSP" => {
            let popped = stack_offset_to_pos(arg_int(inst, 0));
            walk.last_movsp = Some(popped);
            let height = stack.len();
            if height == 0 {
                if popped > 0 {
                    walk.param_height0 = Some(popped);
                }
            } else if popped > height {
                let below = popped - height;
                walk.min_below = Some(walk.min_below.map(|m| m.min(below)).unwrap_or(below));
                stack.clear();
            } else {
                stack.truncate(height - popped);
            }
        }
        "CPTOPSP" => {
            let loc = stack_offset_to_pos(arg_int(inst, 0));
            let copy = stack_size_to_pos(arg_int(inst, 1));
            let height = stack.len();
            if loc > height {
                let depth0 = loc - height;
                walk.max_param_depth = walk.max_param_depth.max(depth0 + copy.saturating_sub(1));
                for k in 0..copy {
                    let depth = depth0 + k;
                    walk.param_seen.insert(depth);
                    let ty = walk.param_ty.get(&depth).copied().unwrap_or(Ty::Int);
                    stack.push(Slot::param(ty, depth));
                }
            } else if loc > 0 && copy > 0 && copy <= loc && loc <= height {
                let first = loc - copy + 1;
                let mut to_push = Vec::with_capacity(copy);
                for pos in (first..=loc).rev() {
                    to_push.push(stack[height - pos]);
                }
                stack.extend(to_push);
            }
        }
        "CPDOWNSP" => {
            let loc = stack_offset_to_pos(arg_int(inst, 0));
            let copy = stack_size_to_pos(arg_int(inst, 1)).max(1);
            let height = stack.len();
            if loc > height {
                let depth = loc - height;
                let ty = ret_ty_from_top(stack, copy);
                walk.below_cpdown.push(BelowCp {
                    depth,
                    ty,
                    slots: copy,
                });
                // May be a param write; classified after param_count is known.
                if let Some(top) = stack.last() {
                    walk.param_ty.entry(depth).or_insert(top.ty);
                    walk.param_seen.insert(depth);
                }
            }
        }
        "JSR" => {
            let Some(target) = arg_jump(inst) else {
                return;
            };
            let Some(&callee) = by_pos.get(&target) else {
                return;
            };
            if let Some(info) = known.get(&callee) {
                pop_n(stack, info.param_count);
                push_return_slots(stack, info.ret, info.ret_slots);
            }
        }
        "ACTION" => apply_action(inst, stack, walk, game),
        "JZ" | "JNZ" => {
            pop_n(stack, 1);
        }
        "DESTRUCT" => apply_destruct(inst, stack),
        "INCxSP" | "DECxSP" => {
            let loc = stack_offset_to_pos(arg_int(inst, 0));
            let height = stack.len();
            if loc > height {
                let depth = loc - height;
                walk.max_param_depth = walk.max_param_depth.max(depth);
                walk.param_ty.insert(depth, Ty::Int);
                walk.param_seen.insert(depth);
            }
        }
        "EQUALTT" | "NEQUALTT" => {
            let slots = stack_size_to_pos(arg_int(inst, 0)).max(1);
            pop_n(stack, 2 * slots);
            stack.push(Slot::new(Ty::Int));
        }
        "ADDVV" | "SUBVV" => {
            pop_n(stack, 6);
            for _ in 0..3 {
                stack.push(Slot::new(Ty::Float));
            }
        }
        "MULVF" | "DIVVF" | "MULFV" | "DIVFV" => {
            pop_n(stack, 4);
            for _ in 0..3 {
                stack.push(Slot::new(Ty::Float));
            }
        }
        op if ty_from_op(op).is_some() => {
            stack.push(Slot::new(ty_from_op(op).unwrap()));
        }
        op if is_unary(op) => {}
        op if is_binary(op) => {
            let ret = binary_ret(op);
            pop_n(stack, 2);
            stack.push(Slot::new(ret));
        }
        _ => {}
    }
}

fn apply_action(inst: &Instruction, stack: &mut Vec<Slot>, walk: &mut Walk, game: Game) {
    let id = inst.routine.unwrap_or(arg_int(inst, 0) as u16);
    let argc = inst.argc.unwrap_or(arg_int(inst, 1) as u8);
    let (ret_ty, ret_slots) = match action(game, id) {
        Some(sig) => {
            let mut slots = 0usize;
            let mut arg_tys = Vec::new();
            for (i, p) in sig.params.iter().enumerate() {
                if i >= argc as usize {
                    break;
                }
                let n = match p.ty {
                    Ty::Void | Ty::Action => 0,
                    Ty::Vector => 3,
                    _ => 1,
                };
                slots += n;
                arg_tys.push((n, p.ty));
            }
            let popped = pop_n(stack, slots);
            // pop_n preserves bottom-to-top order, matching declared parameters.
            let mut idx = 0usize;
            for (n, ty) in arg_tys {
                for _ in 0..n {
                    if idx < popped.len() {
                        if let Some(d) = popped[idx].param_depth {
                            if ty != Ty::Action && ty != Ty::Void {
                                walk.param_ty.insert(d, ty);
                                walk.param_seen.insert(d);
                            }
                        }
                    }
                    idx += 1;
                }
            }
            let ret_slots = match sig.ret {
                Ty::Void | Ty::Action => 0,
                Ty::Vector => 3,
                _ => 1,
            };
            (sig.ret, ret_slots)
        }
        None => {
            pop_n(stack, argc as usize);
            (Ty::Void, 0)
        }
    };
    match ret_slots {
        0 => {}
        3 if ret_ty == Ty::Vector => {
            for _ in 0..3 {
                stack.push(Slot::new(Ty::Float));
            }
        }
        _ => stack.push(Slot::new(ret_ty)),
    }
}

fn apply_destruct(inst: &Instruction, stack: &mut Vec<Slot>) {
    let rem = stack_size_to_pos(arg_int(inst, 0));
    let off = stack_size_to_pos(arg_int(inst, 1));
    let save = stack_size_to_pos(arg_int(inst, 2));
    if rem > stack.len() || off + save > rem {
        return;
    }
    let mut kept = Vec::with_capacity(save);
    for i in 0..save {
        let pos = 1 + off + i;
        kept.push(stack[stack.len() - pos]);
    }
    pop_n(stack, rem);
    for s in kept.into_iter().rev() {
        stack.push(s);
    }
}

fn finish_info(sub: &SubRange, walk: &Walk) -> SubInfo {
    let mut param_count = walk.param_height0.unwrap_or(0);
    if param_count == 0 {
        if let Some(m) = walk.min_below {
            param_count = m;
        }
    }
    if param_count == 0 && walk.max_param_depth > 0 {
        param_count = walk
            .last_movsp
            .unwrap_or(walk.max_param_depth)
            .max(walk.max_param_depth);
    }
    let mut ret = Ty::Unknown;
    let mut ret_slots = 0;
    let mut ret_depth = 0i32;
    for c in &walk.below_cpdown {
        if c.depth == param_count + c.slots || c.depth == param_count + 1 {
            ret = c.ty;
            ret_slots = c.slots;
            ret_depth = c.depth as i32;
        }
    }

    let mut params = vec![Ty::Int; param_count];
    let mut typed = vec![false; param_count];
    for (&depth, &ty) in &walk.param_ty {
        if depth == 0 || depth > param_count {
            continue;
        }
        // depth 1 = immediately below frame = last declared = index param_count-1
        let idx = param_count - depth;
        if ty != Ty::Unknown {
            params[idx] = ty;
            typed[idx] = walk.param_seen.contains(&depth);
        }
    }
    let params_typed = param_count == 0 || typed.iter().all(|&t| t);

    SubInfo {
        range: sub.range.clone(),
        start_pos: sub.start_pos,
        kind: sub.kind,
        ret,
        ret_slots,
        ret_depth,
        param_count,
        params,
        params_typed,
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct CallSiteEstimate {
    max_growth: usize,
    calls: usize,
}

fn call_site_estimates(
    ins: &[Instruction],
    subs: &[(SubId, &SubRange)],
    cfgs: &HashMap<SubId, Cfg>,
    by_pos: &HashMap<u32, SubId>,
) -> HashMap<SubId, CallSiteEstimate> {
    let mut est: HashMap<SubId, CallSiteEstimate> = HashMap::new();
    for (id, sub) in subs {
        let cfg = cfgs.get(id);
        let mut growth_at: HashMap<usize, usize> = HashMap::new();
        let mut q = VecDeque::new();
        let entry = sub.range.start;
        if entry >= ins.len() {
            continue;
        }
        if cfg
            .map(|c| c.dead.get(entry).copied().unwrap_or(true))
            .unwrap_or(false)
        {
            continue;
        }
        growth_at.insert(entry, 0);
        q.push_back(entry);
        let mut seen = HashSet::new();
        while let Some(i) = q.pop_front() {
            if !seen.insert(i) {
                continue;
            }
            let mut growth = growth_at.get(&i).copied().unwrap_or(0);
            let inst = &ins[i];
            match inst.op {
                op if ty_from_op(op).is_some() => growth += 1,
                "CPTOPSP" | "CPTOPBP" => {
                    growth += stack_size_to_pos(arg_int(inst, 1));
                }
                "MOVSP" | "DESTRUCT" => growth = 0,
                "JMP" => {
                    if arg_jump(inst).is_some_and(|t| t < inst.offset) {
                        growth = 0;
                    }
                }
                "JZ" | "JNZ" => growth = growth.saturating_sub(1),
                "JSR" => {
                    if let Some(t) = arg_jump(inst) {
                        if let Some(&callee) = by_pos.get(&t) {
                            let entry = est.entry(callee).or_default();
                            entry.max_growth = entry.max_growth.max(growth);
                            entry.calls += 1;
                        }
                    }
                    growth = 0;
                }
                "ACTION" => growth = 0,
                _ => {}
            }
            let succs: Vec<usize> = cfg
                .map(|c| c.succ.get(i).cloned().unwrap_or_default())
                .unwrap_or_else(|| {
                    if inst.op == "RETN" || i + 1 >= sub.range.end {
                        Vec::new()
                    } else {
                        vec![i + 1]
                    }
                });
            for s in succs {
                growth_at.entry(s).or_insert(growth);
                q.push_back(s);
            }
        }
    }
    est
}

fn cross_check(
    infos: &HashMap<SubId, SubInfo>,
    site_est: &HashMap<SubId, CallSiteEstimate>,
    warnings: &mut Vec<Warning>,
) {
    for (id, info) in infos {
        if let Some(estimate) = site_est.get(id) {
            let est = estimate.max_growth.saturating_sub(info.ret_slots);
            if est > 0 && est != info.param_count {
                warnings.push(warn(
                    *id,
                    Some(info.start_pos),
                    format!(
                        "callee-derived param_count {} disagrees with call-site estimate {}",
                        info.param_count, est
                    ),
                ));
            }
        }
    }
}

fn finalize_return(
    id: SubId,
    site_est: &HashMap<SubId, CallSiteEstimate>,
    infos: &mut HashMap<SubId, SubInfo>,
    warnings: &mut Vec<Warning>,
) {
    let Some(info) = infos.get_mut(&id) else {
        return;
    };
    if info.ret != Ty::Unknown {
        return;
    }

    if !matches!(info.kind, SubKind::User(_)) {
        info.ret = Ty::Void;
        info.ret_slots = 0;
        return;
    }

    let recognized_void = site_est
        .get(&id)
        .is_some_and(|estimate| estimate.calls > 0 && estimate.max_growth <= info.param_count);
    if recognized_void {
        info.ret = Ty::Void;
        info.ret_slots = 0;
    } else {
        info.ret = Ty::Int;
        info.ret_slots = 1;
        push_warning_unique(
            warnings,
            warn(
                id,
                Some(info.start_pos),
                "unresolved return type; defaulting prototype return to int".into(),
            ),
        );
    }
}

fn warn(id: SubId, pos: Option<u32>, msg: String) -> Warning {
    Warning {
        sub: match id {
            SubId::User(n) => Some(n),
            _ => None,
        },
        pos,
        severity: Severity::Warn,
        msg,
    }
}

fn push_warning_unique(warnings: &mut Vec<Warning>, warning: Warning) {
    if warnings.iter().any(|existing| {
        existing.sub == warning.sub
            && existing.pos == warning.pos
            && existing.severity == warning.severity
            && existing.msg == warning.msg
    }) {
        return;
    }
    warnings.push(warning);
}

fn merge_stack_state(current: &mut [Slot], incoming: &[Slot]) -> Result<bool, (usize, usize)> {
    if current.len() != incoming.len() {
        return Err((current.len(), incoming.len()));
    }

    let mut changed = false;
    for (dst, src) in current.iter_mut().zip(incoming) {
        let (ty, type_conflict) = if dst.type_conflict || src.type_conflict {
            (Ty::Unknown, true)
        } else if dst.ty == src.ty {
            (dst.ty, false)
        } else if dst.ty == Ty::Unknown {
            (src.ty, false)
        } else if src.ty == Ty::Unknown {
            (dst.ty, false)
        } else {
            (Ty::Unknown, true)
        };
        let param_depth = if dst.param_depth == src.param_depth {
            dst.param_depth
        } else {
            None
        };
        let merged = Slot {
            ty,
            param_depth,
            type_conflict,
        };
        if *dst != merged {
            *dst = merged;
            changed = true;
        }
    }
    Ok(changed)
}

fn pop_n(stack: &mut Vec<Slot>, n: usize) -> Vec<Slot> {
    let k = n.min(stack.len());
    stack.split_off(stack.len() - k)
}

fn push_return_slots(stack: &mut Vec<Slot>, ty: Ty, recorded_slots: usize) {
    let (slots, slot_ty) = match ty {
        Ty::Void | Ty::Action | Ty::Unknown => (0, Ty::Unknown),
        Ty::Vector => (3, Ty::Float),
        Ty::Struct(_) => (recorded_slots.max(1), ty),
        _ => (1, ty),
    };
    for _ in 0..slots {
        stack.push(Slot::new(slot_ty));
    }
}

fn ret_ty_from_top(stack: &[Slot], slots: usize) -> Ty {
    if slots == 0 || stack.is_empty() {
        return Ty::Unknown;
    }
    if slots == 1 {
        return stack.last().map(|s| s.ty).unwrap_or(Ty::Unknown);
    }
    if slots == 3 && stack.len() >= 3 && stack[stack.len() - 3..].iter().all(|s| s.ty == Ty::Float)
    {
        return Ty::Vector;
    }
    stack.last().map(|s| s.ty).unwrap_or(Ty::Unknown)
}

fn ty_from_op(op: &str) -> Option<Ty> {
    Some(match op {
        "RSADDI" | "CONSTI" => Ty::Int,
        "RSADDF" | "CONSTF" => Ty::Float,
        "RSADDS" | "CONSTS" => Ty::Str,
        "RSADDO" | "CONSTO" => Ty::Object,
        "RSADDEFF" => Ty::Effect,
        "RSADDEVT" => Ty::Event,
        "RSADDLOC" => Ty::Location,
        "RSADDTAL" => Ty::Talent,
        _ => return None,
    })
}

fn is_unary(op: &str) -> bool {
    matches!(op, "NEGI" | "NEGF" | "NOTI" | "COMPI")
}

fn is_binary(op: &str) -> bool {
    op.starts_with("LOG")
        || op.starts_with("EQUAL")
        || op.starts_with("NEQUAL")
        || op.starts_with("GEQ")
        || op.starts_with("GT")
        || op.starts_with("LT")
        || op.starts_with("LEQ")
        || op.starts_with("SH")
        || op.starts_with("USH")
        || op.starts_with("ADD")
        || op.starts_with("SUB")
        || op.starts_with("MUL")
        || op.starts_with("DIV")
        || op.starts_with("MOD")
        || op.starts_with("INCOR")
        || op.starts_with("EXCOR")
        || op.starts_with("BOOL")
}

fn binary_ret(op: &str) -> Ty {
    if is_comparison_or_logical(op) {
        Ty::Int
    } else if op.contains("FF") || op.contains("IF") || op.contains("FI") {
        Ty::Float
    } else if op.ends_with("SS") {
        Ty::Str
    } else {
        Ty::Int
    }
}

fn is_comparison_or_logical(op: &str) -> bool {
    op.starts_with("LOG")
        || op.starts_with("EQUAL")
        || op.starts_with("NEQUAL")
        || op.starts_with("GEQ")
        || op.starts_with("GT")
        || op.starts_with("LT")
        || op.starts_with("LEQ")
        || op.starts_with("BOOL")
}

fn arg_int(ins: &Instruction, i: usize) -> i32 {
    match ins.args.get(i) {
        Some(Arg::Int(v)) => *v as i32,
        _ => 0,
    }
}

fn arg_jump(ins: &Instruction) -> Option<u32> {
    match ins.args.first() {
        Some(Arg::Jump(t)) => Some(*t),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{merge_stack_state, Slot};
    use crate::{StructId, Ty};

    #[test]
    fn stack_merge_prefers_known_type_over_unknown() {
        let mut current = vec![Slot::new(Ty::Unknown)];
        let incoming = vec![Slot::new(Ty::Object)];

        assert_eq!(merge_stack_state(&mut current, &incoming), Ok(true));
        assert_eq!(current[0].ty, Ty::Object);
    }

    #[test]
    fn struct_return_pushes_recorded_slot_count() {
        let mut stack = Vec::new();

        super::push_return_slots(&mut stack, Ty::Struct(StructId(0)), 4);

        assert_eq!(stack.len(), 4);
        assert!(stack.iter().all(|slot| slot.ty == Ty::Struct(StructId(0))));
    }
}
