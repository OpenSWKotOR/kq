//! NCS — compiled NWScript bytecode.
//!
//! Layout (V1.0): `"NCS "`, `"V1.0"`, a magic byte `0x42`, then a **big-endian**
//! u32 file size, then a stream of (opcode, qualifier, operands) instructions.
//! Almost every multi-byte operand is big-endian — the opposite of every other
//! KotOR format. Jump offsets are relative to the start of their instruction.

use std::path::Path;

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
    let (op, kind) = classify(opcode, qualifier).ok_or_else(|| {
        r.malformed(format!(
            "unknown NCS instruction 0x{opcode:02X}/0x{qualifier:02X} at offset {offset}"
        ))
    })?;

    let mut ins = Instruction {
        offset: offset as u32,
        op,
        args: Vec::new(),
        routine: None,
        routine_name: None,
        argc: None,
    };

    match kind {
        Kind::Empty => {}
        Kind::Copy => {
            ins.args.push(Arg::Int(r.i32_be()? as i64));
            ins.args.push(Arg::Int(r.u16_be()? as i64));
        }
        Kind::ConstI => ins.args.push(Arg::Int(r.i32_be()? as i64)),
        Kind::ConstF => ins.args.push(Arg::Float(r.f32_be()? as f64)),
        Kind::ConstS => {
            let len = r.u16_be()? as usize;
            let bytes = r.take(len)?;
            ins.args
                .push(Arg::Str(String::from_utf8_lossy(bytes).into_owned()));
        }
        Kind::ConstO => ins.args.push(Arg::Int(r.i32_be()? as i64)),
        Kind::Action => {
            let routine = r.u16_be()?;
            let argc = r.u8()?;
            ins.routine = Some(routine);
            ins.routine_name = ncs_actions::name(routine);
            ins.argc = Some(argc);
            ins.args.push(Arg::Int(routine as i64));
            ins.args.push(Arg::Int(argc as i64));
        }
        Kind::Offset => ins.args.push(Arg::Int(r.i32_be()? as i64)),
        Kind::Jump => {
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
        Kind::Destruct => {
            ins.args.push(Arg::Int(r.u16_be()? as i64));
            ins.args.push(Arg::Int(r.i16_be()? as i64));
            ins.args.push(Arg::Int(r.u16_be()? as i64));
        }
        Kind::Inc => ins.args.push(Arg::Int(r.u32_be()? as i64)),
        Kind::StoreState => {
            ins.args.push(Arg::Int(r.u32_be()? as i64));
            ins.args.push(Arg::Int(r.u32_be()? as i64));
        }
        Kind::StructEq => ins.args.push(Arg::Int(r.u16_be()? as i64)),
    }
    Ok(ins)
}

#[derive(Clone, Copy)]
enum Kind {
    Empty,
    Copy,
    ConstI,
    ConstF,
    ConstS,
    ConstO,
    Action,
    Offset,
    Jump,
    Destruct,
    Inc,
    StoreState,
    StructEq,
}

