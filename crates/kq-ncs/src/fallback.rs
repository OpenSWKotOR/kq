use std::ops::Range;

use kq_format::ncs::{Arg, Instruction, Ncs};

pub fn format_ins(ins: &Instruction) -> String {
    let mut s = format!("{:5}  {}", ins.offset, ins.op);
    if let Some(name) = ins.routine_name {
        s.push(' ');
        s.push_str(name);
        if let Some(argc) = ins.argc {
            s.push('(');
            s.push_str(&argc.to_string());
            s.push(')');
        }
    } else if !ins.args.is_empty() {
        s.push(' ');
        s.push(' ');
        for (i, arg) in ins.args.iter().enumerate() {
            if i > 0 {
                s.push(' ');
            }
            match arg {
                Arg::Jump(t) => s.push_str(&format!("->{t}")),
                Arg::Int(v) => s.push_str(&v.to_string()),
                Arg::Float(f) => s.push_str(&f.to_string()),
                Arg::Str(st) => {
                    s.push('"');
                    s.push_str(st);
                    s.push('"');
                }
            }
        }
    }
    s
}

pub fn disasm_lines(ncs: &Ncs) -> String {
    ncs.instructions
        .iter()
        .map(format_ins)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Tab-indented comment body for a subroutine that could not be built.
pub fn fallback_sub_body(ins: &[Instruction], range: Range<usize>, reason: &str) -> String {
    let mut out = String::from("\t/* kq: could not decompile this subroutine (reason: ");
    out.push_str(reason);
    out.push_str(").\n\t   Disassembly:\n");
    if let Some(slice) = ins.get(range) {
        for inst in slice {
            out.push_str("\t     ");
            out.push_str(&format_ins(inst));
            out.push('\n');
        }
    }
    out.push_str("\t*/\n");
    out
}
