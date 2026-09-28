//! Assembler DSL and §2.1 split tests.

mod common;

pub use common::{asm, AsmArg};

use kq_ncs::split;
use AsmArg::*;

#[test]
fn split_main_without_globals() {
    // (a) JSR ->21; RETN; … RETN
    let ins = asm(&[
        ("JSR", vec![JumpAbs(21)]),
        ("RETN", vec![]),
        ("CONSTI", vec![Int(0)]),
        ("RETN", vec![]),
    ]);
    let p = split(&ins).unwrap();
    assert!(p.globals.is_none());
    assert!(!p.conditional_header);
    assert_eq!(p.main.start_pos, 21);
}

#[test]
fn split_starting_conditional() {
    // (c) RSADDI; JSR; RETN; body; RETN
    let ins = asm(&[
        ("RSADDI", vec![]),
        ("JSR", vec![JumpAbs(23)]),
        ("RETN", vec![]),
        ("CONSTI", vec![Int(0)]),
        ("CPDOWNSP", vec![Int(-8), Int(4)]),
        ("MOVSP", vec![Int(-4)]),
        ("JMP", vec![JumpAbs(55)]),
        ("MOVSP", vec![Int(-4)]), // dead
        ("RETN", vec![]),
    ]);
    let p = split(&ins).unwrap();
    assert!(p.conditional_header);
}

#[test]
fn retn_inside_store_state_does_not_split() {
    // STORE_STATE; JMP ->after; body; RETN; after…
    // Offsets: JSR@13, RETN@19, STORE_STATE@21, JMP@31, ACTION@37, RETN@42, RETN@44.
    // JMP target is the instruction after the deferred RETN (44), so pending_block_end = 42.
    let ins = asm(&[
        ("JSR", vec![JumpAbs(21)]),
        ("RETN", vec![]),
        ("STORE_STATE", vec![Int(0), Int(8)]),
        ("JMP", vec![JumpAbs(44)]),
        ("ACTION", vec![Int(6), Int(2)]),
        ("RETN", vec![]), // closes deferred, not sub
        ("RETN", vec![]), // closes main
    ]);
    let p = split(&ins).unwrap();
    assert_eq!(p.users.len(), 0);
    assert_eq!(p.deferred.len(), 1);
}
