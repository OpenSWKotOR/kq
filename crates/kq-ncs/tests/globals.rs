//! Globals sub: SAVEBP/RESTOREBP block → `intGLOB_1`, … (research-dencs §2.1(b) / §9.4(6)).

mod common;

use common::{asm, AsmArg::*};
use kq_format::ncs::{Arg, Instruction};
use kq_ncs::{build_globals, split, Const, Expr, Game, GlobalsError, SubKind, Ty};

fn jump(ins: &mut [Instruction], from: usize, to: usize) {
    ins[from].args[0] = Arg::Jump(ins[to].offset);
}

#[test]
fn globals_two_ints() {
    // RSADDI; CONSTI 0; CPDOWNSP; MOVSP; RSADDI; CONSTI 1; … SAVEBP; JSR; RESTOREBP; MOVSP; RETN
    let mut ins = asm(&[
        ("JSR", vec![JumpAbs(0)]),
        ("RETN", vec![]),
        ("RSADDI", vec![]),
        ("CONSTI", vec![Int(0)]),
        ("CPDOWNSP", vec![Int(-8), Int(4)]),
        ("MOVSP", vec![Int(-4)]),
        ("RSADDI", vec![]),
        ("CONSTI", vec![Int(1)]),
        ("CPDOWNSP", vec![Int(-8), Int(4)]),
        ("MOVSP", vec![Int(-4)]),
        ("SAVEBP", vec![]),
        ("JSR", vec![JumpAbs(0)]),
        ("RESTOREBP", vec![]),
        ("MOVSP", vec![Int(-8)]),
        ("RETN", vec![]),
        ("RETN", vec![]),
    ]);
    jump(&mut ins, 0, 2);
    jump(&mut ins, 11, 15);

    let program = split(&ins).unwrap();
    let range = program.globals.expect("globals sub");
    assert_eq!(range.kind, SubKind::Globals);

    let g = build_globals(&ins, &range, Game::K1).unwrap();
    assert_eq!(g.vars[0].name, "intGLOB_1");
    assert_eq!(g.vars[1].name, "intGLOB_2");
    assert_eq!(g.vars[0].ty, Ty::Int);
    assert_eq!(g.vars[1].ty, Ty::Int);
    assert_eq!(g.vars[0].init, Some(Expr::Const(Const::Int(0))));
    assert_eq!(g.vars[1].init, Some(Expr::Const(Const::Int(1))));
}

#[test]
fn globals_bare_rsadd_typed_without_init() {
    let mut ins = asm(&[
        ("JSR", vec![JumpAbs(0)]),
        ("RETN", vec![]),
        ("RSADDI", vec![]),
        ("RSADDF", vec![]),
        ("SAVEBP", vec![]),
        ("JSR", vec![JumpAbs(0)]),
        ("RESTOREBP", vec![]),
        ("MOVSP", vec![Int(-8)]),
        ("RETN", vec![]),
        ("RETN", vec![]),
    ]);
    jump(&mut ins, 0, 2);
    jump(&mut ins, 5, 9);

    let program = split(&ins).unwrap();
    let range = program.globals.expect("globals sub");
    let g = build_globals(&ins, &range, Game::K1).unwrap();
    assert_eq!(g.vars.len(), 2);
    assert_eq!(g.vars[0].name, "intGLOB_1");
    assert_eq!(g.vars[1].name, "floatGLOB_1");
    assert!(g.vars[0].init.is_none());
    assert!(g.vars[1].init.is_none());
}

#[test]
fn globals_no_rsadd_is_error() {
    let mut ins = asm(&[
        ("JSR", vec![JumpAbs(0)]),
        ("RETN", vec![]),
        ("SAVEBP", vec![]),
        ("JSR", vec![JumpAbs(0)]),
        ("RESTOREBP", vec![]),
        ("RETN", vec![]),
        ("RETN", vec![]),
    ]);
    jump(&mut ins, 0, 2);
    jump(&mut ins, 3, 6);

    let program = split(&ins).unwrap();
    let range = program.globals.expect("globals sub");
    assert_eq!(
        build_globals(&ins, &range, Game::K1),
        Err(GlobalsError::NoRecoverableGlobals)
    );
}
