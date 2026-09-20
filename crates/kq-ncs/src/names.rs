//! Generic names and DeNCS `NameGenerator` action table (research-dencs §7).

use std::collections::{HashMap, HashSet};

use crate::ast::Expr;
use crate::stack::Var;
use crate::ty::Ty;

/// Uppercase first letter + rest, spaces → `_`. Empty and 1-char tags → None.
fn camel_tag(s: &str) -> Option<String> {
    if s.len() <= 1 {
        return None;
    }
    let mut chars = s.chars();
    let first = chars.next()?.to_uppercase();
    let rest: String = chars.collect();
    let mut out = String::new();
    out.extend(first);
    out.push_str(&rest);
    Some(out.replace(' ', "_"))
}

/// Returns None to keep the generic `<type><n>` name.
#[rustfmt::skip]
pub fn name_from_action(action: &str, args: &[Expr]) -> Option<String> {
    let tag = |i: usize| args.get(i).and_then(Expr::as_str_const).and_then(camel_tag); // "" and 1-char → None
    let int = |i: usize| args.get(i).and_then(Expr::as_int_const).unwrap_or(-1);
    Some(match action {
        "GetObjectByTag"        => return tag(0).map(|t| format!("o{t}")),
        "GetFirstPC"            => "oPC".into(),
        "GetScriptParameter"    => { let i = int(0); if i > 0 { format!("nParam{i}") } else { "nParam".into() } }
        "GetScriptStringParameter" => "sParam".into(),
        "GetMaxHitPoints"       => "nMaxHP".into(),
        "GetCurrentHitPoints"   => "nCurHP".into(),
        "Random"                => "nRandom".into(),
        "GetArea"               => "oArea".into(),
        "GetEnteringObject"     => "oEntering".into(),
        "GetExitingObject"      => "oExiting".into(),
        "GetPosition"           => "vPosition".into(),
        "GetFacing"             => "fFacing".into(),
        "GetLastAttacker"       => "oAttacker".into(),
        "GetNearestCreature"    => "oNearest".into(),
        "GetDistanceToObject"   => "fDistance".into(),
        "GetIsObjectValid"      => "nValid".into(),
        "GetSpellTargetObject"  => "oTarget".into(),
        "EffectAssuredHit"      => "efHit".into(),
        "GetLastItemEquipped"   => "oLastEquipped".into(),
        "GetCurrentForcePoints" => "nCurFP".into(),
        "GetMaxForcePoints"     => "nMaxFP".into(),
        "EffectHeal"            => "efHeal".into(),
        "EffectDamage"          => "efDamage".into(),
        "EffectAbilityIncrease" => "efAbilityInc".into(),
        "EffectDamageResistance"=> "efDamageRes".into(),
        "EffectResurrection"    => "efResurrect".into(),
        "GetCasterLevel"        => "nCasterLevel".into(),
        "GetFirstObjectInArea" | "GetNextObjectInArea" => "oAreaObject".into(),
        "GetObjectType"         => "nType".into(),
        "GetRacialType"         => "nRace".into(),
        "EffectACIncrease"      => "efACInc".into(),
        "EffectSavingThrowIncrease" => "efSaveInc".into(),
        "EffectAttackIncrease"  => "efAttackInc".into(),
        "EffectDamageReduction" => "efDamageDec".into(),
        "EffectDamageIncrease"  => "efDamageInc".into(),
        "GetGoodEvilValue" | "GetAlignmentGoodEvil" => "nAlign".into(),
        "GetPartyMemberCount"   => "nPartyCount".into(),
        "GetFirstObjectInShape" | "GetNextObjectInShape" => "oShapeObject".into(),
        "EffectEntangle"        => "efEntangle".into(),
        "EffectDeath"           => "efDeath".into(),
        "EffectKnockdown"       => "efKnockdown".into(),
        "GetAbilityScore"       => match int(1) { 0=>"nStrength",1=>"nDex",2=>"nConst",3=>"nInt",4=>"nWis",5=>"nChar",_=>"nAbility" }.into(),
        "EffectParalyze"        => "efParalyze".into(),
        "EffectSpellImmunity"   => "efSpellImm".into(),
        "GetDistanceBetween"    => "fDistance".into(),
        "EffectForceJump"       => "efForceJump".into(),
        "EffectSleep"           => "efSleep".into(),
        "GetItemInSlot"         => match int(0) {
            0=>"oHeadItem",1=>"oBodyItem",3=>"oHandsItem",4=>"oRWeapItem",5=>"oLWeapItem",7=>"oLArmItem",
            8=>"oRArmItem",9=>"oImplantItem",10=>"oBeltItem",14=>"oCWeapLItem",15=>"oCWeapRItem",
            16=>"oCWeapBItem",17=>"oCArmourItem",18=>"oRWeap2Item",19=>"oLWeap2Item",_=>"oSlotItem" }.into(),
        "EffectTemporaryForcePoints" => "efTempFP".into(),
        "EffectConfused"        => "efConfused".into(),
        "EffectFrightened"      => "efFright".into(),
        "EffectChoke"           => "efChoke".into(),
        "EffectStunned"         => "efStun".into(),
        "EffectRegenerate"      => "efRegen".into(),
        "EffectMovementSpeedIncrease" => "efSpeedInc".into(),
        "GetHitDice"            => "nLevel".into(),
        "GetEffectType"         => "nEfType".into(),
        "EffectAreaOfEffect"    => "efAOE".into(),
        "EffectVisualEffect"    => "efVisual".into(),
        "GetFactionWeakestMember" => "oWeakest".into(),
        "GetFactionStrongestMember" => "oStrongest".into(),
        "GetFactionMostDamagedMember" => "oMostDamaged".into(),
        "GetFactionLeastDamagedMember" => "oLeastDamaged".into(),
        "GetWaypointByTag"      => tag(0).map(|t| format!("o{t}")).unwrap_or_else(|| "oWP".into()),
        "GetTransitionTarget"   => "oTransTarget".into(),
        "EffectBeam"            => "efBeam".into(),
        "GetReputation"         => "nRep".into(),
        "GetModuleFileName"     => "sModule".into(),
        "EffectForceResistanceIncrease" => "efForceResInc".into(),
        "GetSpellTargetLocation"=> "locTarget".into(),
        "EffectBodyFuel"        => "efFuel".into(),
        "GetFacingFromLocation" => "fFacing".into(),
        "GetNearestCreatureToLocation" => "oNearestCreat".into(),
        "GetNearestObject" | "GetNearestObjectToLocation" => "oNearest".into(),
        "GetNearestObjectByTag" => return tag(0).map(|t| format!("oNearest{t}")),
        "GetPCSpeaker" | "GetLastSpeaker" => "oSpeaker".into(),
        "GetModule"             => "oModule".into(),
        "CreateObject"          => return tag(1).map(|t| format!("o{t}")),
        "EventSpellCastAt"      => "evSpellCast".into(),
        "GetLastSpellCaster"    => "oCaster".into(),
        "EffectPoison"          => "efPoison".into(),
        "EffectAssuredDeflection" => "efDeflect".into(),
        "GetName"               => "sName".into(),
        "GetLastPerceived"      => "oPerceived".into(),
        "EffectForcePushTargeted" => "efPush".into(),
        "EffectHaste"           => "efHaste".into(),
        "EffectImmunity"        => "efImmunity".into(),
        "GetIsImmune"           => "nImmune".into(),
        "EffectDamageImmunityIncrease" => "efDamageImmInc".into(),
        "GetDistanceBetweenLocations" => "fDistance".into(),
        "GetLocalNumber"        => "nLocal".into(),
        "GetStringLength"       => "nLen".into(),
        "GetObjectPersonalSpace"=> "fPersonalSpace".into(),
        "d3" | "d4" | "d6" | "d8" | "d10" | "d100" => "nRandom".into(),   // d2, d12, d20 are NOT in the table
        "GetPartyMemberByIndex" => "oNPC".into(),
        "GetAttackTarget" | "GetHealTarget" => "oTarget".into(),
        "GetCreatureTalentRandom" => "talRandom".into(),
        "GetPUPOwner"           => "oPUPOwner".into(),
        "GetDistanceToObject2D" => "fDistance".into(),
        "GetCurrentAction"      => "nAction".into(),
        "GetPartyLeader"        => "oLeader".into(),
        "GetFirstEffect"        => "efFirst".into(),
        "GetNextEffect"         => "efNext".into(),
        "GetPartyAIStyle"       => "nStyle".into(),
        "GetNPCAIStyle"         => "nNPCStyle".into(),
        "GetLastHostileTarget"  => "oLastTarget".into(),
        "GetLastHostileActor"   => "oLastActor".into(),
        "GetRandomDestination"  => "vRandom".into(),
        "GetCreatureTalentBest" => "talBest".into(),
        "GetIdFromTalent"       => "nTalent".into(),
        "GetLocalBoolean"       => "nLocalBool".into(),
        "TalentSpell"           => "talSpell".into(),
        "TalentFeat"            => "talFeat".into(),
        "GetGlobalNumber"       => "nGlobal".into(),
        "GetBaseItemType"       => "nItemType".into(),
        "GetFirstItemInInventory" | "GetNextItemInInventory" => "oInvItem".into(),
        "GetSpellBaseForcePointCost" => "nBaseFP".into(),
        "GetLastForcePowerUsed" => "nLastForce".into(),
        // explicitly None in DeNCS:
        "Location" | "FloatToString" | "GetLocation" | "IntToString" | "StringToInt" => return None,
        _ => return None,   // DeNCS prints "Variable Naming: consider adding <action>" to stdout here
    })
}

