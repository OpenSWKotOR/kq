//! Shared NCS assembler for decompiler tests.
//!
//! Offsets start at 13 (after the NCS header). Sizes come from `kotor-ncs-isa`.

use kq_format::ncs::{Arg, Instruction};

#[derive(Clone, Debug)]
pub enum AsmArg {
    JumpAbs(u32),
    Int(i64),
    #[allow(dead_code)]
    Float(f64),
    #[allow(dead_code)]
    Str(String),
}

/// Build instructions with automatic absolute offsets starting at 13.
pub fn asm(lines: &[(&str, Vec<AsmArg>)]) -> Vec<Instruction> {
    let mut offset = 13u32;
    let mut out = Vec::with_capacity(lines.len());
    for (op, args) in lines {
        let spec =
            kotor_ncs_isa::lookup_mnemonic(op).unwrap_or_else(|| panic!("unknown mnemonic {op}"));
        let size = match spec.operands.byte_len() {
            Some(n) => 2 + n as u32,
            None => {
                let len = args
                    .iter()
                    .find_map(|a| match a {
                        AsmArg::Str(s) => Some(s.len()),
                        _ => None,
                    })
                    .unwrap_or(0);
                2 + 2 + len as u32
            }
        };

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
                AsmArg::Float(f) => ins.args.push(Arg::Float(*f)),
                AsmArg::Str(s) => ins.args.push(Arg::Str(s.clone())),
            }
        }
        if *op == "ACTION" {
            if let Some(Arg::Int(id)) = ins.args.first() {
                let id = *id as u16;
                ins.routine = Some(id);
                ins.routine_name = kq_format::ncs_actions::name(id);
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
