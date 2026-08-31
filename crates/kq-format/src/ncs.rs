//! NCS — compiled NWScript bytecode.
//!
//! Layout (V1.0): `"NCS "`, `"V1.0"`, a magic byte `0x42`, then a **big-endian**
//! u32 file size, then a stream of (opcode, qualifier, operands) instructions.
//! Almost every multi-byte operand is big-endian — the opposite of every other
//! KotOR format. Jump offsets are relative to the start of their instruction.

use std::path::Path;

use kotor_ncs_isa::Operands;

use crate::error::Result;
use crate::ncs_actions;
use crate::reader::Reader;

const HEADER_SIZE: usize = 13;
const MAGIC: u8 = 0x42;

/// One decoded instruction, located by its file offset.
#[derive(Clone, Debug)]
pub struct Instruction {
    pub offset: u32,
    pub op: &'static str,
    pub args: Vec<Arg>,
    /// Set for ACTION: the engine-function routine id.
    pub routine: Option<u16>,
    /// Set for ACTION: `nwscript` name when the id is known.
    pub routine_name: Option<&'static str>,
    /// Set for ACTION: how many arguments the call consumes.
    pub argc: Option<u8>,
}

#[derive(Clone, Debug)]
pub enum Arg {
    Int(i64),
    Float(f64),
    Str(String),
    /// Absolute file offset of a jump target.
    Jump(u32),
}

#[derive(Clone, Debug)]
pub struct Ncs {
    pub declared_size: u32,
    pub instructions: Vec<Instruction>,
}

pub fn sniff(data: &[u8]) -> bool {
    data.starts_with(b"NCS ")
}

pub fn read(data: &[u8], path: &Path) -> Result<Ncs> {
    let mut r = Reader::new(data, path);
    r.expect_signature("NCS V1.0")?;
    r.seek(8)?;
    let magic = r.u8()?;
    if magic != MAGIC {
        return Err(r.malformed(format!(
            "invalid NCS header magic: expected 0x{MAGIC:02X}, got 0x{magic:02X}"
        )));
    }
    let declared_size = r.u32_be()?;
    if declared_size as usize > data.len() {
        return Err(r.malformed(format!(
            "NCS size field ({declared_size}) is larger than the file ({})",
            data.len()
        )));
    }
    if declared_size as usize <= HEADER_SIZE {
        return Ok(Ncs {
            declared_size,
            instructions: Vec::new(),
        });
    }

    let end = (declared_size as usize).min(data.len());
    let mut instructions = Vec::new();
    while r.position() < end {
        let offset = r.position();
        if offset + 2 > end {
            break;
        }
        // A size field that includes trailing NULs is common; do not turn
        // those into a run of RESERVED (opcode 0x00) instructions.
        if data[offset..end].iter().all(|&b| b == 0) {
            break;
        }
        match read_instruction(&mut r, offset, end) {
            Ok(ins) => instructions.push(ins),
            Err(e) => {
                // Retail files sometimes include a size that covers trailing
                // zero padding. Treat an all-zero tail as the end, not corruption.
                if data[offset..end].iter().all(|&b| b == 0) {
                    break;
                }
                return Err(e);
            }
        }
    }
    Ok(Ncs {
        declared_size,
        instructions,
    })
}