fn type_word(ty: &Ty) -> &'static str {
    match ty {
        Ty::Unknown => "unknown",
        Ty::Void => "void",
        Ty::Int => "int",
        Ty::Float => "float",
        Ty::Str => "string",
        Ty::Object => "object",
        Ty::Effect => "effect",
        Ty::Event => "event",
        Ty::Location => "location",
        Ty::Talent => "talent",
        Ty::Vector => "vector",
        Ty::Struct(_) => "struct",
        Ty::Action => "action",
    }
}

fn numbered_suffix(name: &str, prefix: &str) -> bool {
    name.strip_prefix(prefix)
        .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
}

fn is_generic_name(name: &str, ty: &Ty) -> bool {
    let word = type_word(ty);
    numbered_suffix(name, word)
        || numbered_suffix(name, &format!("{word}GLOB_"))
        || numbered_suffix(name, &format!("{word}Param"))
}

#[derive(Clone, Debug, Default)]
pub struct NameGen {
    local: HashMap<&'static str, u32>,
    global: HashMap<&'static str, u32>,
    used: HashSet<String>,
}

impl NameGen {
    fn next(map: &mut HashMap<&'static str, u32>, word: &'static str) -> u32 {
        let n = map.entry(word).or_insert(0);
        *n += 1;
        *n
    }