fn classify(opcode: u8, qualifier: u8) -> Option<(&'static str, Kind)> {
    use Kind::*;
    // Reserved / unknown padding: any qualifier, two bytes, no operands.
    if opcode == 0x00 {
        return Some(("RESERVED", Empty));
    }
    match (opcode, qualifier) {
        (0x01, 0x01) => Some(("CPDOWNSP", Copy)),
        (0x02, 0x03) => Some(("RSADDI", Empty)),
        (0x02, 0x04) => Some(("RSADDF", Empty)),
        (0x02, 0x05) => Some(("RSADDS", Empty)),
        (0x02, 0x06) => Some(("RSADDO", Empty)),
        (0x02, 0x10) => Some(("RSADDEFF", Empty)),
        (0x02, 0x11) => Some(("RSADDEVT", Empty)),
        (0x02, 0x12) => Some(("RSADDLOC", Empty)),
        (0x02, 0x13) => Some(("RSADDTAL", Empty)),
        (0x03, 0x01) => Some(("CPTOPSP", Copy)),
        (0x04, 0x03) => Some(("CONSTI", ConstI)),
        (0x04, 0x04) => Some(("CONSTF", ConstF)),
        (0x04, 0x05) => Some(("CONSTS", ConstS)),
        (0x04, 0x06) => Some(("CONSTO", ConstO)),
        (0x05, 0x00) => Some(("ACTION", Action)),
        (0x06, 0x20) => Some(("LOGANDII", Empty)),
        (0x07, 0x20) => Some(("LOGORII", Empty)),
        (0x08, 0x20) => Some(("INCORII", Empty)),
        (0x09, 0x20) => Some(("EXCORII", Empty)),
        (0x0A, 0x20) => Some(("BOOLANDII", Empty)),
        (0x0B, 0x20) => Some(("EQUALII", Empty)),
        (0x0B, 0x21) => Some(("EQUALFF", Empty)),
        (0x0B, 0x22) => Some(("EQUALOO", Empty)),
        (0x0B, 0x23) => Some(("EQUALSS", Empty)),
        (0x0B, 0x24) => Some(("EQUALTT", StructEq)),
        (0x0B, 0x30) => Some(("EQUALEFFEFF", Empty)),
        (0x0B, 0x31) => Some(("EQUALEVTEVT", Empty)),
        (0x0B, 0x32) => Some(("EQUALLOCLOC", Empty)),
        (0x0B, 0x33) => Some(("EQUALTALTAL", Empty)),
        (0x0C, 0x20) => Some(("NEQUALII", Empty)),
        (0x0C, 0x21) => Some(("NEQUALFF", Empty)),
        (0x0C, 0x22) => Some(("NEQUALOO", Empty)),
        (0x0C, 0x23) => Some(("NEQUALSS", Empty)),
        (0x0C, 0x24) => Some(("NEQUALTT", StructEq)),
        (0x0C, 0x30) => Some(("NEQUALEFFEFF", Empty)),
        (0x0C, 0x31) => Some(("NEQUALEVTEVT", Empty)),
        (0x0C, 0x32) => Some(("NEQUALLOCLOC", Empty)),
        (0x0C, 0x33) => Some(("NEQUALTALTAL", Empty)),
        (0x0D, 0x20) => Some(("GEQII", Empty)),
        (0x0D, 0x21) => Some(("GEQFF", Empty)),
        (0x0E, 0x20) => Some(("GTII", Empty)),
        (0x0E, 0x21) => Some(("GTFF", Empty)),
        (0x0F, 0x20) => Some(("LTII", Empty)),
        (0x0F, 0x21) => Some(("LTFF", Empty)),
        (0x10, 0x20) => Some(("LEQII", Empty)),
        (0x10, 0x21) => Some(("LEQFF", Empty)),
        (0x11, 0x20) => Some(("SHLEFTII", Empty)),
        (0x12, 0x20) => Some(("SHRIGHTII", Empty)),
        (0x13, 0x20) => Some(("USHRIGHTII", Empty)),
        (0x14, 0x20) => Some(("ADDII", Empty)),
        (0x14, 0x21) => Some(("ADDFF", Empty)),
        (0x14, 0x25) => Some(("ADDIF", Empty)),
        (0x14, 0x26) => Some(("ADDFI", Empty)),
        (0x14, 0x23) => Some(("ADDSS", Empty)),
        (0x14, 0x3A) => Some(("ADDVV", Empty)),
        (0x15, 0x20) => Some(("SUBII", Empty)),
        (0x15, 0x21) => Some(("SUBFF", Empty)),
        (0x15, 0x25) => Some(("SUBIF", Empty)),
        (0x15, 0x26) => Some(("SUBFI", Empty)),
        (0x15, 0x3A) => Some(("SUBVV", Empty)),
        (0x16, 0x20) => Some(("MULII", Empty)),
        (0x16, 0x21) => Some(("MULFF", Empty)),
        (0x16, 0x25) => Some(("MULIF", Empty)),
        (0x16, 0x26) => Some(("MULFI", Empty)),
        (0x16, 0x3B) => Some(("MULVF", Empty)),
        (0x16, 0x3C) => Some(("MULFV", Empty)),
        (0x17, 0x20) => Some(("DIVII", Empty)),
        (0x17, 0x21) => Some(("DIVFF", Empty)),
        (0x17, 0x25) => Some(("DIVIF", Empty)),
        (0x17, 0x26) => Some(("DIVFI", Empty)),
        (0x17, 0x3B) => Some(("DIVVF", Empty)),
        (0x17, 0x3C) => Some(("DIVFV", Empty)),
        (0x18, 0x20) => Some(("MODII", Empty)),
        (0x19, 0x03) => Some(("NEGI", Empty)),
        (0x19, 0x04) => Some(("NEGF", Empty)),
        (0x1A, 0x03) => Some(("COMPI", Empty)),
        (0x1B, 0x00) => Some(("MOVSP", Offset)),
        (0x1D, 0x00) => Some(("JMP", Jump)),
        (0x1E, 0x00) => Some(("JSR", Jump)),
        (0x1F, 0x00) => Some(("JZ", Jump)),
        (0x20, 0x00) => Some(("RETN", Empty)),
        (0x21, 0x01) => Some(("DESTRUCT", Destruct)),
        (0x22, 0x03) => Some(("NOTI", Empty)),
        (0x23, 0x03) => Some(("DECxSP", Inc)),
        (0x24, 0x03) => Some(("INCxSP", Inc)),
        (0x25, 0x00) => Some(("JNZ", Jump)),
        (0x26, 0x01) => Some(("CPDOWNBP", Copy)),
        (0x27, 0x01) => Some(("CPTOPBP", Copy)),
        (0x28, 0x03) => Some(("DECxBP", Inc)),
        (0x29, 0x03) => Some(("INCxBP", Inc)),
        (0x2A, 0x00) => Some(("SAVEBP", Empty)),
        (0x2B, 0x00) => Some(("RESTOREBP", Empty)),
        (0x2C, 0x10) => Some(("STORE_STATE", StoreState)),
        (0x2D, 0x00) => Some(("NOP", Empty)),
        _ => None,
    }
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
