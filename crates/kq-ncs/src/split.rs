//! Cut a flat instruction list into header + subroutine ranges (DeNCS §2).

use std::collections::HashSet;
use std::ops::Range;

use kq_format::ncs::{Arg, Instruction};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SplitProgram {
    /// Instruction indices, inclusive of the header RETN (half-open range).
    pub header: Range<usize>,
    /// RSADD* before the header JSR.
    pub conditional_header: bool,
    pub globals: Option<SubRange>,
    pub main: SubRange,
    /// User functions id 1.. in file order.
    pub users: Vec<SubRange>,
    /// STORE_STATE blocks, file order, nested via the pending stack.
    pub deferred: Vec<DeferredRegion>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubRange {
    pub kind: SubKind,
    pub range: Range<usize>,
    pub start_pos: u32,
    pub end_pos: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubKind {
    Globals,
    Main,
    User(u16),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeferredRegion {
    pub store_idx: usize,
    pub body: Range<usize>,
    pub after_pos: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SplitError {
    Empty,
    MissingHeaderRetn,
    NoSubroutines,
    StoreStateWithoutJmp { idx: usize },
    StoreStateBadTarget { idx: usize },
    UnclosedStoreState,
    UnterminatedSub,
    JsrTargetNotSub { jsr_pos: u32, target: u32 },
}

struct PendingBlock {
    store_idx: usize,
    body_start: usize,
    retn_pos: u32,
    after_pos: u32,
}

/// Split `ins` into header, optional globals, main, user subs, and deferred STORE_STATE regions.
pub fn split(ins: &[Instruction]) -> Result<SplitProgram, SplitError> {
    if ins.is_empty() {
        return Err(SplitError::Empty);
    }

    let hdr_end = ins
        .iter()
        .position(|i| i.op == "RETN")
        .ok_or(SplitError::MissingHeaderRetn)?;
    let header = 0..hdr_end + 1;
    let conditional_header = header_is_conditional(&ins[header.clone()]);

    let mut start = hdr_end + 1;
    if start >= ins.len() {
        return Err(SplitError::NoSubroutines);
    }

    let mut raw_subs: Vec<Range<usize>> = Vec::new();
    let mut deferred: Vec<DeferredRegion> = Vec::new();
    let mut pending: Vec<PendingBlock> = Vec::new();

    for i in start..ins.len() {
        match ins[i].op {
            "STORE_STATE" => {
                let jmp = ins
                    .get(i + 1)
                    .ok_or(SplitError::StoreStateWithoutJmp { idx: i })?;
                if jmp.op != "JMP" {
                    return Err(SplitError::StoreStateWithoutJmp { idx: i });
                }
                let after_pos =
                    jump_target(jmp).ok_or(SplitError::StoreStateBadTarget { idx: i })?;
                let retn_pos = after_pos
                    .checked_sub(2)
                    .ok_or(SplitError::StoreStateBadTarget { idx: i })?;
                pending.push(PendingBlock {
                    store_idx: i,
                    body_start: i + 2,
                    retn_pos,
                    after_pos,
                });
            }
            "RETN" => {
                if pending.last().map(|p| p.retn_pos) == Some(ins[i].offset) {
                    let block = pending.pop().expect("matched pending STORE_STATE");
                    deferred.push(DeferredRegion {
                        store_idx: block.store_idx,
                        body: block.body_start..i + 1,
                        after_pos: block.after_pos,
                    });
                } else {
                    raw_subs.push(start..i + 1);
                    start = i + 1;
                }
            }
            _ => {}
        }
    }

    if !pending.is_empty() {
        return Err(SplitError::UnclosedStoreState);
    }
    if start != ins.len() {
        return Err(SplitError::UnterminatedSub);
    }
    if raw_subs.is_empty() {
        return Err(SplitError::NoSubroutines);
    }

    deferred.sort_by_key(|d| d.store_idx);

    let mut iter = raw_subs.into_iter();
    let first = iter.next().expect("non-empty raw_subs");
    let rest: Vec<Range<usize>> = iter.collect();

    let (globals, main_range, user_ranges) = if !rest.is_empty() && is_globals_sub(ins, &first) {
        let main_range = rest[0].clone();
        (Some(first), main_range, rest[1..].to_vec())
    } else {
        (None, first, rest)
    };

    let globals = globals.map(|r| sub_range(SubKind::Globals, r, ins));
    let main = sub_range(SubKind::Main, main_range, ins);
    let users: Vec<SubRange> = user_ranges
        .into_iter()
        .enumerate()
        .map(|(i, r)| sub_range(SubKind::User((i + 1) as u16), r, ins))
        .collect();

    check_jsr_targets(ins, globals.as_ref(), &main, &users)?;

    Ok(SplitProgram {
        header,
        conditional_header,
        globals,
        main,
        users,
        deferred,
    })
}

fn header_is_conditional(header: &[Instruction]) -> bool {
    let Some(jsr) = header.iter().position(|i| i.op == "JSR") else {
        return false;
    };
    header[..jsr].iter().any(|i| i.op.starts_with("RSADD"))
}

fn is_globals_sub(ins: &[Instruction], range: &Range<usize>) -> bool {
    ins[range.clone()]
        .iter()
        .rev()
        .any(|i| i.op == "SAVEBP" || i.op == "RESTOREBP")
}

fn sub_range(kind: SubKind, range: Range<usize>, ins: &[Instruction]) -> SubRange {
    let start_pos = ins[range.start].offset;
    let end_pos = ins[range.end - 1].offset;
    SubRange {
        kind,
        range,
        start_pos,
        end_pos,
    }
}

fn jump_target(ins: &Instruction) -> Option<u32> {
    match ins.args.first() {
        Some(Arg::Jump(t)) => Some(*t),
        _ => None,
    }
}

fn check_jsr_targets(
    ins: &[Instruction],
    globals: Option<&SubRange>,
    main: &SubRange,
    users: &[SubRange],
) -> Result<(), SplitError> {
    let mut starts = HashSet::new();
    if let Some(g) = globals {
        starts.insert(g.start_pos);
    }
    starts.insert(main.start_pos);
    for u in users {
        starts.insert(u.start_pos);
    }
    for inst in ins {
        if inst.op != "JSR" {
            continue;
        }
        let target = jump_target(inst).ok_or(SplitError::JsrTargetNotSub {
            jsr_pos: inst.offset,
            target: 0,
        })?;
        if !starts.contains(&target) {
            return Err(SplitError::JsrTargetNotSub {
                jsr_pos: inst.offset,
                target,
            });
        }
    }
    Ok(())
}
