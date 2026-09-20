//! NCS → NSS decompiler (DeNCS algorithm port).

mod actions;
mod actions_gen;
mod ast;
mod build;
mod cfg;
mod cleanup;
mod emit;
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
pub use cleanup::{cleanup, VarTable};
pub use emit::{emit_program, format_float, EmitBody};
pub use fallback::fallback_sub_body;
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

    if ncs.instructions.is_empty() {
        return Decompiled {
            source: "/* kq: empty NCS */\n".into(),
            complete: false,
            warnings: Vec::new(),
            subs: Vec::new(),
        };
    }

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

    let mut items = Vec::new();
    let mut reports = Vec::new();
    let mut structs: Option<StructTable> = None;
    let mut ranges = program
        .users
        .iter()
        .filter_map(|range| match range.kind {
            SubKind::User(id) => Some((SubId::User(id), range)),
            _ => None,
        })
        .collect::<Vec<_>>();
    ranges.sort_by_key(|(id, _)| match id {
        SubId::User(n) => *n,
        _ => u16::MAX,
    });
    ranges.push((SubId::Main, &program.main));
    for (id, range) in ranges {
        let info = match protos.get(&id) {
            Some(info) => {
                let mut info = info.clone();
                if id == SubId::Main && program.conditional_header {
                    info.ret = Ty::Int;
                }
                info
            }
            None => synthetic_sub_info(id, range),
        };
        let Some(cfg) = cfgs.get(&id) else {
            push_fallback_sub(
                &mut reports,
                &mut items,
                &ncs.instructions,
                id,
                range,
                info,
                "missing CFG".into(),
            );
            continue;
        };
        match build_sub(&ncs.instructions, &info, cfg, &globals, &protos, game) {
            Ok((mut block, mut vars, sub_structs)) => {
                cleanup::cleanup(&mut block, &mut vars);
                match &mut structs {
                    Some(acc) => acc.absorb(sub_structs),
                    None => structs = Some(sub_structs),
                }
                reports.push(SubReport {
                    id,
                    start: range.start_pos,
                    end: range.end_pos,
                    status: SubStatus::Ok,
                });
                items.push((id, info, emit::EmitBody::Built(block)));
            }
            Err(err) => {
                push_fallback_sub(
                    &mut reports,
                    &mut items,
                    &ncs.instructions,
                    id,
                    range,
                    info,
                    format_build_error(&err),
                );
            }
        }
    }

    if items.is_empty() {
        return fallback_decompile(ncs, "no functions emitted");
    }
    let structs = structs.unwrap_or_default();
    let globals_ref = program.globals.as_ref().map(|_| &globals);
    let source = emit::emit_program(&structs, globals_ref, &items);
    if source.is_empty() {
        warnings.push(Warning {
            sub: None,
            pos: None,
            severity: Severity::Warn,
            msg: "no functions emitted".into(),
        });
        return fallback_decompile(ncs, "no functions emitted");
    }
    let complete = reports.iter().all(|s| matches!(s.status, SubStatus::Ok))
        && warnings.iter().all(|w| w.severity != Severity::Error);
    Decompiled {
        source,
        complete,
        warnings,
        subs: reports,
    }
}

fn push_fallback_sub(
    reports: &mut Vec<SubReport>,
    items: &mut Vec<(SubId, SubInfo, emit::EmitBody)>,
    ins: &[kq_format::ncs::Instruction],
    id: SubId,
    range: &SubRange,
    info: SubInfo,
    reason: String,
) {
    let body = fallback::fallback_sub_body(ins, range.range.clone(), &reason);
    reports.push(SubReport {
        id,
        start: range.start_pos,
        end: range.end_pos,
        status: SubStatus::Fallback(reason),
    });
    items.push((id, info, emit::EmitBody::Fallback(body)));
}

fn format_build_error(err: &BuildError) -> String {
    match err {
        BuildError::StackUnderflow { pos } => format!("unbalanced stack at {pos:#x}"),
        BuildError::BadOperand { pos } => format!("bad operand at {pos:#x}"),
        BuildError::BadJump { pos } => format!("bad jump at {pos:#x}"),
        BuildError::Unsupported { pos, op } => format!("unsupported {op} at {pos:#x}"),
    }
}

fn synthetic_sub_info(id: SubId, range: &SubRange) -> SubInfo {
    SubInfo {
        range: range.range.clone(),
        start_pos: range.start_pos,
        kind: match id {
            SubId::Header | SubId::Main => SubKind::Main,
            SubId::Globals => SubKind::Globals,
            SubId::User(n) => SubKind::User(n),
        },
        ret: Ty::Void,
        ret_slots: 0,
        ret_depth: 0,
        param_count: 0,
        params: Vec::new(),
        params_typed: false,
    }
}

fn fallback_decompile(ncs: &kq_format::ncs::Ncs, reason: &str) -> Decompiled {
    let body = fallback::fallback_sub_body(&ncs.instructions, 0..ncs.instructions.len(), reason);
    let source = format!("void main() {{\n{body}}}\n");
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

    fn inst(
        offset: u32,
        op: &'static str,
        args: Vec<kq_format::ncs::Arg>,
    ) -> kq_format::ncs::Instruction {
        kq_format::ncs::Instruction {
            offset,
            op,
            args,
            routine: None,
            routine_name: None,
            argc: None,
        }
    }

    /// Header JSR → empty main; user sub is ADDII with an empty stack (build fails).
    fn ncs_broken_user_ok_main() -> Ncs {
        use kq_format::ncs::Arg;
        Ncs {
            declared_size: 27,
            instructions: vec![
                inst(13, "JSR", vec![Arg::Jump(21)]),
                inst(19, "RETN", vec![]),
                inst(21, "RETN", vec![]),
                inst(23, "ADDII", vec![]),
                inst(25, "RETN", vec![]),
            ],
        }
    }

    #[test]
    fn build_failure_falls_back_per_sub() {
        let d = decompile(&ncs_broken_user_ok_main(), Game::K1);
        assert!(
            !d.complete,
            "complete={}; source=\n{}",
            d.complete, d.source
        );
        assert!(
            d.source.contains("/* kq: could not decompile"),
            "{}",
            d.source
        );
        assert!(d.source.contains("Disassembly:"), "{}", d.source);
        assert!(
            d.subs
                .iter()
                .any(|s| matches!(s.status, SubStatus::Fallback(_))),
            "{:#?}",
            d.subs
        );
        assert!(
            d.subs
                .iter()
                .any(|s| s.id == SubId::Main && s.status == SubStatus::Ok),
            "main should still be Ok; subs={:#?}",
            d.subs
        );
        assert!(d.source.contains("void main()"), "{}", d.source);
        assert!(
            d.source.contains("sub1"),
            "prototype missing:\n{}",
            d.source
        );
        assert!(
            !d.source.contains("__unknown"),
            "placeholder identifier:\n{}",
            d.source
        );
    }

    #[test]
    fn empty_ncs_is_comment_not_err() {
        let d = decompile(
            &Ncs {
                declared_size: 13,
                instructions: Vec::new(),
            },
            Game::K1,
        );
        assert!(!d.complete);
        assert_eq!(d.source.trim(), "/* kq: empty NCS */");
    }
}
