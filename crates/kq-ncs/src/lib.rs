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
pub use emit::{emit_program, format_float};
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
        let Some(info) = protos.get(&id) else {
            return fallback_decompile(ncs, "missing inferred prototype");
        };
        let Some(cfg) = cfgs.get(&id) else {
            return fallback_decompile(ncs, "missing CFG");
        };
        match build_sub(&ncs.instructions, info, cfg, &globals, &protos, game) {
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
                let mut info = info.clone();
                if id == SubId::Main && program.conditional_header {
                    info.ret = Ty::Int;
                }
                items.push((id, info, block));
            }
            Err(err) => {
                return fallback_decompile(ncs, &format!("build failed at {id:?}: {err:?}"));
            }
        }
    }

    let Some(structs) = structs else {
        return fallback_decompile(ncs, "no functions emitted");
    };
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
