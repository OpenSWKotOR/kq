//! NCS — compiled NWScript bytecode.
//!
//! Layout (V1.0): `"NCS "`, `"V1.0"`, a magic byte `0x42`, then a **big-endian**
//! u32 file size, then a stream of (opcode, qualifier, operands) instructions.
//! Almost every multi-byte operand is big-endian — the opposite of every other
//! KotOR format. Jump offsets are relative to the start of their instruction.
//!
//! Decode and encode live in `kotor-ncs` (DeNCS `Decoder.java`). This module
//! is the kq-facing wrapper so errors still name the file.

use std::path::Path;

use crate::error::{FormatError, Result};

pub use kotor_ncs::{Arg, Instruction, Ncs};

pub fn sniff(data: &[u8]) -> bool {
    kotor_ncs::sniff(data)
}

pub fn read(data: &[u8], path: &Path) -> Result<Ncs> {
    kotor_ncs::read(data).map_err(|e| FormatError::Malformed {
        path: path.to_path_buf(),
        message: e.message,
    })
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
        let mut body = Vec::new();
        body.extend_from_slice(&[0x04, 0x05]);
        body.extend_from_slice(&2u16.to_be_bytes());
        body.extend_from_slice(b"hi");
        body.extend_from_slice(&[0x05, 0x00]);
        body.extend_from_slice(&200u16.to_be_bytes());
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

    #[test]
    fn increment_offset_is_signed() {
        let mut data = header(19);
        data.extend_from_slice(&[0x24, 0x03]);
        data.extend_from_slice(&(-12i32).to_be_bytes());
        let n = read(&data, Path::new("t.ncs")).unwrap();
        assert_eq!(n.instructions.last().unwrap().args[0], Arg::Int(-12));
    }
}
