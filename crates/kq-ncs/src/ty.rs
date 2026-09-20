/// NWScript value type. Struct internment lands in a later task.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ty {
    /// Placeholder for unresolved stack slots (filled in by later typing).
    #[allow(dead_code)]
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
    /// Parameter-only; ACTION callbacks occupy no stack slots.
    Action,
}