    fn take(&mut self, name: String) -> String {
        self.used.insert(name.clone());
        name
    }

    fn collide(&mut self, base: &str) -> String {
        if self.used.insert(base.to_string()) {
            return base.to_string();
        }
        let mut n = 2u32;
        loop {
            let candidate = format!("{base}{n}");
            if self.used.insert(candidate.clone()) {
                return candidate;
            }
            n += 1;
        }
    }

    pub fn generic(&mut self, ty: &Ty) -> String {
        let word = type_word(ty);
        let n = Self::next(&mut self.local, word);
        self.take(format!("{word}{n}"))
    }

    pub fn global(&mut self, ty: &Ty) -> String {
        let word = type_word(ty);
        let n = Self::next(&mut self.global, word);
        self.take(format!("{word}GLOB_{n}"))
    }

    /// `ordinal` is 1-based across types (`intParam1`, `objectParam2`).
    pub fn param(&mut self, ty: &Ty, ordinal: usize) -> String {
        let word = type_word(ty);
        self.take(format!("{word}Param{ordinal}"))
    }

    pub fn apply_action_hint(&mut self, var: &mut Var, action: &str, args: &[Expr]) {
        let generic = match var.name.as_deref() {
            None => true,
            Some(name) => is_generic_name(name, &var.ty),
        };
        if !generic {
            return;
        }
        let Some(hint) = name_from_action(action, args) else {
            return;
        };
        var.name = Some(self.collide(&hint));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stack::{Const, VarKind};

    fn object_var(name: Option<String>) -> Var {
        Var {
            ty: Ty::Object,
            name,
            kind: VarKind::Local,
            assigned: false,
            on_stack: 0,
            parent_struct: None,
        }
    }

    #[test]
    fn name_from_action_table_spot_checks() {
        assert_eq!(name_from_action("GetFirstPC", &[]).as_deref(), Some("oPC"));
        assert_eq!(name_from_action("Random", &[]).as_deref(), Some("nRandom"));
        let tag = Expr::Const(Const::Str("kor33b_victim1".into()));
        assert_eq!(
            name_from_action("GetObjectByTag", &[tag]).as_deref(),
            Some("oKor33b_victim1")
        );
        let ab = Expr::Const(Const::Str("a b".into()));
        assert_eq!(
            name_from_action("GetObjectByTag", &[ab]).as_deref(),
            Some("oA_b")
        );
        assert_eq!(name_from_action("Location", &[]), None);
        assert_eq!(name_from_action("d12", &[]), None); // not in table
        assert_eq!(name_from_action("d6", &[]).as_deref(), Some("nRandom"));
    }

    #[test]
    fn collision_suffix() {
        let mut ng = NameGen::default();
        let mut a = object_var(Some(ng.generic(&Ty::Object)));
        let mut b = object_var(Some(ng.generic(&Ty::Object)));
        ng.apply_action_hint(&mut a, "GetFirstPC", &[]);
        ng.apply_action_hint(&mut b, "GetFirstPC", &[]);
        assert_eq!(a.name.as_deref(), Some("oPC"));
        assert_eq!(b.name.as_deref(), Some("oPC2"));
    }

    #[test]
    fn name_generic_global_param_prefixes() {
        let mut ng = NameGen::default();
        assert_eq!(ng.generic(&Ty::Int), "int1");
        assert_eq!(ng.generic(&Ty::Int), "int2");
        assert_eq!(ng.generic(&Ty::Object), "object1");
        assert_eq!(ng.generic(&Ty::Str), "string1");
        assert_eq!(ng.generic(&Ty::Vector), "vector1");
        assert_eq!(ng.generic(&Ty::Struct(crate::ty::StructId(0))), "struct1");

        let mut ng = NameGen::default();
        assert_eq!(ng.global(&Ty::Int), "intGLOB_1");
        assert_eq!(ng.global(&Ty::Str), "stringGLOB_1");
        assert_eq!(ng.global(&Ty::Int), "intGLOB_2");

        let mut ng = NameGen::default();
        assert_eq!(ng.param(&Ty::Int, 1), "intParam1");
        assert_eq!(ng.param(&Ty::Object, 2), "objectParam2");
        assert_eq!(ng.param(&Ty::Int, 3), "intParam3");
    }
}
