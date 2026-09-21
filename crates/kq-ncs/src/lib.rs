//! NCS → NSS decompiler. Algorithms live in `kotor-ncs` (DeNCS port).

use std::collections::HashMap;

use kq_format::ncs::{Instruction, Ncs};

pub use kq_index::Game;
pub use kotor_ncs::{
    analyze, cleanup, disasm_comment, emit_program, fallback_sub_body, format_float, split, BinOp,
    Block, BlockEnd, BuildError, Cfg, Const, CpDownTarget, Decompiled, DeferredRegion, ElseArm,
    EmitBody, Entry, Expr, GlobalTable, GlobalVar, GlobalsError, LocalStack, NameGen, Severity,
    SplitError, SplitProgram, StackError, Stmt, StructDef, StructId, StructTable, SubId, SubInfo,
    SubKind, SubRange, SubReport, SubStatus, SwitchCase, Ty, UnaryOp, Var, VarId, VarKind,
    VarTable, Warning,
};

fn map_game(game: Game) -> kotor_ncs::Game {
    match game {
        Game::K1 => kotor_ncs::Game::K1,
        Game::K2 => kotor_ncs::Game::K2,
    }
}

pub fn decompile(ncs: &Ncs, game: Game) -> Decompiled {
    kotor_ncs::decompile(ncs, map_game(game))
}

pub fn infer_prototypes(
    ins: &[Instruction],
    split: &SplitProgram,
    cfgs: &HashMap<SubId, Cfg>,
    game: Game,
) -> (HashMap<SubId, SubInfo>, Vec<Warning>) {
    kotor_ncs::infer_prototypes(ins, split, cfgs, map_game(game))
}

pub fn build_globals(
    ins: &[Instruction],
    range: &SubRange,
    game: Game,
) -> Result<GlobalTable, GlobalsError> {
    kotor_ncs::build_globals(ins, range, map_game(game))
}

pub fn build_sub(
    ins: &[Instruction],
    sub: &SubInfo,
    cfg: &Cfg,
    globals: &GlobalTable,
    protos: &HashMap<SubId, SubInfo>,
    game: Game,
) -> Result<(Block, VarTable, StructTable), BuildError> {
    kotor_ncs::build_sub(ins, sub, cfg, globals, protos, map_game(game))
}
