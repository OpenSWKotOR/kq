//! Pattern tests for the decompiler pipeline.

mod common;

use common::{asm, AsmArg::*};
use kq_format::ncs::Ncs;
use kq_ncs::{analyze, decompile, split, Game, SplitError, SubKind};

fn asm_to_ncs(lines: &[(&str, Vec<common::AsmArg>)]) -> Ncs {
    let instructions = asm(lines);
    let declared_size = instructions
        .last()
        .map(|last| last.offset + 2)
        .unwrap_or(13);
    Ncs {
        declared_size,
        instructions,
    }
}

#[test]
fn golden_false_starting_conditional() {
    let ncs = asm_to_ncs(&[
        ("RSADDI", vec![]),
        ("JSR", vec![JumpAbs(23)]),
        ("RETN", vec![]),
        ("CONSTI", vec![Int(0)]),
        ("CPDOWNSP", vec![Int(-8), Int(4)]),
        ("MOVSP", vec![Int(-4)]),
        ("JMP", vec![JumpAbs(55)]),
        ("MOVSP", vec![Int(-4)]),
        ("RETN", vec![]),
    ]);
    let d = decompile(&ncs, Game::K1);
    assert_eq!(
        d.source.trim(),
        "int StartingConditional() {\n\treturn 0;\n}"
    );
}

#[test]
fn golden_k_pdan_juhani11_if() {
    let ncs = asm_to_ncs(&[
        ("JSR", vec![JumpAbs(21)]),
        ("RETN", vec![]),
        ("CONSTS", vec![Str("DAN_JEDI_PLOT".into())]),
        ("ACTION", vec![Int(580), Int(1)]),
        ("CONSTI", vec![Int(3)]),
        ("EQUALII", vec![]),
        ("JZ", vec![JumpAbs(91)]),
        ("CONSTI", vec![Int(4)]),
        ("CONSTS", vec![Str("DAN_JEDI_PLOT".into())]),
        ("ACTION", vec![Int(581), Int(2)]),
        ("JMP", vec![JumpAbs(91)]),
        ("CONSTI", vec![Int(2)]),
        ("CONSTS", vec![Str("DAN_JUHANI_PLOT".into())]),
        ("ACTION", vec![Int(581), Int(2)]),
        ("RETN", vec![]),
    ]);
    let d = decompile(&ncs, Game::K1);
    assert_eq!(
        d.source.trim(),
        "void main() {\n\tif (GetGlobalNumber(\"DAN_JEDI_PLOT\") == 3) {\n\t\tSetGlobalNumber(\"DAN_JEDI_PLOT\", 4);\n\t}\n\tSetGlobalNumber(\"DAN_JUHANI_PLOT\", 2);\n}"
    );
}

#[test]
fn golden_hjuh_h02_and_guard() {
    let ncs = asm_to_ncs(&[
        ("RSADDI", vec![]),
        ("JSR", vec![JumpAbs(23)]),
        ("RETN", vec![]),
        ("RSADDI", vec![]),
        ("CONSTS", vec![Str("G_JUHANIH_STATE".into())]),
        ("ACTION", vec![Int(580), Int(1)]),
        ("CONSTI", vec![Int(1)]),
        ("EQUALII", vec![]),
        ("CPTOPSP", vec![Int(-4), Int(4)]),
        ("JZ", vec![JumpAbs(100)]),
        ("ACTION", vec![Int(548), Int(0)]),
        ("ACTION", vec![Int(166), Int(1)]),
        ("CONSTS", vec![Str("T_LEVH".into())]),
        ("ACTION", vec![Int(580), Int(1)]),
        ("GTII", vec![]),
        ("LOGANDII", vec![]),
        ("CPDOWNSP", vec![Int(-8), Int(4)]),
        ("MOVSP", vec![Int(-4)]),
        ("CPTOPSP", vec![Int(-4), Int(4)]),
        ("JZ", vec![JumpAbs(202)]),
        ("CONSTI", vec![Int(2)]),
        ("CONSTS", vec![Str("G_JUHANIH_STATE".into())]),
        ("ACTION", vec![Int(581), Int(2)]),
        ("CONSTS", vec![Str("T_LEVH".into())]),
        ("ACTION", vec![Int(580), Int(1)]),
        ("CONSTI", vec![Int(1)]),
        ("ADDII", vec![]),
        ("CONSTS", vec![Str("T_LEVH".into())]),
        ("ACTION", vec![Int(581), Int(2)]),
        ("JMP", vec![JumpAbs(202)]),
        ("CPTOPSP", vec![Int(-4), Int(4)]),
        ("CPDOWNSP", vec![Int(-12), Int(4)]),
        ("MOVSP", vec![Int(-8)]),
        ("JMP", vec![JumpAbs(242)]),
        ("MOVSP", vec![Int(-4)]),
        ("MOVSP", vec![Int(-4)]),
        ("RETN", vec![]),
    ]);
    let d = decompile(&ncs, Game::K1);
    assert_eq!(
        d.source.trim(),
        "int StartingConditional() {\n\tint int1 = GetGlobalNumber(\"G_JUHANIH_STATE\") == 1 && GetHitDice(GetFirstPC()) > GetGlobalNumber(\"T_LEVH\");\n\tif (int1) {\n\t\tSetGlobalNumber(\"G_JUHANIH_STATE\", 2);\n\t\tSetGlobalNumber(\"T_LEVH\", GetGlobalNumber(\"T_LEVH\") + 1);\n\t}\n\treturn int1;\n}"
    );
}

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

#[test]
fn cfg_starting_conditional_dead_epilogue() {
    let ins = asm(&[
        ("RSADDI", vec![]),
        ("JSR", vec![JumpAbs(23)]),
        ("RETN", vec![]),
        ("CONSTI", vec![Int(0)]),
        ("CPDOWNSP", vec![Int(-8), Int(4)]),
        ("MOVSP", vec![Int(-4)]),
        ("JMP", vec![JumpAbs(55)]),
        ("MOVSP", vec![Int(-4)]),
        ("RETN", vec![]),
    ]);
    let p = split(&ins).unwrap();
    let cfg = analyze(&ins, &p.main, &[]);
    let dead_movsp = ins.iter().position(|i| i.offset == 49).unwrap();
    assert!(cfg.dead[dead_movsp]);
}

#[test]
fn cfg_log_or_extra_jz_from_mand04_shape() {
    let ins = asm(&[
        ("JSR", vec![JumpAbs(21)]),
        ("RETN", vec![]),
        ("CONSTI", vec![Int(0)]),
        ("CPTOPSP", vec![Int(-4), Int(4)]),
        ("JZ", vec![JumpAbs(55)]),
        ("CPTOPSP", vec![Int(-4), Int(4)]),
        ("JZ", vec![JumpAbs(61)]),
        ("CONSTI", vec![Int(1)]),
        ("LOGORII", vec![]),
        ("RETN", vec![]),
    ]);
    let p = split(&ins).unwrap();
    let cfg = analyze(&ins, &p.main, &p.deferred);
    let extra_jz_idx = ins.iter().position(|i| i.offset == 49).unwrap();
    assert!(cfg.log_or_extra_jz.contains(&extra_jz_idx));
}
