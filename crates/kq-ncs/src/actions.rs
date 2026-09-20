use crate::actions_gen::ACTIONS_GEN;
use crate::ty::Ty;
use crate::Game;

pub const K1_ACTION_COUNT: usize = 772;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParamSig {
    pub ty: Ty,
    pub default: Option<&'static str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActionSig {
    pub name: &'static str,
    pub ret: Ty,
    pub params: &'static [ParamSig],
}

/// Full TSL-superset; missing ids are None.
pub fn actions(game: Game) -> &'static [Option<ActionSig>] {
    match game {
        Game::K1 => &ACTIONS_GEN[..K1_ACTION_COUNT],
        Game::K2 => &ACTIONS_GEN[..],
    }
}

pub fn action(game: Game, id: u16) -> Option<&'static ActionSig> {
    actions(game)
        .get(id as usize)
        .and_then(|slot| slot.as_ref())
}

#[cfg(test)]
mod tests {
    use crate::actions::{action, K1_ACTION_COUNT};
    use crate::ty::Ty;
    use crate::Game;

    #[test]
    fn k1_prefix_ids_match_known_bindings() {
        use crate::actions::{action, K1_ACTION_COUNT};
        assert_eq!(K1_ACTION_COUNT, 772);
        assert_eq!(action(Game::K1, 0).unwrap().name, "Random");
        assert_eq!(action(Game::K1, 28).unwrap().name, "GetFacing");
        assert_eq!(action(Game::K1, 28).unwrap().ret, Ty::Float);
        assert_eq!(action(Game::K1, 6).unwrap().name, "AssignCommand");
        assert!(matches!(
            action(Game::K1, 6).unwrap().params[1].ty,
            Ty::Action
        ));
        // K1 must not see TSL-only ids
        assert!(action(Game::K1, 772).is_none());
    }

    #[test]
    fn tsl_ids_de_ncs_got_wrong() {
        assert_eq!(action(Game::K2, 771).unwrap().name, "GetItemComponent");
        assert_eq!(
            action(Game::K2, 772).unwrap().name,
            "GetItemComponentPieceValue"
        );
        assert_eq!(action(Game::K2, 806).unwrap().name, "QueueMovie");
    }

    #[test]
    fn k1_prefix_equals_tsl_prefix() {
        for id in 0..K1_ACTION_COUNT {
            let a = action(Game::K1, id as u16);
            let b = action(Game::K2, id as u16);
            assert_eq!(a.map(|s| s.name), b.map(|s| s.name));
        }
    }

    #[test]
    fn action_typed_params() {
        assert!(matches!(
            action(Game::K1, 6).unwrap().params[1].ty,
            Ty::Action
        ));
        assert!(matches!(
            action(Game::K1, 7).unwrap().params[1].ty,
            Ty::Action
        ));
        assert!(matches!(
            action(Game::K1, 294).unwrap().params[0].ty,
            Ty::Action
        ));
    }
}
