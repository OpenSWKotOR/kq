//! Local stack: 1-slot entries, position 1 = top (`len - pos`).

use crate::ty::{StructTable, Ty};

pub fn stack_offset_to_pos(off: i32) -> usize {
    (-off / 4) as usize
}

pub fn stack_size_to_pos(size: i32) -> usize {
    (size / 4) as usize
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VarId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VarKind {
    Local,
    Param,
    Return,
    Temp,
    Global,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Const {
    Int(i32),
    Float(f32),
    Str(String),
    Object(i32),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    Var(VarId),
    Const(Const),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Var {
    pub ty: Ty,
    pub name: Option<String>,
    pub kind: VarKind,
    pub assigned: bool,
    pub on_stack: u32,
    pub parent_struct: Option<VarId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StackError {
    Underflow,
    OutOfRange { pos: usize },
    InvalidCopy { off: i32, size: i32 },
    InvalidDestruct { rem: i32, off: i32, save: i32 },
    InvalidStructify { first: usize, count: usize },
    NotVar { pos: usize },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CpDownTarget {
    Assign { pos: usize, count: usize },
    Return { loc: usize },
}

#[derive(Clone, Debug, Default)]
pub struct LocalStack {
    entries: Vec<Entry>,
    vars: Vec<Var>,
}

impl LocalStack {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, pos: usize) -> Option<&Entry> {
        if pos == 0 || pos > self.entries.len() {
            return None;
        }
        self.entries.get(self.entries.len() - pos)
    }

    pub fn var(&self, id: VarId) -> &Var {
        &self.vars[id.0 as usize]
    }

    pub fn alloc_var(&mut self, var: Var) -> VarId {
        let id = VarId(self.vars.len() as u32);
        self.vars.push(var);
        id
    }

    pub fn push_var(&mut self, var: Var) -> VarId {
        let id = self.alloc_var(var);
        self.push(Entry::Var(id));
        id
    }

    pub fn push(&mut self, e: Entry) {
        if let Entry::Var(id) = e {
            self.vars[id.0 as usize].on_stack += 1;
        }
        self.entries.push(e);
    }

    pub fn pop_n(&mut self, n: usize) -> Result<Vec<Entry>, StackError> {
        if n > self.entries.len() {
            return Err(StackError::Underflow);
        }
        let start = self.entries.len() - n;
        let popped: Vec<Entry> = self.entries.drain(start..).rev().collect();
        for e in &popped {
            if let Entry::Var(id) = e {
                let v = &mut self.vars[id.0 as usize];
                v.on_stack = v.on_stack.saturating_sub(1);
            }
        }
        Ok(popped)
    }

    pub fn cptopsp(&mut self, off: i32, size: i32) -> Result<(), StackError> {
        let loc = stack_offset_to_pos(off);
        let copy = stack_size_to_pos(size);
        if copy == 0 {
            return Ok(());
        }
        if loc == 0 || loc > self.entries.len() || copy > loc {
            return Err(StackError::InvalidCopy { off, size });
        }
        let first = loc - copy + 1;
        let mut to_push = Vec::with_capacity(copy);
        for pos in (first..=loc).rev() {
            let e = self
                .get(pos)
                .cloned()
                .ok_or(StackError::OutOfRange { pos })?;
            to_push.push(e);
        }
        for e in to_push {
            self.push(e);
        }
        Ok(())
    }

    pub fn cpdownsp(&mut self, off: i32, size: i32) -> Result<CpDownTarget, StackError> {
        let loc = stack_offset_to_pos(off);
        let copy = stack_size_to_pos(size);
        if loc >= self.entries.len() {
            return Ok(CpDownTarget::Return { loc });
        }
        if loc == 0 || copy == 0 || copy > loc {
            return Err(StackError::InvalidCopy { off, size });
        }
        for i in 0..copy {
            let dest_pos = loc - i;
            if let Some(Entry::Var(id)) = self.get(dest_pos).cloned() {
                self.vars[id.0 as usize].assigned = true;
            }
        }
        Ok(CpDownTarget::Assign {
            pos: loc,
            count: copy,
        })
    }

    pub fn destruct(&mut self, rem: i32, off: i32, save: i32) -> Result<(), StackError> {
        let rem_slots = stack_size_to_pos(rem);
        let off_slots = stack_size_to_pos(off);
        let save_slots = stack_size_to_pos(save);
        if rem_slots > self.entries.len() || off_slots + save_slots > rem_slots {
            return Err(StackError::InvalidDestruct { rem, off, save });
        }
        let mut kept = Vec::with_capacity(save_slots);
        for i in 0..save_slots {
            let pos = 1 + off_slots + i;
            let e = self
                .get(pos)
                .cloned()
                .ok_or(StackError::OutOfRange { pos })?;
            kept.push(e);
        }
        self.pop_n(rem_slots)?;
        for e in kept.into_iter().rev() {
            self.push(e);
        }
        Ok(())
    }

    pub fn structify(
        &mut self,
        first: usize,
        count: usize,
        structs: &mut StructTable,
    ) -> Result<VarId, StackError> {
        if count == 0 {
            return Err(StackError::InvalidStructify { first, count });
        }
        let mut field_ids = Vec::with_capacity(count);
        let mut types = Vec::with_capacity(count);
        for i in (0..count).rev() {
            let pos = first + i;
            match self.get(pos) {
                Some(Entry::Var(id)) => {
                    field_ids.push(*id);
                    types.push(self.vars[id.0 as usize].ty);
                }
                Some(Entry::Const(_)) => return Err(StackError::NotVar { pos }),
                None => return Err(StackError::OutOfRange { pos }),
            }
        }
        let ty = if types.len() == 3 && types.iter().all(|t| *t == Ty::Float) {
            Ty::Vector
        } else {
            Ty::Struct(structs.intern(types))
        };
        let parent = self.alloc_var(Var {
            ty,
            name: None,
            kind: VarKind::Local,
            assigned: false,
            on_stack: 0,
            parent_struct: None,
        });
        for id in field_ids {
            self.vars[id.0 as usize].parent_struct = Some(parent);
        }
        Ok(parent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ty::{StructTable, Ty};

    fn float_var() -> Var {
        Var {
            ty: Ty::Float,
            name: None,
            kind: VarKind::Local,
            assigned: false,
            on_stack: 0,
            parent_struct: None,
        }
    }

    fn int_var() -> Var {
        Var {
            ty: Ty::Int,
            name: None,
            kind: VarKind::Local,
            assigned: false,
            on_stack: 0,
            parent_struct: None,
        }
    }

    #[test]
    fn cptopsp_dup_and_vector_copy() {
        assert_eq!(stack_offset_to_pos(-4), 1);
        assert_eq!(stack_offset_to_pos(-12), 3);
        assert_eq!(stack_size_to_pos(12), 3);

        let mut s = LocalStack::new();
        let _a = s.push_var(float_var());
        let _b = s.push_var(float_var());
        let c = s.push_var(float_var());
        s.cptopsp(-4, 4).unwrap();
        assert_eq!(s.len(), 4);
        assert_eq!(s.get(1), Some(&Entry::Var(c)));
        assert_eq!(s.get(2), Some(&Entry::Var(c)));

        let mut s = LocalStack::new();
        let a = s.push_var(float_var());
        let b = s.push_var(float_var());
        let c = s.push_var(float_var());
        s.cptopsp(-12, 12).unwrap();
        assert_eq!(s.len(), 6);
        assert_eq!(s.get(1), Some(&Entry::Var(c)));
        assert_eq!(s.get(2), Some(&Entry::Var(b)));
        assert_eq!(s.get(3), Some(&Entry::Var(a)));
        assert_eq!(s.get(4), Some(&Entry::Var(c)));
        assert_eq!(s.get(5), Some(&Entry::Var(b)));
        assert_eq!(s.get(6), Some(&Entry::Var(a)));
    }

    #[test]
    fn destruct_keeps_field() {
        let mut s = LocalStack::new();
        let x = s.push_var(float_var());
        let _y = s.push_var(float_var());
        let _z = s.push_var(float_var());

        s.destruct(12, 8, 4).unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s.get(1), Some(&Entry::Var(x)));

        let mut s = LocalStack::new();
        let _x = s.push_var(float_var());
        let _y = s.push_var(float_var());
        let z = s.push_var(float_var());
        s.destruct(12, 0, 4).unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s.get(1), Some(&Entry::Var(z)));

        let mut s = LocalStack::new();
        let _x = s.push_var(float_var());
        let y = s.push_var(float_var());
        let _z = s.push_var(float_var());
        s.destruct(12, 4, 4).unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s.get(1), Some(&Entry::Var(y)));
    }

    #[test]
    fn structify_vectors_and_structs() {
        let mut structs = StructTable::new();
        let mut s = LocalStack::new();
        let x = s.push_var(float_var());
        let y = s.push_var(float_var());
        let z = s.push_var(float_var());
        let vid = s.structify(1, 3, &mut structs).unwrap();
        assert_eq!(s.var(vid).ty, Ty::Vector);
        assert_eq!(s.var(x).parent_struct, Some(vid));
        assert_eq!(s.var(y).parent_struct, Some(vid));
        assert_eq!(s.var(z).parent_struct, Some(vid));
        assert!(structs.decls().next().is_none());

        let mut s = LocalStack::new();
        let a = s.push_var(int_var());
        let b = s.push_var(int_var());
        let sid = s.structify(1, 2, &mut structs).unwrap();
        match s.var(sid).ty {
            Ty::Struct(id) => {
                assert_eq!(structs.decl_name(id), "struct structtype1");
                assert_eq!(
                    structs.field_names(id),
                    &["int1".to_string(), "int2".to_string()]
                );
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(s.var(a).parent_struct, Some(sid));
        assert_eq!(s.var(b).parent_struct, Some(sid));
    }

    #[test]
    fn cpdownsp_assigns_or_return() {
        let mut s = LocalStack::new();
        s.push(Entry::Const(Const::Int(0))); // filler below dest (pos 3 once stack is full)
        let dest = s.push_var(int_var());
        s.push(Entry::Const(Const::Int(7))); // top; dest stays at pos 2, len 3 > loc 2
        match s.cpdownsp(-8, 4).unwrap() {
            CpDownTarget::Assign { pos, count } => {
                assert_eq!(pos, 2);
                assert_eq!(count, 1);
            }
            other => panic!("{other:?}"),
        }
        assert!(s.var(dest).assigned);

        let mut s = LocalStack::new();
        s.push(Entry::Const(Const::Int(1)));
        match s.cpdownsp(-12, 4).unwrap() {
            CpDownTarget::Return { loc } => assert_eq!(loc, 3),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn cpdownsp_loc_eq_len_is_return() {
        let n = 2usize;
        let mut s = LocalStack::new();
        s.push(Entry::Const(Const::Int(1)));
        s.push(Entry::Const(Const::Int(2)));
        assert_eq!(s.len(), n);
        match s.cpdownsp(-(n as i32 * 4) as i32, 4).unwrap() {
            CpDownTarget::Return { loc } => assert_eq!(loc, n),
            other => panic!("{other:?}"),
        }
    }
}