fn read_instruction(r: &mut Reader<'_>, offset: usize, end: usize) -> Result<Instruction> {
    let opcode = r.u8()?;
    let qualifier = r.u8()?;
    let decoded = kotor_ncs_isa::lookup(opcode, qualifier).ok_or_else(|| {
        r.malformed(format!(
            "unknown NCS instruction 0x{opcode:02X}/0x{qualifier:02X} at offset {offset}"
        ))
    })?;
    let op = decoded.mnemonic;
    let kind = decoded.operands;

    let mut ins = Instruction {
        offset: offset as u32,
        op,
        args: Vec::new(),
        routine: None,
        routine_name: None,
        argc: None,
    };

    match kind {
        Operands::None => {}
        Operands::Copy => {
            ins.args.push(Arg::Int(r.i32_be()? as i64));
            ins.args.push(Arg::Int(r.u16_be()? as i64));
        }
        Operands::ConstInt => ins.args.push(Arg::Int(r.i32_be()? as i64)),
        Operands::ConstFloat => ins.args.push(Arg::Float(r.f32_be()? as f64)),
        Operands::ConstString => {
            let len = r.u16_be()? as usize;
            let bytes = r.take(len)?;
            ins.args
                .push(Arg::Str(String::from_utf8_lossy(bytes).into_owned()));
        }
        Operands::ConstObject => ins.args.push(Arg::Int(r.i32_be()? as i64)),
        Operands::Action => {
            let routine = r.u16_be()?;
            let argc = r.u8()?;
            ins.routine = Some(routine);
            ins.routine_name = ncs_actions::name(routine);
            ins.argc = Some(argc);
            ins.args.push(Arg::Int(routine as i64));
            ins.args.push(Arg::Int(argc as i64));
        }
        Operands::Offset => ins.args.push(Arg::Int(r.i32_be()? as i64)),
        Operands::Jump => {
            let rel = r.i32_be()?;
            // Relative to the start of this instruction (opcode + qual + i32 = 6).
            let target = offset as i64 + rel as i64;
            if target < 0 || target as usize > end {
                return Err(r.malformed(format!(
                    "jump at {offset} lands outside the file (rel {rel}, target {target})"
                )));
            }
            ins.args.push(Arg::Jump(target as u32));
        }
        Operands::Destruct => {
            ins.args.push(Arg::Int(r.u16_be()? as i64));
            ins.args.push(Arg::Int(r.i16_be()? as i64));
            ins.args.push(Arg::Int(r.u16_be()? as i64));
        }
        Operands::Increment => ins.args.push(Arg::Int(r.u32_be()? as i64)),
        Operands::StoreState => {
            ins.args.push(Arg::Int(r.u32_be()? as i64));
            ins.args.push(Arg::Int(r.u32_be()? as i64));
        }
        Operands::StructCompare => ins.args.push(Arg::Int(r.u16_be()? as i64)),
    }
    Ok(ins)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn header(size: u32) -> Vec<u8> {
        let mut b = b"NCS V1.0".to_vec();
        b.push(0x42);
        b.extend_from_slice(&size.to_be_bytes());
        b
    }

    #[test]
    fn empty_script_is_valid() {
        let data = header(13);
        let n = read(&data, Path::new("empty.ncs")).unwrap();
        assert!(n.instructions.is_empty());
    }

    #[test]
    fn retn_only() {
        let mut data = header(15);
        data.extend_from_slice(&[0x20, 0x00]);
        let n = read(&data, Path::new("retn.ncs")).unwrap();
        assert_eq!(n.instructions.len(), 1);
        assert_eq!(n.instructions[0].op, "RETN");
        assert_eq!(n.instructions[0].offset, 13);
    }

    #[test]
    fn consts_and_action() {
        // CONSTS "hi" + ACTION GetObjectByTag(2) + RETN
        let mut body = Vec::new();
        body.extend_from_slice(&[0x04, 0x05]);
        body.extend_from_slice(&2u16.to_be_bytes());
        body.extend_from_slice(b"hi");
        body.extend_from_slice(&[0x05, 0x00]);
        body.extend_from_slice(&200u16.to_be_bytes()); // GetObjectByTag in both K1 and TSL
        body.push(2);
        body.extend_from_slice(&[0x20, 0x00]);
        let mut data = header(13 + body.len() as u32);
        data.extend_from_slice(&body);
        let n = read(&data, Path::new("call.ncs")).unwrap();
        assert_eq!(n.instructions[0].op, "CONSTS");
        match &n.instructions[0].args[0] {
            Arg::Str(s) => assert_eq!(s, "hi"),
            other => panic!("{other:?}"),
        }
        assert_eq!(n.instructions[1].op, "ACTION");
        assert_eq!(n.instructions[1].routine_name, Some("GetObjectByTag"));
        assert_eq!(n.instructions[1].argc, Some(2));
    }

    #[test]
    fn jump_is_absolute() {
        // JMP +0 (to itself) would be weird; JMP to RETN after it.
        // JMP instruction is 6 bytes at 13; RETN at 19.
        let mut data = header(21);
        data.extend_from_slice(&[0x1D, 0x00]);
        data.extend_from_slice(&6i32.to_be_bytes());
        data.extend_from_slice(&[0x20, 0x00]);
        let n = read(&data, Path::new("jmp.ncs")).unwrap();
        match n.instructions[0].args[0] {
            Arg::Jump(t) => assert_eq!(t, 19),
            ref other => panic!("{other:?}"),
        }
    }

    #[test]
    fn trailing_zero_padding_is_not_an_error() {
        let mut data = header(20);
        data.extend_from_slice(&[0x20, 0x00]);
        data.extend_from_slice(&[0, 0, 0, 0, 0]);
        let n = read(&data, Path::new("pad.ncs")).unwrap();
        assert_eq!(n.instructions.len(), 1);
    }

    #[test]
    fn bad_magic_is_rejected() {
        let mut data = b"NCS V1.0".to_vec();
        data.push(0x00);
        data.extend_from_slice(&13u32.to_be_bytes());
        assert!(read(&data, Path::new("bad.ncs")).is_err());
    }
}
