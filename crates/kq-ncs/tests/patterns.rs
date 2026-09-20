//! Pattern tests for the decompiler pipeline. Task 3: split section.

mod common;

use common::{asm, AsmArg::*};
use kq_ncs::{split, SplitError, SubKind};

#[test]
fn split_main_with_globals() {
    // (b) header JSR → globals; globals SAVEBP/JSR/RESTOREBP; then main; then sub1.
    // Offsets: globals@21, main@61, user1@69.
    let ins = asm(&[
        ("JSR", vec![JumpAbs(21)]),
        ("RETN", vec![]),
        ("RSADDI", vec![]),
        ("CONSTI", vec![Int(0)]),
        ("CPDOWNSP", vec![Int(-8), Int(4)]),
        ("MOVSP", vec![Int(-4)]),
        ("SAVEBP", vec![]),
        ("JSR", vec![JumpAbs(61)]),
        ("RESTOREBP", vec![]),
        ("MOVSP", vec![Int(-4)]),
        ("RETN", vec![]),
        ("CONSTI", vec![Int(0)]),
        ("RETN", vec![]),
        ("RETN", vec![]),
    ]);
    let p = split(&ins).unwrap();
    assert!(!p.conditional_header);
    let g = p.globals.expect("globals sub");
    assert_eq!(g.kind, SubKind::Globals);
    assert_eq!(g.start_pos, 21);
    assert_eq!(p.main.kind, SubKind::Main);
    assert_eq!(p.main.start_pos, 61);
    assert_eq!(p.users.len(), 1);
    assert_eq!(p.users[0].kind, SubKind::User(1));
    assert_eq!(p.users[0].start_pos, 69);
}

#[test]
fn split_savebp_alone_is_main_not_globals() {
    // Globals only when the first sub has SAVEBP/RESTOREBP *and* another sub follows.
    let ins = asm(&[
        ("JSR", vec![JumpAbs(21)]),
        ("RETN", vec![]),
        ("RSADDI", vec![]),
        ("SAVEBP", vec![]),
        ("RESTOREBP", vec![]),
        ("RETN", vec![]),
    ]);
    let p = split(&ins).unwrap();
    assert!(p.globals.is_none());
    assert_eq!(p.main.start_pos, 21);
    assert!(p.users.is_empty());
}

#[test]
fn split_jsr_target_must_be_a_sub_start() {
    let ins = asm(&[
        ("JSR", vec![JumpAbs(99)]),
        ("RETN", vec![]),
        ("CONSTI", vec![Int(0)]),
        ("RETN", vec![]),
    ]);
    match split(&ins) {
        Err(SplitError::JsrTargetNotSub { target: 99, .. }) => {}
        other => panic!("expected JsrTargetNotSub, got {other:?}"),
    }
}
