mod common;

use std::collections::HashMap;

use common::{asm, AsmArg};
use kq_format::ncs::{Arg, Instruction};
use kq_ncs::{analyze, infer_prototypes, split, Cfg, Game, SplitProgram, SubId, SubKind, Ty};

use AsmArg::*;

fn jump(ins: &mut [Instruction], from: usize, to: usize) {
    ins[from].args[0] = Arg::Jump(ins[to].offset);
}

fn cfgs_for(ins: &[Instruction], program: &SplitProgram) -> HashMap<SubId, Cfg> {
    let mut cfgs = HashMap::new();
    if let Some(globals) = &program.globals {
        cfgs.insert(SubId::Globals, analyze(ins, globals, &program.deferred));
    }
    cfgs.insert(SubId::Main, analyze(ins, &program.main, &program.deferred));
    for user in &program.users {
        if let SubKind::User(id) = user.kind {
            cfgs.insert(SubId::User(id), analyze(ins, user, &program.deferred));
        }
    }
    cfgs
}

fn infer(ins: &[Instruction]) -> (HashMap<SubId, kq_ncs::SubInfo>, Vec<kq_ncs::Warning>) {
    let program = split(ins).unwrap();
    let cfgs = cfgs_for(ins, &program);
    infer_prototypes(ins, &program, &cfgs, Game::K1)
}

#[test]
fn void_two_params() {
    let mut ins = asm(&[
        ("JSR", vec![JumpAbs(0)]),
        ("RETN", vec![]),
        ("CONSTI", vec![Int(1)]),
        ("CONSTI", vec![Int(2)]),
        ("JSR", vec![JumpAbs(0)]),
        ("RETN", vec![]),
        ("MOVSP", vec![Int(-8)]),
        ("RETN", vec![]),
    ]);
    jump(&mut ins, 0, 2);
    jump(&mut ins, 4, 6);

    let (infos, warnings) = infer(&ins);
    let sub = &infos[&SubId::User(1)];
    assert_eq!(sub.param_count, 2);
    assert_eq!(sub.ret, Ty::Void);
    assert_eq!(sub.params, vec![Ty::Int, Ty::Int]);
    assert!(warnings.is_empty(), "{warnings:#?}");
}

#[test]
fn int_return_one_param() {
    let mut ins = asm(&[
        ("JSR", vec![JumpAbs(0)]),
        ("RETN", vec![]),
        ("RSADDI", vec![]),
        ("CONSTI", vec![Int(0)]),
        ("JSR", vec![JumpAbs(0)]),
        ("MOVSP", vec![Int(-4)]),
        ("RETN", vec![]),
        ("CONSTI", vec![Int(1)]),
        ("CPDOWNSP", vec![Int(-12), Int(4)]),
        ("MOVSP", vec![Int(-4)]),
        ("MOVSP", vec![Int(-4)]),
        ("RETN", vec![]),
    ]);
    jump(&mut ins, 0, 2);
    jump(&mut ins, 4, 7);

    let (infos, warnings) = infer(&ins);
    let sub = &infos[&SubId::User(1)];
    assert_eq!(sub.param_count, 1);
    assert_eq!(sub.ret, Ty::Int);
    assert_eq!(sub.ret_depth, 2);
    assert_eq!(sub.params, vec![Ty::Int]);
    assert!(warnings.is_empty(), "{warnings:#?}");
}

#[test]
fn recursive_pair_scc_converges() {
    let mut ins = asm(&[
        ("JSR", vec![JumpAbs(0)]),
        ("RETN", vec![]),
        ("CONSTI", vec![Int(1)]),
        ("JSR", vec![JumpAbs(0)]),
        ("RETN", vec![]),
        ("CPTOPSP", vec![Int(-4), Int(4)]),
        ("JSR", vec![JumpAbs(0)]),
        ("MOVSP", vec![Int(-4)]),
        ("RETN", vec![]),
        ("CPTOPSP", vec![Int(-4), Int(4)]),
        ("JSR", vec![JumpAbs(0)]),
        ("MOVSP", vec![Int(-4)]),
        ("RETN", vec![]),
    ]);
    jump(&mut ins, 0, 2);
    jump(&mut ins, 3, 5);
    jump(&mut ins, 6, 9);
    jump(&mut ins, 10, 5);

    let (infos, warnings) = infer(&ins);
    assert_eq!(infos[&SubId::User(1)].param_count, 1);
    assert_eq!(infos[&SubId::User(2)].param_count, 1);
    assert_eq!(infos[&SubId::User(1)].ret, Ty::Void);
    assert_eq!(infos[&SubId::User(2)].ret, Ty::Void);
    assert!(warnings.is_empty(), "{warnings:#?}");
}

#[test]
fn jsr_pushes_int_return_for_caller_inference() {
    let mut ins = asm(&[
        ("JSR", vec![JumpAbs(0)]),
        ("RETN", vec![]),
        ("RSADDI", vec![]),
        ("JSR", vec![JumpAbs(0)]),
        ("MOVSP", vec![Int(-4)]),
        ("RETN", vec![]),
        ("JSR", vec![JumpAbs(0)]),
        ("CPDOWNSP", vec![Int(-8), Int(4)]),
        ("MOVSP", vec![Int(-4)]),
        ("RETN", vec![]),
        ("CONSTI", vec![Int(7)]),
        ("CPDOWNSP", vec![Int(-8), Int(4)]),
        ("MOVSP", vec![Int(-4)]),
        ("RETN", vec![]),
    ]);
    jump(&mut ins, 0, 2);
    jump(&mut ins, 3, 6);
    jump(&mut ins, 6, 10);

    let (infos, _) = infer(&ins);
    assert_eq!(infos[&SubId::User(1)].ret, Ty::Int);
    assert_eq!(infos[&SubId::User(2)].ret, Ty::Int);
}

