//! NWScript value types and interned struct layouts (research-dencs §3.1–3.2).

use std::collections::HashMap;

/// NWScript value type. `Action` is parameter-only (no stack slots).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ty {
    /// Placeholder for unresolved stack slots (filled in by later typing).
    Unknown,
    Void,
    Int,
    Float,
    Str,
    Object,
    Effect,
    Event,
    Location,
    Talent,
    Vector,
    Struct(StructId),
    /// Parameter-only; ACTION callbacks occupy no stack slots.
    Action,
}

impl Ty {
    pub fn from_qualifier(q: u8) -> Ty {
        match q {
            0 => Ty::Void,
            3 => Ty::Int,
            4 => Ty::Float,
            5 => Ty::Str,
            6 => Ty::Object,
            16 => Ty::Effect,
            17 => Ty::Event,
            18 => Ty::Location,
            19 => Ty::Talent,
            _ => Ty::Unknown,
        }
    }

    pub fn slots(self, structs: &StructTable) -> usize {
        match self {
            Ty::Void | Ty::Action => 0,
            Ty::Vector => 3,
            Ty::Struct(id) => structs.get(id).map(|d| d.slots()).unwrap_or(1),
            _ => 1,
        }
    }

    pub fn decl_name(self, structs: &StructTable) -> String {
        match self {
            Ty::Struct(id) => structs.decl_name(id),
            other => other.word().to_string(),
        }
    }

    fn word(self) -> &'static str {
        match self {
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
}

/// 0-based intern key; printed names are 1-based `structtypeN`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StructId(pub u32);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StructDef {
    pub name: String,
    pub elements: Vec<Ty>,
    pub field_names: Vec<String>,
}

impl StructDef {
    pub fn is_vector(&self) -> bool {
        self.elements.len() == 3 && self.elements.iter().all(|t| *t == Ty::Float)
    }

    pub fn slots(&self) -> usize {
        if self.is_vector() {
            3
        } else {
            self.elements.len()
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct StructTable {
    defs: Vec<StructDef>,
}

impl StructTable {
    pub fn new() -> Self {
        Self { defs: Vec::new() }
    }

    pub fn intern(&mut self, elements: Vec<Ty>) -> StructId {
        if let Some((i, _)) = self
            .defs
            .iter()
            .enumerate()
            .find(|(_, d)| d.elements == elements)
        {
            return StructId(i as u32);
        }
        let name = format!("structtype{}", self.defs.len() + 1);
        let field_names = field_names(&elements);
        self.defs.push(StructDef {
            name,
            elements,
            field_names,
        });
        StructId((self.defs.len() - 1) as u32)
    }

    pub fn get(&self, id: StructId) -> Option<&StructDef> {
        self.defs.get(id.0 as usize)
    }

    pub fn decl_name(&self, id: StructId) -> String {
        match self.get(id) {
            Some(def) if def.is_vector() => "vector".into(),
            Some(def) => format!("struct {}", def.name),
            None => "struct".into(),
        }
    }

    pub fn field_names(&self, id: StructId) -> &[String] {
        self.get(id)
            .map(|d| d.field_names.as_slice())
            .unwrap_or(&[])
    }

    pub fn decls(&self) -> impl Iterator<Item = &StructDef> {
        self.defs.iter().filter(|d| !d.is_vector())
    }

    /// Merge interned layouts from another table (same element list → same id).
    pub fn absorb(&mut self, other: Self) {
        for def in other.defs {
            self.intern(def.elements);
        }
    }
}

fn field_names(elements: &[Ty]) -> Vec<String> {
    if elements.len() == 3 && elements.iter().all(|t| *t == Ty::Float) {
        return vec!["x".into(), "y".into(), "z".into()];
    }
    let mut counts: HashMap<&'static str, usize> = HashMap::new();
    elements
        .iter()
        .map(|ty| {
            let word = ty.word();
            let n = counts.entry(word).and_modify(|c| *c += 1).or_insert(1);
            format!("{word}{n}")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_qualifier_maps_ncs_bytes() {
        assert_eq!(Ty::from_qualifier(3), Ty::Int);
        assert_eq!(Ty::from_qualifier(4), Ty::Float);
        assert_eq!(Ty::from_qualifier(5), Ty::Str);
        assert_eq!(Ty::from_qualifier(6), Ty::Object);
        assert_eq!(Ty::from_qualifier(16), Ty::Effect);
        assert_eq!(Ty::from_qualifier(17), Ty::Event);
        assert_eq!(Ty::from_qualifier(18), Ty::Location);
        assert_eq!(Ty::from_qualifier(19), Ty::Talent);
        assert_eq!(Ty::from_qualifier(0), Ty::Void);
        assert_eq!(Ty::from_qualifier(1), Ty::Unknown);
    }

    #[test]
    fn slots_and_decl_names() {
        let mut structs = StructTable::new();
        assert_eq!(Ty::Void.slots(&structs), 0);
        assert_eq!(Ty::Action.slots(&structs), 0);
        assert_eq!(Ty::Vector.slots(&structs), 3);
        assert_eq!(Ty::Int.slots(&structs), 1);
        assert_eq!(Ty::Int.decl_name(&structs), "int");
        assert_eq!(Ty::Vector.decl_name(&structs), "vector");
        assert_eq!(Ty::Event.decl_name(&structs), "event");

        let vec_id = structs.intern(vec![Ty::Float, Ty::Float, Ty::Float]);
        let pair = structs.intern(vec![Ty::Int, Ty::Object]);
        let pair2 = structs.intern(vec![Ty::Int, Ty::Object]);
        assert_eq!(pair, pair2);
        assert_eq!(Ty::Struct(vec_id).slots(&structs), 3);
        assert_eq!(Ty::Struct(vec_id).decl_name(&structs), "vector");
        assert_eq!(Ty::Struct(pair).decl_name(&structs), "struct structtype2");
        let decls: Vec<_> = structs.decls().collect();
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].name, "structtype2");
        assert_eq!(decls[0].field_names, ["int1", "object1"]);
    }
}