#[test]
fn jsr_pushes_vector_return_for_caller_inference() {
    let mut ins = asm(&[
        ("JSR", vec![JumpAbs(0)]),
        ("RETN", vec![]),
        ("RSADDF", vec![]),
        ("RSADDF", vec![]),
        ("RSADDF", vec![]),
        ("JSR", vec![JumpAbs(0)]),
        ("MOVSP", vec![Int(-12)]),
        ("RETN", vec![]),
        ("JSR", vec![JumpAbs(0)]),
        ("CPDOWNSP", vec![Int(-24), Int(12)]),
        ("MOVSP", vec![Int(-12)]),
        ("RETN", vec![]),
        ("CONSTF", vec![Float(1.0)]),
        ("CONSTF", vec![Float(2.0)]),
        ("CONSTF", vec![Float(3.0)]),
        ("CPDOWNSP", vec![Int(-24), Int(12)]),
        ("MOVSP", vec![Int(-12)]),
        ("RETN", vec![]),
    ]);
    jump(&mut ins, 0, 2);
    jump(&mut ins, 5, 8);
    jump(&mut ins, 8, 12);

    let (infos, _) = infer(&ins);
    assert_eq!(infos[&SubId::User(1)].ret, Ty::Vector);
    assert_eq!(infos[&SubId::User(2)].ret, Ty::Vector);
}

#[test]
fn unresolved_return_defaults_to_int_with_warning() {
    let mut ins = asm(&[
        ("JSR", vec![JumpAbs(0)]),
        ("RETN", vec![]),
        ("RSADDI", vec![]),
        ("JSR", vec![JumpAbs(0)]),
        ("MOVSP", vec![Int(-4)]),
        ("RETN", vec![]),
        ("RETN", vec![]),
    ]);
    jump(&mut ins, 0, 2);
    jump(&mut ins, 3, 6);

    let (infos, warnings) = infer(&ins);
    assert_eq!(infos[&SubId::User(1)].ret, Ty::Int);
    assert!(warnings.iter().any(|warning| {
        warning.sub == Some(1)
            && warning.msg.contains("unresolved return")
            && warning.msg.contains("int")
    }));
}

#[test]
fn heterogeneous_action_params_keep_declared_order() {
    let mut ins = asm(&[
        ("JSR", vec![JumpAbs(0)]),
        ("RETN", vec![]),
        ("CONSTO", vec![Int(0)]),
        ("CONSTI", vec![Int(0)]),
        ("JSR", vec![JumpAbs(0)]),
        ("RETN", vec![]),
        ("CPTOPSP", vec![Int(-8), Int(4)]),
        ("CPTOPSP", vec![Int(-8), Int(4)]),
        ("ACTION", vec![Int(22), Int(2)]),
        ("MOVSP", vec![Int(-8)]),
        ("RETN", vec![]),
    ]);
    jump(&mut ins, 0, 2);
    jump(&mut ins, 4, 6);

    let (infos, _) = infer(&ins);
    assert_eq!(infos[&SubId::User(1)].params, vec![Ty::Object, Ty::Int]);
}

#[test]
fn floating_comparison_returns_int() {
    let mut ins = asm(&[
        ("JSR", vec![JumpAbs(0)]),
        ("RETN", vec![]),
        ("RSADDI", vec![]),
        ("JSR", vec![JumpAbs(0)]),
        ("MOVSP", vec![Int(-4)]),
        ("RETN", vec![]),
        ("CONSTF", vec![Float(1.0)]),
        ("CONSTF", vec![Float(2.0)]),
        ("EQUALFF", vec![]),
        ("CPDOWNSP", vec![Int(-8), Int(4)]),
        ("MOVSP", vec![Int(-4)]),
        ("RETN", vec![]),
    ]);
    jump(&mut ins, 0, 2);
    jump(&mut ins, 3, 6);

    let (infos, _) = infer(&ins);
    assert_eq!(infos[&SubId::User(1)].ret, Ty::Int);
}

#[test]
fn more_than_eight_param_slots_are_not_truncated() {
    let mut ins = asm(&[
        ("JSR", vec![JumpAbs(0)]),
        ("RETN", vec![]),
        ("JSR", vec![JumpAbs(0)]),
        ("RETN", vec![]),
        ("MOVSP", vec![Int(-36)]),
        ("RETN", vec![]),
    ]);
    jump(&mut ins, 0, 2);
    jump(&mut ins, 2, 4);

    let (infos, _) = infer(&ins);
    assert_eq!(infos[&SubId::User(1)].param_count, 9);
    assert_eq!(infos[&SubId::User(1)].params.len(), 9);
}

#[test]
fn mismatched_stack_height_join_warns() {
    let mut ins = asm(&[
        ("JSR", vec![JumpAbs(0)]),
        ("RETN", vec![]),
        ("CONSTI", vec![Int(1)]),
        ("JZ", vec![JumpAbs(0)]),
        ("CONSTI", vec![Int(2)]),
        ("JMP", vec![JumpAbs(0)]),
        ("JMP", vec![JumpAbs(0)]),
        ("RETN", vec![]),
    ]);
    jump(&mut ins, 0, 2);
    jump(&mut ins, 3, 6);
    jump(&mut ins, 5, 7);
    jump(&mut ins, 6, 7);

    let (_, warnings) = infer(&ins);
    assert!(warnings
        .iter()
        .any(|warning| warning.msg.contains("stack height")));
}
